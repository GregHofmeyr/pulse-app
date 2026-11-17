//! Sports score pings (#385), against the fake API-Sports (`testing::fake_sports`): nothing here reaches the real
//! network. Dormant without a key; the team lists read once and cached; one card per match from the match-day banner
//! through the live score to full time, edited in place; pings off and no channel mean no requests at all; the daily
//! allowance (ours and API-Sports' own "requests" answer); a plan refusal backing off with a note; rugby read by day;
//! the Sports channel's rules; team logos as server media, a card that ran out of live data saying so, and the
//! owner's league limit (#406).
mod common;

use common::*;
use pulse_protocol::ids::{ChannelId, ServerId, UserId};
use pulse_protocol::rest::{
    Message, SportsCard, SportsChannel, SportsLeague, SportsPhase, SportsStatus, SportsTeams,
};
use pulse_server::config::Config;
use pulse_server::sports::api::Sport;
use pulse_server::testing::fake_sports::FakeSports;
use serde_json::json;

const PSL: u32 = 288;
const URC: u32 = 76;
const COASTAL: u32 = 10;
const HIGHVELD: u32 = 20;
const LAKESIDE: u32 = 30;

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// A kick-off `ago` seconds back, but never before today (UTC), so the day's schedule read includes it.
fn kicked_off(ago: i64) -> i64 {
    let n = now();
    (n - ago).max(n / 86_400 * 86_400 + 1)
}

async fn setup_with(tweak: impl FnOnce(&mut Config)) -> (TestApp, FakeSports) {
    let fake = FakeSports::start().await;
    let mut cfg = test_config();
    fake.configure(&mut cfg);
    tweak(&mut cfg);
    fake.add_team(PSL, COASTAL, "Coastal City");
    fake.add_team(PSL, HIGHVELD, "Highveld United");
    fake.add_team(PSL, LAKESIDE, "Lakeside Stars");
    fake.add_team(URC, 501, "Coastal XV");
    fake.add_team(URC, 502, "Highveld XV");
    (spawn_with(cfg).await, fake)
}

async fn setup() -> (TestApp, FakeSports) {
    setup_with(|_| {}).await
}

async fn join(app: &TestApp, token: &str, server: ServerId) {
    let r = post_json(app, token, &format!("/servers/{server}/join"), json!({})).await;
    assert!(r.status().is_success());
}

