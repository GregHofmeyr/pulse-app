//! Steam linking and live achievements (#379), against the fake Steam (`testing::fake_steam`): nothing here reaches
//! the real network. Linking through OpenID (and every way a return can be refused), the backfill and completion rate,
//! live unlock cards and the 100 % card in a server's Achievements channel, Completionist, the toggles, privacy
//! backoff, unlinking, the tail, the daily allowance and the Achievements channel's rules.
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, ServerFrame};
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{
    AchievementCard, AchievementsChannel, Message, PlayingUpdate, Profile, SteamLinkStart,
    SteamStatus,
};
use pulse_server::config::Config;
use pulse_server::testing::fake_steam::FakeSteam;
use reqwest::Url;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message as Ws;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

const ALEX_STEAM: &str = "76500000000000001";
const SAM_STEAM: &str = "76500000000000002";
const SPACE: u32 = 4_000_001;
const RACER: u32 = 4_000_002;
/// Long before anyone linked: a backfilled unlock.
const LONG_AGO: i64 = 1_000_000_000;

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

async fn setup_with(tweak: impl FnOnce(&mut Config)) -> (TestApp, FakeSteam) {
    let fake = FakeSteam::start().await;
    let mut cfg = test_config();
    fake.configure(&mut cfg);
    tweak(&mut cfg);
    let app = spawn_with(cfg).await;
    fake.add_game(
        SPACE,
        "Space Game",
        &[
            ("FIRST", "First Steps", "Walk a bit."),
            ("SECOND", "Second Wind", "Run a bit."),
            ("THIRD", "Third Time", "Fly."),
        ],
        40.0,
    );
    fake.add_game(
        RACER,
        "Racer",
        &[("LAP", "One Lap", "Drive."), ("WIN", "Winner", "Win.")],
        10.0,
    );
    fake.player(ALEX_STEAM, "alex on steam");
    fake.player(SAM_STEAM, "sam on steam");
    (app, fake)
}

async fn setup() -> (TestApp, FakeSteam) {
    setup_with(|_| {}).await
}

async fn status(app: &TestApp, token: &str) -> SteamStatus {
    get_json(app, token, "/me/steam").await
}

/// Start linking as `token` and sign in to the fake as `steam_id`; the return URL Steam sends the browser to, pointed
/// at the test server (the configured public URL isn't served).
async fn return_url(app: &TestApp, fake: &FakeSteam, token: &str, steam_id: &str) -> String {
    let r = post_json(app, token, "/me/steam/link", json!({})).await;
    assert_eq!(r.status(), 200);
    let start: SteamLinkStart = r.json().await.unwrap();
    assert!(
        start.url.starts_with(&fake.openid_endpoint()),
        "{}",
        start.url
    );
    fake.sign_in_as(steam_id);
    let browser = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let r = browser.get(&start.url).send().await.unwrap();
    assert_eq!(r.status(), 302);
    let loc = Url::parse(r.headers()["location"].to_str().unwrap()).unwrap();
    assert_eq!(loc.host_str(), Some("pulse.example.test"));
    app.url(&format!("{}?{}", loc.path(), loc.query().unwrap()))
}

async fn open(app: &TestApp, url: &str) -> (u16, String) {
    let r = app.http.get(url).send().await.unwrap();
    let status = r.status().as_u16();
    let csp = r.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_string();
    assert!(csp.starts_with("default-src 'none'"), "{csp}");
    (status, r.text().await.unwrap())
}

async fn link(app: &TestApp, fake: &FakeSteam, token: &str, steam_id: &str) -> String {
    let url = return_url(app, fake, token, steam_id).await;
    open(app, &url).await.1
}

async fn wait_backfill(app: &TestApp, token: &str) -> SteamStatus {
    for _ in 0..200 {
        let st = status(app, token).await;
        if st.link.as_ref().is_some_and(|l| l.backfill_done) {
            return st;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the backfill never finished");
}

async fn hello(app: &TestApp, token: &str) -> Socket {
    let mut ws = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap()
        .0;
    let f = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
        client_version: pulse_protocol::PROTOCOL_VERSION,
    })
    .unwrap();
    ws.send(Ws::text(f)).await.unwrap();
    loop {
        if let Some(ServerFrame::Ready(_)) = next_frame(&mut ws).await {
            return ws;
        }
    }
}

