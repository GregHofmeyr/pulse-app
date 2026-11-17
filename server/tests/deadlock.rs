//! Deadlock (#390), against the fake deadlock-api (`testing::fake_deadlock`): nothing here reaches the real network.
//! The Deadlock channel's rules, post-match cards (the baseline, one card per match with friends merged, art through
//! the server, the switches), ranks on the member list, the weekly rank movers, squad stats, patch notes (and Steam's
//! news as the fallback), going dark and coming back, the history allowance's 429, the key header, and unlinking.
mod common;

use common::*;
use pulse_protocol::ids::{ChannelId, ServerId, UserId};
use pulse_protocol::rest::{
    DeadlockCard, DeadlockChannel, DeadlockMatchCard, DeadlockProfile, DeadlockRanks, Message,
    SteamStatus,
};
use pulse_server::config::Config;
use pulse_server::testing::fake_deadlock::{FakeDeadlock, HAZE, INFERNUS};
use serde_json::json;

const ALEX_STEAM: &str = "76500000000000001";
const SAM_STEAM: &str = "76500000000000002";
const KIM_STEAM: &str = "76500000000000003";
const ALEX: u32 = 1;
const SAM: u32 = 2;
const KIM: u32 = 3;

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

async fn setup_with(tweak: impl FnOnce(&mut Config)) -> (TestApp, FakeDeadlock) {
    let fake = FakeDeadlock::start().await;
    let mut cfg = test_config();
    fake.configure(&mut cfg);
    tweak(&mut cfg);
    (spawn_with(cfg).await, fake)
}

async fn setup() -> (TestApp, FakeDeadlock) {
    setup_with(|_| {}).await
}

/// Link `user` to `steam_id` straight in the database (the OpenID flow is `tests/steam.rs`'s).
async fn link(app: &TestApp, user: UserId, steam_id: &str) {
    sqlx::query(
        "INSERT INTO steam_links (user_id, steam_id, linked_at, backfill) VALUES (?, ?, ?, 'done')",
    )
    .bind(user.to_string())
    .bind(steam_id)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(&app.db)
    .await
    .unwrap();
}

async fn join(app: &TestApp, token: &str, server: ServerId) {
    let r = post_json(app, token, &format!("/servers/{server}/join"), json!({})).await;
    assert!(r.status().is_success());
}