async fn put(app: &TestApp, token: &str, path: &str, body: serde_json::Value) -> reqwest::Response {
    app.http
        .put(app.url(path))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

async fn set_channel(app: &TestApp, token: &str, server: ServerId, ch: Option<ChannelId>) -> u16 {
    put(
        app,
        token,
        &format!("/servers/{server}/sports-channel"),
        json!({ "channel_id": ch }),
    )
    .await
    .status()
    .as_u16()
}

async fn follow(app: &TestApp, token: &str, league: &str, team: u32) -> reqwest::Response {
    post_json(
        app,
        token,
        "/me/sports/follows",
        json!({ "league": league, "team_id": team }),
    )
    .await
}

async fn teams(app: &TestApp, token: &str, league: &str) -> SportsTeams {
    get_json(app, token, &format!("/sports/teams?league={league}")).await
}

async fn tick(app: &TestApp) {
    pulse_server::sports::poller::tick(&app.state()).await;
}

async fn cards(app: &TestApp, token: &str, ch: ChannelId) -> Vec<(Message, SportsCard)> {
    let msgs: Vec<Message> =
        get_json(app, token, &format!("/channels/{ch}/messages?limit=100")).await;
    let mut out: Vec<_> = msgs
        .into_iter()
        .filter_map(|m| {
            let c = m.sports.clone()?;
            Some((m, *c))
        })
        .collect();
    out.sort_by_key(|a| a.0.id);
    out
}

struct World {
    app: TestApp,
    fake: FakeSports,
    alex: String,
    sam: String,
    sam_id: UserId,
    server: ServerId,
    general: ChannelId,
}

/// alex (owner) and sam in a server whose #general is its Sports channel; sam follows Coastal City.
async fn world_with(tweak: impl FnOnce(&mut Config)) -> World {
    let (app, fake) = setup_with(tweak).await;
    let (_, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let server = create_server(&app, &alex, "Braai crew").await;
    join(&app, &sam, server.id).await;
    let general = general(&app, &alex, server.id).await.id;
    assert_eq!(
        set_channel(&app, &alex, server.id, Some(general)).await,
        200
    );
    assert_eq!(teams(&app, &sam, "psl").await.teams.len(), 3);
    assert_eq!(follow(&app, &sam, "psl", COASTAL).await.status(), 200);
    World {
        app,
        fake,
        alex,
        sam,
        sam_id,
        server: server.id,
        general,
    }
}

async fn world() -> World {
    world_with(|_| {}).await
}

#[tokio::test]
async fn without_a_key_nothing_is_fetched_or_posted() {
    let (app, fake) = setup_with(|c| c.sports.api_key = None).await;
    let (alex_id, alex) = register(&app, "alex").await;
    let server = create_server(&app, &alex, "Braai crew").await;
    let general = general(&app, &alex, server.id).await.id;
    let st: SportsStatus = get_json(&app, &alex, "/me/sports").await;
    assert!(!st.available && st.pings && st.follows.is_empty());
    assert_eq!(
        st.leagues,
        vec![SportsLeague::Psl, SportsLeague::Urc, SportsLeague::Epl]
    );
    let r = app
        .http
        .get(app.url("/sports/teams?league=psl"))
        .bearer_auth(&alex)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    assert_eq!(follow(&app, &alex, "psl", COASTAL).await.status(), 400);
    // even with a follow and a channel in place (say a key was removed), the poller never calls
    assert_eq!(
        set_channel(&app, &alex, server.id, Some(general)).await,
        200
    );
    sqlx::query("INSERT INTO sports_follows (user_id, league, team_id, team_name, created_at) VALUES (?, 'psl', ?, 'Coastal City', '2026-01-01')")
        .bind(alex_id.to_string())
        .bind(COASTAL)
        .execute(&app.db)
        .await
        .unwrap();
    fake.add_match(
        PSL,
        1001,
        now() + 3600,
        (COASTAL, "Coastal City"),
        (HIGHVELD, "Highveld United"),
    );
    tick(&app).await;
    assert_eq!(fake.total_calls(), 0);
    assert!(cards(&app, &alex, general).await.is_empty());
}

#[tokio::test]
async fn team_lists_are_read_once_a_week_and_follows_come_from_them() {
    let (app, fake) = setup().await;
    let (_, sam) = register(&app, "sam").await;
    let psl = teams(&app, &sam, "psl").await;
    assert_eq!(
        psl.teams
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>(),
        ["Coastal City", "Highveld United", "Lakeside Stars"]
    );
    teams(&app, &sam, "psl").await;
    assert_eq!(fake.calls("football/teams"), 1, "cached");
    let q = fake.last_query("football/teams");
    assert_eq!(q.get("league").map(String::as_str), Some("288"));
    let season = pulse_server::sports::season_for(chrono::Utc::now().date_naive()).to_string();
    assert_eq!(q.get("season"), Some(&season));
    // rugby's flat team shape, from the rugby host
    let urc = teams(&app, &sam, "urc").await;
    assert_eq!(urc.teams.len(), 2);
    assert_eq!(fake.calls("rugby/teams"), 1);
    assert_eq!(
        fake.last_query("rugby/teams")
            .get("league")
            .map(String::as_str),
        Some("76")
    );
    // only a listed team can be followed
    assert_eq!(follow(&app, &sam, "psl", 999).await.status(), 400);
    assert_eq!(
        follow(&app, &sam, "epl", COASTAL).await.status(),
        400,
        "not in that league's list"
    );
    let st: SportsStatus = follow(&app, &sam, "urc", 502).await.json().await.unwrap();
    assert_eq!(st.follows.len(), 1);
    assert_eq!(
        (st.follows[0].league, st.follows[0].name.as_str()),
        (SportsLeague::Urc, "Highveld XV")
    );
    let r = app
        .http
        .delete(app.url("/me/sports/follows/urc/502"))
        .bearer_auth(&sam)
        .send()
        .await
        .unwrap();
    let st: SportsStatus = r.json().await.unwrap();
    assert!(st.follows.is_empty());
    let r = app
        .http
        .get(app.url("/sports/teams?league=nba"))
        .bearer_auth(&sam)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn one_card_per_match_from_the_banner_through_the_live_score_to_full_time() {
    let w = world().await;
    let (app, fake) = (&w.app, &w.fake);
    // later today: a match-day banner, no live reads before kick-off
    let later = now() + 2 * 3600;
    fake.add_match(
        PSL,
        2001,
        later,
        (COASTAL, "Coastal City"),
        (LAKESIDE, "Lakeside Stars"),
    );
    // a match nobody follows: never carded, never read live
    fake.add_match(
        PSL,
        2002,
        later,
        (HIGHVELD, "Highveld United"),
        (40, "Other FC"),
    );
    tick(app).await;
    assert_eq!(fake.calls("football/fixtures"), 1, "the schedule");
    let q = fake.last_query("football/fixtures");
    assert_eq!(q.get("league").map(String::as_str), Some("288"));
    assert!(q.contains_key("from") && q.contains_key("to"));
    let got = cards(app, &w.alex, w.general).await;
    assert_eq!(got.len(), 1);
    let (msg, card) = &got[0];
    assert_eq!(
        (card.phase, card.fixture_id, card.status.as_str()),
        (SportsPhase::Upcoming, 2001, "NS")
    );
    assert_eq!(
        (card.home.name.as_str(), card.home.score),
        ("Coastal City", None)
    );
    assert_eq!(card.followers, vec![w.sam_id]);
    assert!(
        msg.content
            .starts_with("⚽ Match day: Coastal City v Lakeside Stars, kick-off "),
        "{}",
        msg.content
    );
    assert!(msg.author_id.is_none());
    tick(app).await;
    assert_eq!(
        fake.calls("football/fixtures"),
        1,
        "nothing more until kick-off"
    );
    assert_eq!(cards(app, &w.alex, w.general).await.len(), 1);
}

#[tokio::test]
async fn a_live_match_is_read_by_id_and_its_card_edited_in_place() {
    let w = world().await;
    let (app, fake) = (&w.app, &w.fake);
    let kickoff = kicked_off(20 * 60);
    fake.add_match(
        PSL,
        3001,
        kickoff,
        (COASTAL, "Coastal City"),
        (HIGHVELD, "Highveld United"),
    );
    fake.set_score(3001, "1H", Some(18), 1, 0);
    fake.add_goal(3001, 12, COASTAL, "A. Striker", "Penalty");
    tick(app).await;
    // the schedule, then one live read with the events
    assert_eq!(fake.calls("football/fixtures"), 2);
    assert_eq!(
        fake.last_query("football/fixtures")
            .get("ids")
            .map(String::as_str),
        Some("3001")
    );
    let got = cards(app, &w.alex, w.general).await;
    assert_eq!(got.len(), 1);
    let first = got[0].0.id;
    let card = &got[0].1;
    assert_eq!(
        (card.phase, card.minute, card.home.score, card.away.score),
        (SportsPhase::Live, Some(18), Some(1), Some(0))
    );
    assert_eq!(card.scorers.len(), 1);
    assert_eq!(
        (
            card.scorers[0].name.as_str(),
            card.scorers[0].note.as_deref(),
            card.scorers[0].home
        ),
        ("A. Striker", Some("pen"), true)
    );
    // within the gap between live reads: nothing
    tick(app).await;
    assert_eq!(fake.calls("football/fixtures"), 2);
    // full time: the same card, edited
    fake.set_score(3001, "FT", Some(90), 1, 1);
    fake.add_goal(3001, 77, HIGHVELD, "E. Nine", "Normal Goal");
    app.sports.forget_live_read(Sport::Football);
    tick(app).await;
    assert_eq!(fake.calls("football/fixtures"), 3);
    let got = cards(app, &w.alex, w.general).await;
    assert_eq!(got.len(), 1, "edited, not posted again");
    assert_eq!(got[0].0.id, first);
    let card = &got[0].1;
    assert_eq!(
        (card.phase, card.minute, card.home.score, card.away.score),
        (SportsPhase::Final, None, Some(1), Some(1))
    );
    assert_eq!(
        card.scorers
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["A. Striker", "E. Nine"]
    );
    assert_eq!(
        got[0].0.content,
        "⚽ Full time: Coastal City 1 – 1 Highveld United"
    );
    // done: no more reads
    app.sports.forget_live_read(Sport::Football);
    tick(app).await;
    assert_eq!(fake.calls("football/fixtures"), 3);
}

#[tokio::test]
async fn pings_off_or_no_channel_means_no_requests() {
    let w = world().await;
    let (app, fake) = (&w.app, &w.fake);
    let before = fake.total_calls();
    fake.add_match(
        PSL,
        4001,
        now() + 3600,
        (COASTAL, "Coastal City"),
        (HIGHVELD, "Highveld United"),
    );
    let st: SportsStatus = put(app, &w.sam, "/me/sports", json!({ "pings": false }))
        .await
        .json()
        .await
        .unwrap();
    assert!(!st.pings && st.follows.len() == 1, "the follows are kept");
    tick(app).await;
    assert_eq!(fake.total_calls(), before);
    put(app, &w.sam, "/me/sports", json!({ "pings": true })).await;
    assert_eq!(set_channel(app, &w.alex, w.server, None).await, 200);
    tick(app).await;
    assert_eq!(fake.total_calls(), before, "no channel anywhere");
    assert_eq!(
        set_channel(app, &w.alex, w.server, Some(w.general)).await,
        200
    );
    tick(app).await;
    // the schedule (the card's two logos come from the media host, which spends no requests)
    assert_eq!(fake.total_calls() - fake.calls("media"), before + 1);
    assert_eq!(cards(app, &w.alex, w.general).await.len(), 1);
}

#[tokio::test]
async fn the_daily_allowance_is_never_passed() {
    let w = world_with(|c| c.sports.daily_cap = 3).await;
    let (app, fake) = (&w.app, &w.fake);
    // the team list took one
    assert_eq!(
        app.sports.api().used_today(&app.db, Sport::Football).await,
        1
    );
    fake.add_match(
        PSL,
        5001,
        kicked_off(10 * 60),
        (COASTAL, "Coastal City"),
        (HIGHVELD, "Highveld United"),
    );
    fake.set_score(5001, "1H", Some(9), 0, 0);
    tick(app).await;
    // the schedule took the second; a live read would leave less than the reserve, so none
    assert_eq!(fake.calls("football/fixtures"), 1);
    assert_eq!(
        app.sports.api().used_today(&app.db, Sport::Football).await,
        2
    );
    for _ in 0..3 {
        app.sports.forget_live_read(Sport::Football);
        tick(app).await;
    }
    assert_eq!(fake.calls("football/fixtures"), 1);
    // and nothing past the cap, whoever asks
    assert!(app.sports.api().take(&app.db, Sport::Football).await);
    assert!(!app.sports.api().take(&app.db, Sport::Football).await);
    assert_eq!(
        app.sports.api().used_today(&app.db, Sport::Football).await,
        3
    );
    // rugby has its own
    assert_eq!(app.sports.api().used_today(&app.db, Sport::Rugby).await, 0);
}

#[tokio::test]
async fn api_sports_saying_the_day_is_spent_stops_the_day() {
    let w = world().await;
    let (app, fake) = (&w.app, &w.fake);
    fake.add_match(
        PSL,
        6001,
        now() + 3600,
        (COASTAL, "Coastal City"),
        (HIGHVELD, "Highveld United"),
    );
    fake.spend_day();
    tick(app).await;
    assert_eq!(fake.calls("football/fixtures"), 1);
    assert_eq!(
        app.sports.api().used_today(&app.db, Sport::Football).await,
        app.cfg.sports.daily_cap
    );
    let st: SportsStatus = get_json(app, &w.sam, "/me/sports").await;
    assert!(
        st.note.as_deref().is_some_and(|n| n.contains("used up")),
        "{:?}",
        st.note
    );
    // the schedule's retry gap and the spent day both hold it
    sqlx::query("DELETE FROM sports_state")
        .execute(&app.db)
        .await
        .unwrap();
    tick(app).await;
    assert_eq!(fake.calls("football/fixtures"), 1);
    assert!(cards(app, &w.alex, w.general).await.is_empty());
}

#[tokio::test]
async fn a_plan_refusal_backs_off_with_a_note() {
    let w = world().await;
    let (app, fake) = (&w.app, &w.fake);
    fake.refuse_plan();
    tick(app).await;
    assert_eq!(fake.calls("football/fixtures"), 1);
    let st: SportsStatus = get_json(app, &w.sam, "/me/sports").await;
    assert!(
        st.note.as_deref().is_some_and(|n| n.contains("plan")),
        "{:?}",
        st.note
    );
    // backing off (and the schedule's own retry gap): no second request
    sqlx::query("DELETE FROM sports_state")
        .execute(&app.db)
        .await
        .unwrap();
    tick(app).await;
    assert_eq!(fake.calls("football/fixtures"), 1);
}

#[tokio::test]
async fn rugby_is_read_by_the_day_and_shows_the_score() {
    let w = world().await;
    let (app, fake) = (&w.app, &w.fake);
    teams(app, &w.sam, "urc").await;
    assert_eq!(follow(app, &w.sam, "urc", 502).await.status(), 200);
    let kickoff = kicked_off(30 * 60);
    fake.add_match(
        URC,
        30_001,
        kickoff,
        (501, "Coastal XV"),
        (502, "Highveld XV"),
    );
    fake.set_score(30_001, "1H", None, 7, 10);
    tick(app).await;
    // the season's games once, then the day's for the live read
    assert_eq!(fake.calls("rugby/games"), 2);
    let day = chrono::DateTime::from_timestamp(kickoff, 0)
        .unwrap()
        .date_naive()
        .to_string();
    assert_eq!(fake.last_query("rugby/games").get("date"), Some(&day));
    let got = cards(app, &w.alex, w.general).await;
    assert_eq!(got.len(), 1);
    let (msg, card) = &got[0];
    assert_eq!(
        (card.league, card.phase, card.home.score, card.away.score),
        (SportsLeague::Urc, SportsPhase::Live, Some(7), Some(10))
    );
    assert!(card.scorers.is_empty());
    assert_eq!(msg.content, "🏉 Live: Coastal XV 7 – 10 Highveld XV");
    fake.set_score(30_001, "FT", None, 27, 20);
    app.sports.forget_live_read(Sport::Rugby);
    tick(app).await;
    let got = cards(app, &w.alex, w.general).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].1.phase, SportsPhase::Final);
    assert_eq!(
        got[0].0.content,
        "🏉 Full time: Coastal XV 27 – 20 Highveld XV"
    );
    // football wasn't touched beyond its team list
    assert_eq!(
        fake.calls("football/fixtures"),
        1,
        "PSL's schedule (sam still follows Coastal City)"
    );
}

#[tokio::test]
async fn the_sports_channel_is_the_owners_pick_of_a_text_channel() {
    let w = world().await;
    let app = &w.app;
    let got: SportsChannel = get_json(
        app,
        &w.sam,
        &format!("/servers/{}/sports-channel", w.server),
    )
    .await;
    assert_eq!(got.channel_id, Some(w.general));
    assert_eq!(set_channel(app, &w.sam, w.server, None).await, 403);
    let chans: Vec<pulse_protocol::rest::Channel> =
        get_json(app, &w.alex, &format!("/servers/{}/channels", w.server)).await;
    let voice = chans
        .iter()
        .find(|c| c.kind == pulse_protocol::rest::ChannelKind::Voice)
        .unwrap();
    assert_eq!(
        set_channel(app, &w.alex, w.server, Some(voice.id)).await,
        400
    );
    let (_, kim) = register(app, "kim").await;
    let other = create_server(app, &kim, "Elsewhere").await;
    let elsewhere = general(app, &kim, other.id).await.id;
    assert_eq!(
        set_channel(app, &w.alex, w.server, Some(elsewhere)).await,
        400
    );
    let r = app
        .http
        .get(app.url(&format!("/servers/{}/sports-channel", other.id)))
        .bearer_auth(&w.alex)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403, "not a member");
    assert_eq!(set_channel(app, &w.alex, w.server, None).await, 200);
    let got: SportsChannel = get_json(
        app,
        &w.alex,
        &format!("/servers/{}/sports-channel", w.server),
    )
    .await;
    assert_eq!(got.channel_id, None);
}