async fn next_frame(ws: &mut Socket) -> Option<ServerFrame> {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .ok()??
            .ok()?;
        match m {
            Ws::Text(t) => return Some(serde_json::from_str(&t).unwrap()),
            Ws::Close(_) => return None,
            _ => continue,
        }
    }
}

async fn drain(ws: &mut Socket, ms: u64) -> Vec<Event> {
    let mut got = vec![];
    while let Ok(Some(f)) = tokio::time::timeout(Duration::from_millis(ms), next_frame(ws)).await {
        if let ServerFrame::Event(e) = f {
            got.push(e);
        }
    }
    got
}

/// Send a Playing frame and wait until the server holds it.
async fn play(app: &TestApp, ws: &mut Socket, me: UserId, update: Option<PlayingUpdate>) {
    let want = update.is_some();
    let f = serde_json::to_string(&ClientFrame::Playing { playing: update }).unwrap();
    ws.send(Ws::text(f)).await.unwrap();
    for _ in 0..200 {
        if app.hub.playing_of(me).is_some() == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the server never held the game");
}

fn steam_game(name: &str, app_id: u32, hide_art: bool) -> PlayingUpdate {
    PlayingUpdate {
        name: name.into(),
        steam_app_id: Some(app_id),
        elapsed_secs: Some(60),
        hide_art,
    }
}

async fn tick(app: &TestApp) {
    pulse_server::steam::poller::tick(&app.state()).await;
}

async fn cards(app: &TestApp, token: &str, ch: ChannelId) -> Vec<(Message, AchievementCard)> {
    let msgs: Vec<Message> =
        get_json(app, token, &format!("/channels/{ch}/messages?limit=100")).await;
    let mut out: Vec<_> = msgs
        .into_iter()
        .filter_map(|m| {
            let c = m.achievement.clone()?;
            Some((m, *c))
        })
        .collect();
    out.sort_by_key(|a| a.0.id);
    out
}

/// alex (linked to ALEX_STEAM, backfilled) owns a server whose #general is its Achievements channel.
async fn linked_alex(app: &TestApp, fake: &FakeSteam) -> (UserId, String, ChannelId) {
    let (alex, token) = register(app, "alex").await;
    let server = create_server(app, &token, "Gamers").await;
    let general = general(app, &token, server.id).await;
    let r = app
        .http
        .put(app.url(&format!("/servers/{}/achievements-channel", server.id)))
        .bearer_auth(&token)
        .json(&json!({ "channel_id": general.id }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let got: AchievementsChannel = r.json().await.unwrap();
    assert_eq!(got.channel_id, Some(general.id));
    fake.own(ALEX_STEAM, SPACE, 600);
    fake.unlock(ALEX_STEAM, SPACE, "FIRST", LONG_AGO);
    let page = link(app, fake, &token, ALEX_STEAM).await;
    assert!(page.contains("Steam linked"), "{page}");
    wait_backfill(app, &token).await;
    (alex, token, general.id)
}

#[tokio::test]
async fn linking_through_steam_openid_shows_the_account_and_backfills() {
    let (app, fake) = setup().await;
    let (alex, token) = register(&app, "alex").await;
    let before = status(&app, &token).await;
    assert!(before.linking_available && before.achievements_available);
    assert_eq!(before.link, None);

    let mut ws = hello(&app, &token).await;
    fake.own(ALEX_STEAM, SPACE, 600);
    fake.own(ALEX_STEAM, RACER, 30);
    fake.unlock(ALEX_STEAM, SPACE, "FIRST", LONG_AGO);
    fake.unlock(ALEX_STEAM, RACER, "LAP", LONG_AGO);
    fake.unlock(ALEX_STEAM, RACER, "WIN", LONG_AGO);
    let (code, page) = open(&app, &return_url(&app, &fake, &token, ALEX_STEAM).await).await;
    assert_eq!(code, 200);
    assert!(
        page.contains("Steam linked") && page.contains("close this tab"),
        "{page}"
    );

    let st = wait_backfill(&app, &token).await;
    let l = st.link.unwrap();
    assert_eq!(l.steam_id, ALEX_STEAM);
    assert_eq!(l.persona.as_deref(), Some("alex on steam"));
    assert!(l.share_achievements && !l.private);
    // Space Game 1/3 and Racer 2/2: (33.3 + 100) / 2
    assert_eq!(l.completion_rate, Some(66.7));
    assert_eq!(l.perfect_games, 1);
    // a game completed before linking still earns Completionist (quietly: no card anywhere)
    let tier: i64 = pulse_server::testing::count(
        &app,
        &format!("SELECT COALESCE(MAX(tier), 0) FROM user_badges WHERE user_id = '{alex}' AND badge_key = 'completionist'"),
    )
    .await;
    assert_eq!(tier, 1);
    assert_eq!(fake.calls("GetOwnedGames"), 1);
    assert_eq!(fake.calls("GetPlayerAchievements"), 2);

    // Settings heard about it without asking
    let told = drain(&mut ws, 300).await;
    assert!(told.iter().any(|e| matches!(e,
        Event::SteamLinkChanged { user_id, status } if *user_id == alex && status.link.is_some())));

    // the profile card shows the completion rate
    let p: Profile = get_json(&app, &token, &format!("/users/{alex}/profile")).await;
    assert_eq!(p.stats.steam_completion, Some(66.7));
    assert_eq!(p.stats.steam_perfect_games, Some(1));
}

#[tokio::test]
async fn a_return_that_doesnt_check_out_links_nothing() {
    let (app, fake) = setup().await;
    let (_, token) = register(&app, "alex").await;

    // used twice: the second is refused
    let url = return_url(&app, &fake, &token, ALEX_STEAM).await;
    assert!(open(&app, &url).await.1.contains("Steam linked"));
    let (_, again) = open(&app, &url).await;
    assert!(again.contains("expired or was already used"), "{again}");
    app.http
        .delete(app.url("/me/steam"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();

    // a signature Steam never issued: Steam doesn't vouch for it
    let url = return_url(&app, &fake, &token, ALEX_STEAM).await;
    let forged = url.replace("openid.sig=", "openid.sig=00");
    let (_, page) = open(&app, &forged).await;
    assert!(page.contains("didn't confirm"), "{page}");
    assert_eq!(status(&app, &token).await.link, None);

    // someone else's claimed id swapped in (identity left alone)
    let url = return_url(&app, &fake, &token, ALEX_STEAM).await;
    let swapped = url.replacen(
        &format!("openid.claimed_id=https%3A%2F%2Fsteamcommunity.com%2Fopenid%2Fid%2F{ALEX_STEAM}"),
        &format!("openid.claimed_id=https%3A%2F%2Fsteamcommunity.com%2Fopenid%2Fid%2F{SAM_STEAM}"),
        1,
    );
    assert_ne!(swapped, url);
    let (_, page) = open(&app, &swapped).await;
    assert!(page.contains("didn't check out"), "{page}");

    // no state, or a made-up one
    let (_, page) = open(&app, &app.url("/steam/openid/return")).await;
    assert!(page.contains("missing its sign-in"), "{page}");
    let (_, page) = open(&app, &app.url("/steam/openid/return?state=abc")).await;
    assert!(page.contains("expired"), "{page}");
    assert_eq!(status(&app, &token).await.link, None);
}

#[tokio::test]
async fn one_steam_account_per_person() {
    let (app, fake) = setup().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    assert!(
        link(&app, &fake, &alex, ALEX_STEAM)
            .await
            .contains("Steam linked")
    );
    let page = link(&app, &fake, &sam, ALEX_STEAM).await;
    assert!(page.contains("already linked to someone else"), "{page}");
    assert_eq!(status(&app, &sam).await.link, None);
    // the same person again changes nothing
    assert!(
        link(&app, &fake, &alex, ALEX_STEAM)
            .await
            .contains("Steam linked")
    );
    assert_eq!(status(&app, &alex).await.link.unwrap().steam_id, ALEX_STEAM);
}

#[tokio::test]
async fn live_unlocks_post_cards_and_a_100_percent_card() {
    let (app, fake) = setup().await;
    let (alex, token, general) = linked_alex(&app, &fake).await;
    let mut ws = hello(&app, &token).await;
    play(
        &app,
        &mut ws,
        alex,
        Some(steam_game("Space Game", SPACE, false)),
    )
    .await;

    // nothing new: no card, and the backfilled unlock is never announced
    tick(&app).await;
    assert!(cards(&app, &token, general).await.is_empty());

    fake.unlock(ALEX_STEAM, SPACE, "SECOND", now());
    tick(&app).await;
    let got = cards(&app, &token, general).await;
    assert_eq!(got.len(), 1);
    let (msg, card) = &got[0];
    assert_eq!(msg.author_id, None);
    assert_eq!(card.user_id, alex);
    assert_eq!((card.app_id, card.game.as_str()), (SPACE, "Space Game"));
    assert_eq!((card.unlocked, card.total, card.complete), (2, 3, false));
    let a = &card.achievements[0];
    assert_eq!(a.name, "Second Wind");
    assert_eq!(a.description.as_deref(), Some("Run a bit."));
    assert_eq!(a.global_percent, Some(20.0));
    let icon = a.icon.clone().expect("an icon, fetched by the server");
    assert!(
        msg.content.contains("alex unlocked Second Wind"),
        "{}",
        msg.content
    );

    // again: nothing new
    tick(&app).await;
    assert_eq!(cards(&app, &token, general).await.len(), 1);

    fake.unlock(ALEX_STEAM, SPACE, "THIRD", now());
    tick(&app).await;
    let got = cards(&app, &token, general).await;
    assert_eq!(got.len(), 3);
    assert_eq!(got[1].1.achievements[0].name, "Third Time");
    let done = &got[2].1;
    assert!(done.complete && done.achievements.is_empty());
    assert_eq!((done.unlocked, done.total), (3, 3));
    assert!(done.art.is_some(), "the game's header art");
    assert!(got[2].0.content.contains("completed Space Game"));
    assert!(
        !got.iter()
            .any(|(_, c)| c.achievements.iter().any(|a| a.name == "First Steps"))
    );

    let tier: i64 = pulse_server::testing::count(
        &app,
        &format!("SELECT COALESCE(MAX(tier), 0) FROM user_badges WHERE user_id = '{alex}' AND badge_key = 'completionist'"),
    )
    .await;
    assert_eq!(tier, 1, "Completionist");
    let p: Profile = get_json(&app, &token, &format!("/users/{alex}/profile")).await;
    assert_eq!(p.stats.steam_completion, Some(100.0));
    // the awards' record: two unlocks and the 100 %
    let recorded = pulse_server::testing::count(
        &app,
        &format!("SELECT COUNT(*) FROM steam_unlocks WHERE user_id = '{alex}'"),
    )
    .await;
    assert_eq!(recorded, 3);

    // the cards' pictures outlive the media sweep
    pulse_server::media::cleanup(&app.db, &app.cfg.media_dir, Duration::ZERO)
        .await
        .unwrap();
    let kept = pulse_server::testing::count(
        &app,
        &format!("SELECT COUNT(*) FROM media WHERE hash = '{}'", icon.hash),
    )
    .await;
    assert_eq!(kept, 1);
}

#[tokio::test]
async fn a_game_never_seen_announces_only_this_sessions_unlocks() {
    let (app, fake) = setup().await;
    let (alex, token, general) = linked_alex(&app, &fake).await;
    // Racer wasn't owned at backfill time; one old unlock, one just now
    fake.unlock(ALEX_STEAM, RACER, "LAP", LONG_AGO);
    fake.unlock(ALEX_STEAM, RACER, "WIN", now());
    let mut ws = hello(&app, &token).await;
    play(&app, &mut ws, alex, Some(steam_game("Racer", RACER, false))).await;
    tick(&app).await;
    let got = cards(&app, &token, general).await;
    assert_eq!(got.len(), 2, "the unlock and the 100 % it made");
    assert_eq!(got[0].1.achievements.len(), 1);
    assert_eq!(got[0].1.achievements[0].name, "Winner");
    assert!(got[1].1.complete);
}

#[tokio::test]
async fn achievements_work_with_game_art_hidden() {
    let (app, fake) = setup().await;
    let (alex, token, general) = linked_alex(&app, &fake).await;
    let mut ws = hello(&app, &token).await;
    play(
        &app,
        &mut ws,
        alex,
        Some(steam_game("Space Game", SPACE, true)),
    )
    .await;
    // nobody is shown the app id (so no art)
    let shown = app.hub.playing_of(alex).unwrap();
    assert_eq!((shown.steam_app_id, shown.art), (None, None));
    fake.unlock(ALEX_STEAM, SPACE, "SECOND", now());
    tick(&app).await;
    assert_eq!(cards(&app, &token, general).await.len(), 1);
}

#[tokio::test]
async fn sharing_off_means_no_polling_and_no_profile_stat() {
    let (app, fake) = setup().await;
    let (alex, token, general) = linked_alex(&app, &fake).await;
    let r = app
        .http
        .put(app.url("/me/steam/settings"))
        .bearer_auth(&token)
        .json(&json!({ "share_achievements": false }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let st: SteamStatus = r.json().await.unwrap();
    assert!(!st.link.unwrap().share_achievements);
    let mut ws = hello(&app, &token).await;
    play(
        &app,
        &mut ws,
        alex,
        Some(steam_game("Space Game", SPACE, false)),
    )
    .await;
    fake.unlock(ALEX_STEAM, SPACE, "SECOND", now());
    let calls = fake.calls("GetPlayerAchievements");
    tick(&app).await;
    assert_eq!(fake.calls("GetPlayerAchievements"), calls, "not polled");
    assert!(cards(&app, &token, general).await.is_empty());
    let p: Profile = get_json(&app, &token, &format!("/users/{alex}/profile")).await;
    assert_eq!(p.stats.steam_completion, None);
}

#[tokio::test]
async fn private_game_details_back_off_and_show_in_settings() {
    let (app, fake) = setup().await;
    let (alex, token, general) = linked_alex(&app, &fake).await;
    let mut ws = hello(&app, &token).await;
    play(
        &app,
        &mut ws,
        alex,
        Some(steam_game("Space Game", SPACE, false)),
    )
    .await;
    fake.set_private(ALEX_STEAM, true);
    tick(&app).await;
    assert!(status(&app, &token).await.link.unwrap().private);
    let calls = fake.calls("GetPlayerAchievements");
    tick(&app).await;
    assert_eq!(fake.calls("GetPlayerAchievements"), calls, "backed off");

    // public again: the next answer clears it (the backoff is an hour; ask directly)
    fake.set_private(ALEX_STEAM, false);
    fake.unlock(ALEX_STEAM, SPACE, "SECOND", now());
    pulse_server::steam::poller::poll_one(&app.state(), alex, ALEX_STEAM, SPACE, now(), None)
        .await
        .unwrap();
    assert!(!status(&app, &token).await.link.unwrap().private);
    assert_eq!(cards(&app, &token, general).await.len(), 1);
}

#[tokio::test]
async fn a_private_profile_pauses_the_backfill() {
    let (app, fake) = setup().await;
    let (_, token) = register(&app, "alex").await;
    fake.own(ALEX_STEAM, SPACE, 600);
    fake.set_private(ALEX_STEAM, true);
    link(&app, &fake, &token, ALEX_STEAM).await;
    let mut st = status(&app, &token).await;
    for _ in 0..200 {
        if st.link.as_ref().unwrap().private {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        st = status(&app, &token).await;
    }
    let l = st.link.unwrap();
    assert!(l.private && !l.backfill_done);
}

#[tokio::test]
async fn unlinking_forgets_the_account_and_its_state() {
    let (app, fake) = setup().await;
    let (alex, token, _) = linked_alex(&app, &fake).await;
    let r = app
        .http
        .delete(app.url("/me/steam"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let st: SteamStatus = r.json().await.unwrap();
    assert_eq!(st.link, None);
    let left = pulse_server::testing::count(
        &app,
        &format!("SELECT COUNT(*) FROM steam_achievement_state WHERE user_id = '{alex}'"),
    )
    .await;
    assert_eq!(left, 0);
    // relinking reads the games again
    let owned = fake.calls("GetOwnedGames");
    link(&app, &fake, &token, ALEX_STEAM).await;
    wait_backfill(&app, &token).await;
    assert_eq!(fake.calls("GetOwnedGames"), owned + 1);
    // and someone else may now link that account... after alex lets go of it
    let (_, sam) = register(&app, "sam").await;
    app.http
        .delete(app.url("/me/steam"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert!(
        link(&app, &fake, &sam, ALEX_STEAM)
            .await
            .contains("Steam linked")
    );
}

#[tokio::test]
async fn the_last_game_is_polled_for_the_tail_only() {
    let (app, fake) = setup_with(|c| c.steam.tail = Duration::ZERO).await;
    let (alex, token, _) = linked_alex(&app, &fake).await;
    let mut ws = hello(&app, &token).await;
    play(
        &app,
        &mut ws,
        alex,
        Some(steam_game("Space Game", SPACE, false)),
    )
    .await;
    tick(&app).await;
    let calls = fake.calls("GetPlayerAchievements");
    play(&app, &mut ws, alex, None).await;
    tick(&app).await;
    assert_eq!(fake.calls("GetPlayerAchievements"), calls, "no tail");

    // with the default tail, a game that just ended is still read
    let (app, fake) = setup().await;
    let (alex, token, _) = linked_alex(&app, &fake).await;
    let mut ws = hello(&app, &token).await;
    play(
        &app,
        &mut ws,
        alex,
        Some(steam_game("Space Game", SPACE, false)),
    )
    .await;
    tick(&app).await;
    let calls = fake.calls("GetPlayerAchievements");
    play(&app, &mut ws, alex, None).await;
    tick(&app).await;
    assert_eq!(fake.calls("GetPlayerAchievements"), calls + 1);
}

#[tokio::test]
async fn nobody_playing_is_never_polled() {
    let (app, fake) = setup().await;
    linked_alex(&app, &fake).await;
    let calls = fake.total_calls();
    tick(&app).await;
    assert_eq!(fake.total_calls(), calls);
}

#[tokio::test]
async fn the_daily_allowance_holds_whatever_happens() {
    let (app, fake) = setup_with(|c| c.steam.daily_cap = 3).await;
    let (alex, token) = register(&app, "alex").await;
    fake.own(ALEX_STEAM, SPACE, 600);
    fake.own(ALEX_STEAM, RACER, 600);
    link(&app, &fake, &token, ALEX_STEAM).await;
    let mut ws = hello(&app, &token).await;
    play(
        &app,
        &mut ws,
        alex,
        Some(steam_game("Space Game", SPACE, false)),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    for _ in 0..5 {
        tick(&app).await;
    }
    assert_eq!(fake.total_calls(), 3);
}

#[tokio::test]
async fn without_a_key_linking_works_and_nothing_is_polled() {
    let (app, fake) = setup_with(|c| c.steam.api_key = None).await;
    let (alex, token) = register(&app, "alex").await;
    let st = status(&app, &token).await;
    assert!(st.linking_available && !st.achievements_available);
    assert!(
        link(&app, &fake, &token, ALEX_STEAM)
            .await
            .contains("Steam linked")
    );
    let l = status(&app, &token).await.link.unwrap();
    assert_eq!(l.persona, None, "the persona needs the key");
    let mut ws = hello(&app, &token).await;
    play(
        &app,
        &mut ws,
        alex,
        Some(steam_game("Space Game", SPACE, false)),
    )
    .await;
    tick(&app).await;
    assert_eq!(fake.total_calls(), 0);
}

#[tokio::test]
async fn without_a_public_url_linking_is_off() {
    let fake = FakeSteam::start().await;
    let mut cfg = test_config();
    fake.configure(&mut cfg);
    cfg.embeds.public_url = None;
    let app = spawn_with(cfg).await;
    let (_, token) = register(&app, "alex").await;
    assert!(!status(&app, &token).await.linking_available);
    let r = post_json(&app, &token, "/me/steam/link", json!({})).await;
    assert_eq!(r.status(), 503);
}

#[tokio::test]
async fn only_the_owner_picks_one_of_the_servers_text_channels() {
    let (app, _fake) = setup().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let (_, kim) = register(&app, "kim").await;
    let server = create_server(&app, &alex, "Gamers").await;
    let other = create_server(&app, &kim, "Elsewhere").await;
    let general = general(&app, &alex, server.id).await;
    let r = post_json(
        &app,
        &sam,
        &format!("/servers/{}/join", server.id),
        json!({}),
    )
    .await;
    assert!(r.status().is_success());
    let path = format!("/servers/{}/achievements-channel", server.id);
    let put = |token: &str, body: serde_json::Value| {
        let req = app.http.put(app.url(&path)).bearer_auth(token).json(&body);
        async move { req.send().await.unwrap().status().as_u16() }
    };
    let got: AchievementsChannel = get_json(&app, &sam, &path).await;
    assert_eq!(got.channel_id, None, "off by default");
    assert_eq!(put(&sam, json!({ "channel_id": general.id })).await, 403);
    let voice = post_json(
        &app,
        &alex,
        &format!("/servers/{}/channels", server.id),
        json!({ "kind": "voice", "name": "Lounge" }),
    )
    .await;
    assert_eq!(voice.status(), 200);
    let voice: pulse_protocol::rest::Channel = voice.json().await.unwrap();
    assert_eq!(put(&alex, json!({ "channel_id": voice.id })).await, 400);
    let elsewhere = general_of(&app, &kim, other.id).await;
    assert_eq!(put(&alex, json!({ "channel_id": elsewhere })).await, 400);
    assert_eq!(put(&alex, json!({ "channel_id": general.id })).await, 200);
    let got: AchievementsChannel = get_json(&app, &sam, &path).await;
    assert_eq!(got.channel_id, Some(general.id));
    assert_eq!(put(&alex, json!({ "channel_id": null })).await, 200);
    let got: AchievementsChannel = get_json(&app, &alex, &path).await;
    assert_eq!(got.channel_id, None);
    // a stranger can't even look
    let r = app
        .http
        .get(app.url(&path))
        .bearer_auth(&kim)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
}

async fn general_of(
    app: &TestApp,
    token: &str,
    server: pulse_protocol::ids::ServerId,
) -> ChannelId {
    general(app, token, server).await.id
}

// ---------- Steam hours (#383) ----------

/// `name` registered and linked to `steam_id` (backfilled).
async fn linked(app: &TestApp, fake: &FakeSteam, name: &str, steam_id: &str) -> (UserId, String) {
    let (id, token) = register(app, name).await;
    let page = link(app, fake, &token, steam_id).await;
    assert!(page.contains("Steam linked"), "{page}");
    wait_backfill(app, &token).await;
    (id, token)
}

async fn hours_tick(app: &TestApp) {
    pulse_server::steam::hours::tick(&app.state()).await;
}

async fn set_steam(app: &TestApp, token: &str, body: serde_json::Value) -> SteamStatus {
    let r = app
        .http
        .put(app.url("/me/steam/settings"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

#[tokio::test]
async fn steam_hours_show_on_profiles_the_board_and_touch_grass() {
    let (app, fake) = setup().await;
    let (alex, alex_t) = linked(&app, &fake, "alex", ALEX_STEAM).await;
    let (sam, sam_t) = linked(&app, &fake, "sam", SAM_STEAM).await;
    let (_, kim_t) = register(&app, "kim").await;
    let server = create_server(&app, &alex_t, "Gamers").await;
    for t in [&sam_t, &kim_t] {
        let r = post_json(&app, t, &format!("/servers/{}/join", server.id), json!({})).await;
        assert!(r.status().is_success());
    }
    fake.played_recently(ALEX_STEAM, SPACE, 20 * 60);
    fake.played_recently(ALEX_STEAM, RACER, 90);
    fake.played_recently(SAM_STEAM, RACER, 45);
    hours_tick(&app).await;

    let p: Profile = get_json(&app, &kim_t, &format!("/users/{alex}/profile")).await;
    assert_eq!(p.stats.steam_recent_minutes, Some(20 * 60 + 90));
    assert_eq!(
        status(&app, &alex_t).await.link.unwrap().recent_minutes,
        Some(1290)
    );
    // most first; kim (not linked) isn't on it
    let board: pulse_protocol::rest::SteamHours =
        get_json(&app, &kim_t, &format!("/servers/{}/steam-hours", server.id)).await;
    let got: Vec<_> = board
        .entries
        .iter()
        .map(|e| (e.user_id, e.minutes))
        .collect();
    assert_eq!(got, [(alex, 1290), (sam, 45)]);
    // 21.5 h in two weeks: Touch Grass's first tier; sam's 45 minutes earn nothing
    let tier = |who: UserId| {
        let app = &app;
        async move {
            pulse_server::testing::count(
                app,
                &format!("SELECT COALESCE(MAX(tier), 0) FROM user_badges WHERE user_id = '{who}' AND badge_key = 'touch_grass'"),
            )
            .await
        }
    };
    assert_eq!(tier(alex).await, 1);
    assert_eq!(tier(sam).await, 0);
    // a quiet fortnight later: the card follows, the badge stays
    fake.clear_recent(ALEX_STEAM);
    hours_tick(&app).await;
    let p: Profile = get_json(&app, &kim_t, &format!("/users/{alex}/profile")).await;
    assert_eq!(p.stats.steam_recent_minutes, Some(0));
    assert_eq!(tier(alex).await, 1);
    let board: pulse_protocol::rest::SteamHours =
        get_json(&app, &kim_t, &format!("/servers/{}/steam-hours", server.id)).await;
    assert_eq!(board.entries.len(), 1, "nobody with no hours");
    // a stranger can't look
    let (_, jo_t) = register(&app, "jo").await;
    let r = app
        .http
        .get(app.url(&format!("/servers/{}/steam-hours", server.id)))
        .bearer_auth(&jo_t)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
}

#[tokio::test]
async fn steam_hours_off_hides_them_and_stops_reading() {
    let (app, fake) = setup().await;
    let (alex, token) = linked(&app, &fake, "alex", ALEX_STEAM).await;
    let server = create_server(&app, &token, "Gamers").await;
    fake.played_recently(ALEX_STEAM, SPACE, 600);
    hours_tick(&app).await;
    assert!(
        status(&app, &token).await.link.unwrap().share_hours,
        "on by default"
    );

    let st = set_steam(&app, &token, json!({ "share_hours": false })).await;
    let l = st.link.unwrap();
    assert!(!l.share_hours);
    assert_eq!(l.recent_minutes, None);
    assert!(l.share_achievements, "the other switch stays");
    let calls = fake.calls("GetRecentlyPlayedGames");
    hours_tick(&app).await;
    assert_eq!(fake.calls("GetRecentlyPlayedGames"), calls, "not read");
    let p: Profile = get_json(&app, &token, &format!("/users/{alex}/profile")).await;
    assert_eq!(p.stats.steam_recent_minutes, None);
    let board: pulse_protocol::rest::SteamHours =
        get_json(&app, &token, &format!("/servers/{}/steam-hours", server.id)).await;
    assert!(board.entries.is_empty());

    // back on: read again straight away
    set_steam(&app, &token, json!({ "share_hours": true })).await;
    for _ in 0..200 {
        if status(&app, &token).await.link.unwrap().recent_minutes == Some(600) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the hours were never read again");
}

#[tokio::test]
async fn a_private_profile_shows_no_hours() {
    let (app, fake) = setup().await;
    let (alex, token) = linked(&app, &fake, "alex", ALEX_STEAM).await;
    fake.played_recently(ALEX_STEAM, SPACE, 600);
    hours_tick(&app).await;
    fake.set_private(ALEX_STEAM, true);
    hours_tick(&app).await;
    let p: Profile = get_json(&app, &token, &format!("/users/{alex}/profile")).await;
    assert_eq!(p.stats.steam_recent_minutes, None);
}