async fn set_channel(app: &TestApp, token: &str, server: ServerId, body: serde_json::Value) -> u16 {
    app.http
        .put(app.url(&format!("/servers/{server}/deadlock-channel")))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
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

async fn matches_tick(app: &TestApp) {
    pulse_server::deadlock::matches::tick(&app.state()).await;
}

async fn cards(app: &TestApp, token: &str, ch: ChannelId) -> Vec<(Message, DeadlockCard)> {
    let msgs: Vec<Message> =
        get_json(app, token, &format!("/channels/{ch}/messages?limit=100")).await;
    let mut out: Vec<_> = msgs
        .into_iter()
        .filter_map(|m| {
            let c = m.deadlock.clone()?;
            Some((m, *c))
        })
        .collect();
    out.sort_by_key(|a| a.0.id);
    out
}

fn match_cards(all: &[(Message, DeadlockCard)]) -> Vec<(Message, DeadlockMatchCard)> {
    all.iter()
        .filter_map(|(m, c)| match c {
            DeadlockCard::Match(mc) => Some((m.clone(), mc.clone())),
            _ => None,
        })
        .collect()
}

/// alex (owner) and sam in a server whose #general is its Deadlock channel; both linked.
struct World {
    app: TestApp,
    fake: FakeDeadlock,
    alex: (UserId, String),
    sam: (UserId, String),
    server: ServerId,
    general: ChannelId,
}

async fn world() -> World {
    let (app, fake) = setup().await;
    let alex = register(&app, "alex").await;
    let sam = register(&app, "sam").await;
    let server = create_server(&app, &alex.1, "Squad").await;
    join(&app, &sam.1, server.id).await;
    let general = general(&app, &alex.1, server.id).await.id;
    assert_eq!(
        set_channel(&app, &alex.1, server.id, json!({ "channel_id": general })).await,
        200
    );
    link(&app, alex.0, ALEX_STEAM).await;
    link(&app, sam.0, SAM_STEAM).await;
    World {
        app,
        fake,
        alex,
        sam,
        server: server.id,
        general,
    }
}

#[tokio::test]
async fn only_the_owner_picks_a_text_channel_and_the_switches() {
    let (app, _fake) = setup().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let (_, kim) = register(&app, "kim").await;
    let server = create_server(&app, &alex, "Squad").await;
    join(&app, &sam, server.id).await;
    let general = general(&app, &alex, server.id).await.id;
    let path = format!("/servers/{}/deadlock-channel", server.id);
    let got: DeadlockChannel = get_json(&app, &sam, &path).await;
    assert_eq!(got.channel_id, None, "none by default");
    assert!(
        got.match_cards && got.rank_movers && got.patch_notes,
        "every kind on by default"
    );
    assert_eq!(
        set_channel(&app, &sam, server.id, json!({ "channel_id": general })).await,
        403
    );
    let voice = post_json(
        &app,
        &alex,
        &format!("/servers/{}/channels", server.id),
        json!({ "kind": "voice", "name": "Lounge" }),
    )
    .await;
    let voice: pulse_protocol::rest::Channel = voice.json().await.unwrap();
    assert_eq!(
        set_channel(&app, &alex, server.id, json!({ "channel_id": voice.id })).await,
        400
    );
    assert_eq!(
        set_channel(
            &app,
            &alex,
            server.id,
            json!({ "channel_id": general, "patch_notes": false })
        )
        .await,
        200
    );
    let got: DeadlockChannel = get_json(&app, &sam, &path).await;
    assert_eq!(got.channel_id, Some(general));
    assert!(got.match_cards && !got.patch_notes);
    // a switch left out stays
    assert_eq!(
        set_channel(
            &app,
            &alex,
            server.id,
            json!({ "channel_id": general, "rank_movers": false })
        )
        .await,
        200
    );
    let got: DeadlockChannel = get_json(&app, &alex, &path).await;
    assert!(!got.patch_notes && !got.rank_movers);
    assert_eq!(
        set_channel(&app, &alex, server.id, json!({ "channel_id": null })).await,
        200
    );
    let got: DeadlockChannel = get_json(&app, &alex, &path).await;
    assert_eq!(got.channel_id, None);
    let r = app
        .http
        .get(app.url(&path))
        .bearer_auth(&kim)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403, "a stranger can't look");
}

#[tokio::test]
async fn new_matches_post_one_card_per_match_with_friends_on_it() {
    let w = world().await;
    let (app, fake) = (&w.app, &w.fake);
    // played before Pulse looked: the baseline, never announced
    fake.add_match(ALEX, 50_000_001, HAZE, (1, 9, 2), false, now() - 7200, None);
    matches_tick(app).await;
    assert!(
        cards(app, &w.alex.1, w.general).await.is_empty(),
        "the first read is a baseline"
    );
    assert_eq!(fake.calls("match-history"), 2, "alex and sam read once");

    // alex and sam in the same new ranked match, on opposite results
    fake.set_rank(ALEX, 83);
    fake.add_match(
        ALEX,
        50_000_002,
        INFERNUS,
        (12, 3, 8),
        true,
        now() - 2000,
        Some((112, 83)),
    );
    fake.add_match(
        SAM,
        50_000_002,
        HAZE,
        (4, 7, 10),
        false,
        now() - 2000,
        Some((-90, 75)),
    );
    for who in [w.alex.0, w.sam.0] {
        pulse_server::deadlock::matches::poll_one(
            &app.state(),
            who,
            if who == w.alex.0 { ALEX } else { SAM },
        )
        .await
        .unwrap();
    }
    let all = cards(app, &w.sam.1, w.general).await;
    let mc = match_cards(&all);
    assert_eq!(mc.len(), 1, "one card for the match: {all:?}");
    let (msg, card) = &mc[0];
    assert_eq!(card.match_id, 50_000_002);
    assert_eq!(card.duration_s, 1864);
    assert_eq!(card.players.len(), 2);
    let a = &card.players[0];
    assert_eq!(
        (
            a.user_id,
            a.hero.as_str(),
            a.kills,
            a.deaths,
            a.assists,
            a.won
        ),
        (w.alex.0, "Infernus", 12, 3, 8, true)
    );
    assert_eq!(a.rank_delta, Some(112));
    let rank = a.rank.as_ref().unwrap();
    assert_eq!(
        (rank.tier, rank.subrank, rank.name.as_str()),
        (8, 3, "Oracle")
    );
    assert!(rank.image.is_some(), "the rank badge, the server's copy");
    assert!(a.hero_art.is_some(), "hero art, the server's copy");
    let s = &card.players[1];
    assert_eq!(
        (s.user_id, s.hero.as_str(), s.won),
        (w.sam.0, "Haze", false)
    );
    assert_eq!(msg.author_id, None);
    assert!(
        msg.content.contains("alex won as Infernus · 12/3/8"),
        "{}",
        msg.content
    );
    assert!(msg.content.contains("sam lost as Haze"), "{}", msg.content);
    assert!(fake.calls("art") >= 3);

    // read again: nothing new, nothing posted
    pulse_server::deadlock::matches::poll_one(&app.state(), w.alex.0, ALEX)
        .await
        .unwrap();
    assert_eq!(
        match_cards(&cards(app, &w.alex.1, w.general).await).len(),
        1
    );
}