#[tokio::test]
async fn cards_carry_both_teams_logos_as_server_media() {
    let w = world().await;
    let (app, fake) = (&w.app, &w.fake);
    let later = now() + 2 * 3600;
    fake.add_match(
        PSL,
        6001,
        later,
        (COASTAL, "Coastal City"),
        (LAKESIDE, "Lakeside Stars"),
    );
    // a team API-Sports has no logo for
    let no_logo = pulse_server::testing::fake_sports::NO_LOGO;
    fake.add_match(
        PSL,
        6002,
        later + 600,
        (COASTAL, "Coastal City"),
        (no_logo, "New FC"),
    );
    tick(app).await;
    let got = cards(app, &w.alex, w.general).await;
    assert_eq!(got.len(), 2);
    let first = &got.iter().find(|c| c.1.fixture_id == 6001).unwrap().1;
    let (home, away) = (
        first.home.logo.clone().expect("home logo"),
        first.away.logo.clone().expect("away logo"),
    );
    assert_eq!((home.width, home.height), (96, 96));
    assert_eq!((away.width, away.height), (96, 96));
    // stored media anyone signed in can load
    let r = app
        .http
        .get(app.url(&format!("/media/{}", home.hash)))
        .bearer_auth(&w.sam)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let second = &got.iter().find(|c| c.1.fixture_id == 6002).unwrap().1;
    assert_eq!(
        second.home.logo.as_ref().map(|l| &l.hash),
        Some(&home.hash),
        "the same team's logo, remembered"
    );
    assert!(second.away.logo.is_none(), "none to be had");
    // Coastal and Lakeside fetched once each, the missing one tried once
    assert_eq!(fake.calls("media"), 3);
    // a card edit later fetches nothing again (the missing one waits an hour)
    let f = pulse_server::sports::poller::load(&app.state(), SportsLeague::Psl, 6002)
        .await
        .unwrap()
        .unwrap();
    pulse_server::sports::poller::sync_cards(&app.state(), &f, false)
        .await
        .unwrap();
    assert_eq!(fake.calls("media"), 3);
    // kept from the media sweep
    assert_eq!(count(app, "SELECT COUNT(*) FROM sports_media").await, 2);
    assert_eq!(
        fake.calls("football/fixtures"),
        1,
        "logos spend no requests"
    );
}