#[tokio::test]
async fn the_switches_keep_matches_and_ranks_private() {
    let w = world().await;
    let (app, fake) = (&w.app, &w.fake);
    matches_tick(app).await; // baselines
    let st = set_steam(app, &w.alex.1, json!({ "deadlock_matches": false })).await;
    let l = st.link.unwrap();
    assert!(!l.deadlock_matches && l.deadlock_rank && l.deadlock_squad);
    set_steam(app, &w.sam.1, json!({ "deadlock_rank": false })).await;
    fake.add_match(
        ALEX,
        50_000_010,
        INFERNUS,
        (1, 1, 1),
        true,
        now() - 600,
        None,
    );
    fake.add_match(
        SAM,
        50_000_011,
        HAZE,
        (2, 2, 2),
        true,
        now() - 600,
        Some((50, 75)),
    );
    let before = fake.calls("match-history");
    // forget when they were read: both due now, but alex's cards are off
    app.deadlock.forget(w.sam.0);
    matches_tick(app).await;
    assert_eq!(
        fake.calls("match-history"),
        before + 1,
        "alex's history isn't read with cards off"
    );
    let mc = match_cards(&cards(app, &w.alex.1, w.general).await);
    assert_eq!(mc.len(), 1);
    let p = &mc[0].1.players[0];
    assert_eq!(p.user_id, w.sam.0);
    assert_eq!(
        (p.rank_delta, p.rank.is_some()),
        (None, false),
        "sam doesn't show their rank"
    );

    // back on: what was played meanwhile isn't announced (a new baseline)
    set_steam(app, &w.alex.1, json!({ "deadlock_matches": true })).await;
    matches_tick(app).await;
    assert_eq!(
        match_cards(&cards(app, &w.alex.1, w.general).await).len(),
        1
    );
}

#[tokio::test]
async fn no_channel_no_card_and_matches_still_count_for_the_profile() {
    let w = world().await;
    let (app, fake) = (&w.app, &w.fake);
    assert_eq!(
        set_channel(app, &w.alex.1, w.server, json!({ "channel_id": null })).await,
        200
    );
    matches_tick(app).await;
    fake.add_match(
        ALEX,
        50_000_020,
        INFERNUS,
        (7, 2, 5),
        true,
        now() - 300,
        None,
    );
    pulse_server::deadlock::matches::poll_one(&app.state(), w.alex.0, ALEX)
        .await
        .unwrap();
    assert!(cards(app, &w.alex.1, w.general).await.is_empty());
    let p: DeadlockProfile = get_json(
        app,
        &w.sam.1,
        &format!("/servers/{}/members/{}/deadlock", w.server, w.alex.0),
    )
    .await;
    let last = p.last_match.unwrap();
    assert_eq!(
        (last.hero.as_str(), last.kills, last.won),
        ("Infernus", 7, true)
    );
}

#[tokio::test]
async fn ranks_show_on_the_member_list_and_movers_post_weekly() {
    let w = world().await;
    let (app, fake) = (&w.app, &w.fake);
    fake.set_rank(ALEX, 83);
    fake.set_rank(SAM, 75);
    let s = app.state();
    pulse_server::deadlock::ranks::refresh(&s).await.unwrap();
    let r: DeadlockRanks = get_json(
        app,
        &w.sam.1,
        &format!("/servers/{}/deadlock-ranks", w.server),
    )
    .await;
    assert_eq!(r.entries.len(), 2);
    let a = r.entries.iter().find(|e| e.user_id == w.alex.0).unwrap();
    assert_eq!((a.rank.badge, a.rank.name.as_str()), (83, "Oracle"));
    assert!(a.rank.image.is_some());
    assert_eq!(fake.calls("rank"), 1, "one batch call for everyone");

    // the first movers run starts the clock; a week later the movement since posts, most up first
    let t0 = chrono::Utc::now();
    pulse_server::deadlock::ranks::movers_at(&s, t0)
        .await
        .unwrap();
    fake.set_rank(ALEX, 85);
    fake.set_rank(SAM, 72);
    pulse_server::deadlock::ranks::refresh(&s).await.unwrap();
    pulse_server::deadlock::ranks::movers_at(&s, t0 + chrono::Duration::days(3))
        .await
        .unwrap();
    assert!(
        cards(app, &w.alex.1, w.general).await.is_empty(),
        "not a week yet"
    );
    pulse_server::deadlock::ranks::movers_at(&s, t0 + chrono::Duration::days(7))
        .await
        .unwrap();
    let all = cards(app, &w.alex.1, w.general).await;
    let DeadlockCard::RankMovers(m) = &all.last().unwrap().1 else {
        panic!("no movers card: {all:?}");
    };
    assert_eq!(m.movers.len(), 2);
    assert_eq!((m.movers[0].user_id, m.movers[0].steps), (w.alex.0, 2));
    assert_eq!((m.movers[1].user_id, m.movers[1].steps), (w.sam.0, -3));
    assert!(
        all.last()
            .unwrap()
            .0
            .content
            .contains("alex Oracle 3 → Oracle 5")
    );
    // nobody moved the next week: nothing posts
    pulse_server::deadlock::ranks::movers_at(&s, t0 + chrono::Duration::days(14))
        .await
        .unwrap();
    assert_eq!(cards(app, &w.alex.1, w.general).await.len(), 1);

    // sam stops showing their rank: gone from the list
    set_steam(app, &w.sam.1, json!({ "deadlock_rank": false })).await;
    let r: DeadlockRanks = get_json(
        app,
        &w.alex.1,
        &format!("/servers/{}/deadlock-ranks", w.server),
    )
    .await;
    assert_eq!(
        r.entries.iter().map(|e| e.user_id).collect::<Vec<_>>(),
        vec![w.alex.0]
    );
}