#[tokio::test]
async fn a_card_whose_window_passes_without_live_data_says_so() {
    let w = world().await;
    let (app, fake) = (&w.app, &w.fake);
    let s = app.state();
    let later = now() + 3600;
    fake.add_match(
        PSL,
        7001,
        later,
        (COASTAL, "Coastal City"),
        (HIGHVELD, "Highveld United"),
    );
    fake.add_match(
        PSL,
        7002,
        later + 60,
        (COASTAL, "Coastal City"),
        (LAKESIDE, "Lakeside Stars"),
    );
    tick(app).await;
    assert_eq!(cards(app, &w.alex, w.general).await.len(), 2);
    // 7001 never got a live read past match day, and its window is gone; 7002 went live and was never called over
    let window = s.cfg.sports.window.as_secs() as i64;
    let max_window = s.cfg.sports.max_window.as_secs() as i64;
    sqlx::query("UPDATE sports_fixtures SET kickoff = ? WHERE fixture_id = 7001")
        .bind(now() - window - 60)
        .execute(&s.db)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE sports_fixtures SET kickoff = ?, status = '2H', home_score = 1, away_score = 0, minute = 70
         WHERE fixture_id = 7002",
    )
    .bind(now() - max_window - 60)
    .execute(&s.db)
    .await
    .unwrap();
    let before = fake.total_calls();
    tick(app).await;
    assert_eq!(fake.total_calls(), before, "nothing live to read");
    let got = cards(app, &w.alex, w.general).await;
    let (m1, c1) = got.iter().find(|c| c.1.fixture_id == 7001).unwrap();
    assert!(c1.no_updates);
    assert_eq!(c1.phase, SportsPhase::Upcoming);
    assert_eq!(
        m1.content,
        "⚽ Coastal City v Highveld United: no live updates, check back later"
    );
    let (m2, c2) = got.iter().find(|c| c.1.fixture_id == 7002).unwrap();
    assert!(c2.no_updates);
    assert_eq!((c2.home.score, c2.away.score), (Some(1), Some(0)));
    assert_eq!(
        m2.content,
        "⚽ Coastal City 1 – 0 Lakeside Stars: no live updates, check back later"
    );
    // done: never read again, and the card stays as it is
    let done: i64 = count(
        app,
        "SELECT COUNT(*) FROM sports_fixtures WHERE fixture_id IN (7001, 7002) AND done = 1",
    )
    .await;
    assert_eq!(done, 2);
    app.sports.forget_live_read(Sport::Football);
    tick(app).await;
    assert_eq!(fake.total_calls(), before);
}