#[tokio::test]
async fn squad_stats_show_heroes_games_together_and_the_best_duo() {
    let w = world().await;
    let (app, fake) = (&w.app, &w.fake);
    let kim = register(app, "kim").await;
    join(app, &kim.1, w.server).await;
    link(app, kim.0, KIM_STEAM).await;
    fake.hero_stats(ALEX, INFERNUS, 22, 40);
    fake.hero_stats(ALEX, HAZE, 3, 5);
    fake.mate(ALEX, SAM, 6, 10);
    fake.mate(ALEX, KIM, 4, 5);
    fake.mate(ALEX, 99, 50, 50); // a stranger: never shown
    let path = format!("/servers/{}/members/{}/deadlock", w.server, w.alex.0);
    let p: DeadlockProfile = get_json(app, &w.sam.1, &path).await;
    assert_eq!(p.heroes.len(), 2);
    assert_eq!(
        (
            p.heroes[0].hero.as_str(),
            p.heroes[0].matches,
            p.heroes[0].wins
        ),
        ("Infernus", 40, 22)
    );
    assert!(p.heroes[0].icon.is_some());
    let with = p.with_you.unwrap();
    assert_eq!((with.user_id, with.matches, with.wins), (w.sam.0, 10, 6));
    assert_eq!(p.best_duo.unwrap().user_id, kim.0, "4 of 5 beats 6 of 10");
    // cached: a second look reads nothing new
    let before = fake.calls("mate-stats");
    let _: DeadlockProfile = get_json(app, &kim.1, &path).await;
    assert_eq!(fake.calls("mate-stats"), before);

    // kim stops sharing squad stats: not anyone's duo any more
    set_steam(app, &kim.1, json!({ "deadlock_squad": false })).await;
    let p: DeadlockProfile = get_json(app, &w.sam.1, &path).await;
    assert_eq!(p.best_duo.unwrap().user_id, w.sam.0);
    // alex stops: no heroes or duos at all
    set_steam(app, &w.alex.1, json!({ "deadlock_squad": false })).await;
    let p: DeadlockProfile = get_json(app, &w.sam.1, &path).await;
    assert!(p.heroes.is_empty() && p.with_you.is_none() && p.best_duo.is_none());
    // not a member: not found
    let jo = register(app, "jo").await;
    let r = app
        .http
        .get(app.url(&format!("/servers/{}/members/{}/deadlock", w.server, jo.0)))
        .bearer_auth(&w.sam.1)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn patch_notes_post_once_after_a_quiet_first_look() {
    let w = world().await;
    let (app, fake) = (&w.app, &w.fake);
    let s = app.state();
    fake.add_patch(
        "Old Update",
        "https://news.example/old",
        "2026-01-01T10:00:00Z",
        "<p>old</p>",
    );
    pulse_server::deadlock::patches::tick(&s).await.unwrap();
    assert!(
        cards(app, &w.alex.1, w.general).await.is_empty(),
        "the first look only remembers"
    );
    let fresh = chrono::Utc::now().to_rfc3339();
    fake.add_patch(
        "Minor Update - 11-04-2026",
        "https://news.example/1",
        &fresh,
        "<p>- Haze: more smoke.</p><p>- Infernus: hotter.</p>",
    );
    // the same update from the forum, under another title
    let later = (chrono::Utc::now() + chrono::Duration::minutes(5)).to_rfc3339();
    fake.add_patch(
        "11-04-2026",
        "https://forum.example/2",
        &later,
        "<p>same</p>",
    );
    fake.add_patch("Bad Link", "http://news.example/3", &fresh, "plain");
    pulse_server::deadlock::patches::tick(&s).await.unwrap();
    pulse_server::deadlock::patches::tick(&s).await.unwrap();
    let all = cards(app, &w.alex.1, w.general).await;
    assert_eq!(all.len(), 1, "{all:?}");
    let DeadlockCard::Patch(p) = &all[0].1 else {
        panic!("not a patch card");
    };
    assert_eq!(p.title, "Minor Update - 11-04-2026");
    assert_eq!(p.link, "https://news.example/1");
    assert_eq!(
        p.summary.as_deref(),
        Some("- Haze: more smoke. - Infernus: hotter.")
    );
    assert_eq!(
        p.heroes.iter().map(|h| h.name.as_str()).collect::<Vec<_>>(),
        vec!["Haze", "Infernus"]
    );
    assert!(p.heroes.iter().all(|h| h.icon.is_some()));
}

#[tokio::test]
async fn steam_news_stands_in_when_the_patches_feed_fails() {
    let (app, fake) = setup().await;
    let mut cfg = (*app.cfg).clone();
    cfg.steam.api_base = format!("http://127.0.0.1:{}", fake.port);
    let mut s = app.state();
    s.cfg = std::sync::Arc::new(cfg);
    let (_, alex) = register(&app, "alex").await;
    let server = create_server(&app, &alex, "Squad").await;
    let general = general(&app, &alex, server.id).await.id;
    assert_eq!(
        set_channel(&app, &alex, server.id, json!({ "channel_id": general })).await,
        200
    );
    fake.add_patch(
        "Old",
        "https://news.example/old",
        "2026-01-01T10:00:00Z",
        "x",
    );
    pulse_server::deadlock::patches::tick(&s).await.unwrap();
    fake.set_down(true);
    fake.add_patch(
        "Gameplay Update",
        "https://news.example/g",
        &chrono::Utc::now().to_rfc3339(),
        "Haze.",
    );
    pulse_server::deadlock::patches::tick(&s).await.unwrap();
    assert!(fake.calls("steam-news") >= 1);
    let all = cards(&app, &alex, general).await;
    assert_eq!(all.len(), 1);
    assert!(matches!(&all[0].1, DeadlockCard::Patch(p) if p.title == "Gameplay Update"));
}

#[tokio::test]
async fn when_deadlock_api_goes_dark_the_bits_hide_until_it_answers() {
    let w = world().await;
    let (app, fake) = (&w.app, &w.fake);
    fake.set_rank(ALEX, 83);
    pulse_server::deadlock::ranks::refresh(&app.state())
        .await
        .unwrap();
    let st: SteamStatus = get_json(app, &w.alex.1, "/me/steam").await;
    assert!(st.deadlock_available);
    fake.set_down(true);
    for _ in 0..3 {
        pulse_server::deadlock::ranks::refresh(&app.state())
            .await
            .unwrap();
    }
    assert!(!app.deadlock.api().available());
    let st: SteamStatus = get_json(app, &w.alex.1, "/me/steam").await;
    assert!(!st.deadlock_available);
    let r: DeadlockRanks = get_json(
        app,
        &w.sam.1,
        &format!("/servers/{}/deadlock-ranks", w.server),
    )
    .await;
    assert!(r.entries.is_empty(), "no stale ranks while it's dark");
    let p: DeadlockProfile = get_json(
        app,
        &w.sam.1,
        &format!("/servers/{}/members/{}/deadlock", w.server, w.alex.0),
    )
    .await;
    assert_eq!(p, DeadlockProfile::default());
    fake.set_down(false);
    pulse_server::deadlock::ranks::refresh(&app.state())
        .await
        .unwrap();
    assert!(app.deadlock.api().available(), "back on its first answer");
    let r: DeadlockRanks = get_json(
        app,
        &w.sam.1,
        &format!("/servers/{}/deadlock-ranks", w.server),
    )
    .await;
    assert_eq!(r.entries.len(), 1);
}

#[tokio::test]
async fn a_429_on_match_history_pauses_it_and_the_allowance_holds() {
    let w = world().await;
    let (app, fake) = (&w.app, &w.fake);
    fake.limit_history(true);
    matches_tick(app).await;
    assert_eq!(
        fake.calls("match-history"),
        1,
        "the round stops at the first 429"
    );
    assert!(app.deadlock.history_paused());
    matches_tick(app).await;
    assert_eq!(fake.calls("match-history"), 1, "paused: not called again");
    assert!(app.deadlock.api().available(), "a 429 isn't going dark");

    // without a key, at most 8 reads an hour, however many are due
    let (app, fake) = setup().await;
    let (_, owner) = register(&app, "alex").await;
    let _ = create_server(&app, &owner, "Squad").await;
    for i in 1..=12u32 {
        let (u, _) = register(&app, &format!("kim{i}")).await;
        link(&app, u, &format!("765000000000000{i:02}")).await;
    }
    matches_tick(&app).await;
    assert_eq!(fake.calls("match-history"), 8);
}

#[tokio::test]
async fn the_key_goes_in_its_header() {
    let (app, fake) = setup_with(|c| c.deadlock.api_key = Some("dl-test-key".into())).await;
    fake.require_key("dl-test-key");
    let (alex, _) = register(&app, "alex").await;
    link(&app, alex, ALEX_STEAM).await;
    fake.set_rank(ALEX, 91);
    pulse_server::deadlock::ranks::refresh(&app.state())
        .await
        .unwrap();
    assert!(app.deadlock.api().available());
    let badge: i64 = sqlx::query_scalar("SELECT badge FROM deadlock_players")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(badge, 91);
}

#[tokio::test]
async fn unlinking_forgets_what_was_read() {
    let w = world().await;
    let app = &w.app;
    matches_tick(app).await;
    assert_eq!(count(app, "SELECT COUNT(*) FROM deadlock_players").await, 2);
    let r = app
        .http
        .delete(app.url("/me/steam"))
        .bearer_auth(&w.alex.1)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(count(app, "SELECT COUNT(*) FROM deadlock_players").await, 1);
}

/// An archived channel (#303) posts nothing, so it can't be picked as the Deadlock channel.
#[tokio::test]
async fn an_archived_channel_cant_be_the_deadlock_channel() {
    let (app, _fake) = setup().await;
    let (_, alex) = register(&app, "alex").await;
    let server = create_server(&app, &alex, "Squad").await;
    let general = general(&app, &alex, server.id).await.id;
    let r = app
        .http
        .put(app.url(&format!("/channels/{general}/archive")))
        .bearer_auth(&alex)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.status());
    assert_eq!(
        set_channel(&app, &alex, server.id, json!({ "channel_id": general })).await,
        403
    );
}