#[tokio::test]
async fn the_owner_can_limit_which_leagues_post() {
    let w = world().await;
    let (app, fake) = (&w.app, &w.fake);
    let path = format!("/servers/{}/sports-channel", w.server);
    // only the owner, never empty, never without a channel
    let r = put(
        app,
        &w.sam,
        &path,
        json!({"channel_id": w.general, "leagues": ["urc"]}),
    )
    .await;
    assert_eq!(r.status(), 403);
    let r = put(
        app,
        &w.alex,
        &path,
        json!({"channel_id": w.general, "leagues": []}),
    )
    .await;
    assert_eq!(r.status(), 400);
    let r = put(
        app,
        &w.alex,
        &path,
        json!({"channel_id": null, "leagues": ["urc"]}),
    )
    .await;
    assert_eq!(r.status(), 400);
    let got: SportsChannel = put(
        app,
        &w.alex,
        &path,
        json!({"channel_id": w.general, "leagues": ["urc"]}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(got.leagues, Some(vec![SportsLeague::Urc]));
    // picking the channel again keeps the limit; everyone sees it
    assert_eq!(
        set_channel(app, &w.alex, w.server, Some(w.general)).await,
        200
    );
    let got: SportsChannel = get_json(app, &w.sam, &path).await;
    assert_eq!(got.leagues, Some(vec![SportsLeague::Urc]));

    // sam follows a PSL team: this server takes no PSL, so nothing is read or posted
    fake.add_match(
        PSL,
        8001,
        now() + 3600,
        (COASTAL, "Coastal City"),
        (HIGHVELD, "Highveld United"),
    );
    let before = fake.total_calls();
    tick(app).await;
    assert_eq!(fake.total_calls(), before, "no league to read");
    assert!(cards(app, &w.alex, w.general).await.is_empty());

    // every league again: no limit, and the card comes
    let got: SportsChannel = put(
        app,
        &w.alex,
        &path,
        json!({"channel_id": w.general, "leagues": ["psl", "urc", "epl"]}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(got.leagues, None);
    tick(app).await;
    assert_eq!(cards(app, &w.alex, w.general).await.len(), 1);
}
