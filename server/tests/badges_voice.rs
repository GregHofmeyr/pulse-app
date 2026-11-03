//! Badges (#49) from voice and streams, the client-reported events (muted talk, the logo egg) and
//! Early Adopter. Server voice only: DM and group calls never count.
mod common;

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use chrono::Utc;
use common::*;
use livekit_api::access_token::AccessToken;
use pulse_protocol::ids::{ChannelId, ServerId, UserId};
use pulse_protocol::rest::{Channel, Profile, StreamInfo};
use pulse_server::badges::catalogue::{HOUR, stat};
use pulse_server::badges::time::sast_day;
use pulse_server::badges::{stats, streams, sweep};
use pulse_server::db::stamp;
use serde_json::json;
use sha2::{Digest, Sha256};

async fn lounge(app: &TestApp, token: &str) -> Channel {
    let s = create_server(app, token, "Main").await;
    let chans: Vec<Channel> = get_json(app, token, &format!("/servers/{}/channels", s.id)).await;
    chans
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap()
}

async fn join_server(app: &TestApp, token: &str, server: ServerId) {
    let r = post_json(app, token, &format!("/servers/{server}/join"), json!({})).await;
    assert!(r.status().is_success(), "join {}", r.status());
}

fn signed(app: &TestApp, body: &str) -> String {
    let sum = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(body.as_bytes()));
    AccessToken::with_api_key(&app.cfg.livekit_key, &app.cfg.livekit_secret)
        .with_sha256(&sum)
        .to_jwt()
        .unwrap()
}

async fn webhook(app: &TestApp, body: &str) {
    let r = app
        .http
        .post(app.url("/livekit/webhook"))
        .header("Authorization", signed(app, body))
        .header("Content-Type", "application/webhook+json")
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}

/// `user` joins `room`'s call, as LiveKit reports it.
async fn vjoin(app: &TestApp, room: ChannelId, user: UserId) {
    let grant = grant_livekit(app, &room.to_string(), &user.to_string()).await;
    let body =
        json!({"event": "participant_joined", "id": "EV_test", "room": {"name": room.to_string()},
        "participant": {"identity": user.to_string(), "attributes": {"pulse_grant": grant}}})
        .to_string();
    webhook(app, &body).await;
    assert!(app.voice_members(room).contains(&user), "joined");
}

async fn vleave(app: &TestApp, room: ChannelId, user: UserId) {
    let body =
        json!({"event": "participant_left", "id": "EV_test", "room": {"name": room.to_string()},
        "participant": {"identity": user.to_string()}})
        .to_string();
    webhook(app, &body).await;
}

/// Move `user`'s open voice session's start `secs` into the past.
async fn backdate(app: &TestApp, user: UserId, secs: i64) {
    let n = sqlx::query(
        "UPDATE voice_sessions SET joined_at = ? WHERE user_id = ? AND left_at IS NULL",
    )
    .bind(stamp(Utc::now() - chrono::Duration::seconds(secs)))
    .bind(user.to_string())
    .execute(&app.db)
    .await
    .unwrap()
    .rows_affected();
    assert_eq!(n, 1, "one open session to backdate");
}

async fn stat_of(app: &TestApp, user: UserId, key: &str) -> i64 {
    let mut c = app.db.acquire().await.unwrap();
    stats::get(&mut c, user, key).await.unwrap()
}

async fn set_stat(app: &TestApp, user: UserId, key: &str, value: i64) {
    let mut c = app.db.acquire().await.unwrap();
    stats::set(&mut c, user, key, value).await.unwrap();
}

async fn holds(app: &TestApp, token: &str, user: UserId, key: &str) -> bool {
    get_json::<Profile>(app, token, &format!("/users/{user}/profile"))
        .await
        .badges
        .iter()
        .any(|b| b.key == key)
}

fn around(secs: i64, want: i64) -> bool {
    (want - 1..=want + 1).contains(&secs)
}

#[tokio::test]
async fn voice_hours_count_only_with_company() {
    let app = spawn().await;
    let (alex, a) = register(&app, "alex").await;
    let (sam, b) = register(&app, "sam").await;
    let l = lounge(&app, &a).await;
    join_server(&app, &b, l.server_id.unwrap()).await;
    // a minute alone adds nothing
    vjoin(&app, l.id, alex).await;
    backdate(&app, alex, 60).await;
    vleave(&app, l.id, alex).await;
    assert_eq!(stat_of(&app, alex, stat::VOICE_SECS).await, 0);
    // alex was alone for a minute before sam came, then they were together for a minute
    vjoin(&app, l.id, alex).await;
    vjoin(&app, l.id, sam).await;
    backdate(&app, alex, 120).await;
    backdate(&app, sam, 60).await;
    vleave(&app, l.id, alex).await;
    let secs = stat_of(&app, alex, stat::VOICE_SECS).await;
    assert!(around(secs, 60), "alex {secs}");
    // sam's minute was all with alex
    vleave(&app, l.id, sam).await;
    let secs = stat_of(&app, sam, stat::VOICE_SECS).await;
    assert!(around(secs, 60), "sam {secs}");
    assert_eq!(stat_of(&app, alex, stat::GHOST).await, 0);
    assert_eq!(stat_of(&app, alex, stat::NIGHT_OWL).await, 0);
}

#[tokio::test]
async fn a_long_night_with_company_is_night_owl_and_the_regular() {
    let app = spawn().await;
    let (alex, a) = register(&app, "alex").await;
    let (sam, b) = register(&app, "sam").await;
    let l = lounge(&app, &a).await;
    join_server(&app, &b, l.server_id.unwrap()).await;
    vjoin(&app, l.id, alex).await;
    vjoin(&app, l.id, sam).await;
    // 25 hours together holds a 03:00 South African time, whenever the test runs
    backdate(&app, alex, 25 * HOUR).await;
    backdate(&app, sam, 25 * HOUR).await;
    vleave(&app, l.id, alex).await;
    assert_eq!(stat_of(&app, alex, stat::NIGHT_OWL).await, 1);
    assert!(around(
        stat_of(&app, alex, stat::VOICE_SECS).await,
        25 * HOUR
    ));
    assert!(holds(&app, &a, alex, "night_owl").await);
    assert!(holds(&app, &a, alex, "the_regular").await);
}

#[tokio::test]
async fn group_calls_never_count() {
    let app = spawn().await;
    let (alex, a) = register(&app, "alex").await;
    let (sam, _) = register(&app, "sam").await;
    let (kim, _) = register(&app, "kim").await;
    let dm = create_dm(&app, &a, &[sam]).await;
    let group = create_dm(&app, &a, &[sam, kim]).await;
    for ch in [dm.id, group.id] {
        vjoin(&app, ch, alex).await;
        vjoin(&app, ch, sam).await;
        vleave(&app, ch, alex).await;
        vleave(&app, ch, sam).await;
    }
    for who in [alex, sam] {
        for key in [stat::VOICE_SECS, stat::GHOST, stat::EARLY_BIRD_DAYS] {
            assert_eq!(stat_of(&app, who, key).await, 0, "{key}");
        }
    }
}

#[tokio::test]
async fn ghost_at_under_five_seconds() {
    let app = spawn().await;
    let (alex, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    set_stat(&app, alex, stat::GHOST, 9).await;
    // ten seconds isn't a ghost
    vjoin(&app, l.id, alex).await;
    backdate(&app, alex, 10).await;
    vleave(&app, l.id, alex).await;
    assert_eq!(stat_of(&app, alex, stat::GHOST).await, 9);
    assert!(!holds(&app, &a, alex, "ghost").await);
    // in and straight out is
    vjoin(&app, l.id, alex).await;
    vleave(&app, l.id, alex).await;
    assert_eq!(stat_of(&app, alex, stat::GHOST).await, 10);
    assert!(holds(&app, &a, alex, "ghost").await);
}

#[tokio::test]
async fn early_bird_first_in_the_server_today() {
    let app = spawn().await;
    let (alex, a) = register(&app, "alex").await;
    let (sam, b) = register(&app, "sam").await;
    let l = lounge(&app, &a).await;
    join_server(&app, &b, l.server_id.unwrap()).await;
    set_stat(&app, alex, stat::EARLY_BIRD_DAYS, 9).await;
    vjoin(&app, l.id, alex).await;
    assert_eq!(stat_of(&app, alex, stat::EARLY_BIRD_DAYS).await, 10);
    assert_eq!(
        stat_of(&app, alex, stat::EARLY_BIRD_LAST_DAY).await,
        sast_day(Utc::now())
    );
    assert!(holds(&app, &a, alex, "early_bird").await);
    // second in isn't first
    vjoin(&app, l.id, sam).await;
    assert_eq!(stat_of(&app, sam, stat::EARLY_BIRD_DAYS).await, 0);
    // nor is the first back again the same day
    vleave(&app, l.id, alex).await;
    vleave(&app, l.id, sam).await;
    vjoin(&app, l.id, alex).await;
    assert_eq!(stat_of(&app, alex, stat::EARLY_BIRD_DAYS).await, 10);
}

async fn open_streams(app: &TestApp) -> Vec<(String, Option<String>)> {
    sqlx::query_as("SELECT started_at, ended_at FROM stream_sessions ORDER BY started_at")
        .fetch_all(&app.db)
        .await
        .unwrap()
}

async fn go_live(app: &TestApp, token: &str, ch: ChannelId) -> StreamInfo {
    let r = post_json(
        app,
        token,
        &format!("/streams/{ch}/live"),
        json!({"settings": {"height": 1080, "fps": 60, "audio": true}}),
    )
    .await;
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

async fn stop(app: &TestApp, token: &str, ch: ChannelId) {
    let r = app
        .http
        .delete(app.url(&format!("/streams/{ch}/live")))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
}

#[tokio::test]
async fn stream_seconds_close_only_the_ended_stream() {
    let app = spawn().await;
    let (alex, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    vjoin(&app, l.id, alex).await;
    let first = go_live(&app, &a, l.id).await;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    stop(&app, &a, l.id).await;
    assert!(stat_of(&app, alex, stat::STREAM_SECS).await >= 1);
    // live again at once: a new row, open
    let second = go_live(&app, &a, l.id).await;
    let rows = open_streams(&app).await;
    assert_eq!(rows.len(), 2);
    assert!(rows[0].1.is_some(), "the stopped stream is closed");
    assert_eq!(rows[1], (second.started_at.clone(), None));
    // the first stream ending (again, late) never closes the second
    streams::ended(&app.state(), &first).await;
    assert_eq!(open_streams(&app).await[1].1, None);
    // leaving voice ends the second
    vleave(&app, l.id, alex).await;
    assert!(open_streams(&app).await.iter().all(|r| r.1.is_some()));
}

#[tokio::test]
async fn streams_left_open_by_a_previous_run_close_at_zero() {
    let app = spawn().await;
    let (alex, a) = register(&app, "alex").await;
    let l = lounge(&app, &a).await;
    vjoin(&app, l.id, alex).await;
    let info = go_live(&app, &a, l.id).await;
    streams::reconcile_on_startup(&app.db).await.unwrap();
    assert_eq!(
        open_streams(&app).await,
        vec![(info.started_at.clone(), Some(info.started_at))]
    );
}

#[tokio::test]
async fn deleting_a_channel_mid_call_still_counts_its_voice_and_stream() {
    let app = spawn().await;
    let (alex, a) = register(&app, "alex").await;
    let (sam, b) = register(&app, "sam").await;
    let l = lounge(&app, &a).await;
    join_server(&app, &b, l.server_id.unwrap()).await;
    vjoin(&app, l.id, alex).await;
    vjoin(&app, l.id, sam).await;
    backdate(&app, alex, 60).await;
    backdate(&app, sam, 60).await;
    go_live(&app, &a, l.id).await;
    let r = app
        .http
        .delete(app.url(&format!("/channels/{}", l.id)))
        .bearer_auth(&a)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    for who in [alex, sam] {
        let secs = stat_of(&app, who, stat::VOICE_SECS).await;
        assert!(around(secs, 60), "{secs}");
    }
    assert!(open_streams(&app).await.iter().all(|r| r.1.is_some()));
}

async fn muted_talk(app: &TestApp, token: &str) {
    let r = post_json(app, token, "/me/badges/muted-talk", json!({})).await;
    assert_eq!(r.status(), 204);
}

#[tokio::test]
async fn muted_talk_counts_in_server_voice_only_and_is_rate_limited() {
    let app = spawn().await;
    let (alex, a) = register(&app, "alex").await;
    let (sam, _) = register(&app, "sam").await;
    let l = lounge(&app, &a).await;
    // not in voice
    muted_talk(&app, &a).await;
    assert_eq!(stat_of(&app, alex, stat::DOOS).await, 0);
    // in a DM call
    let dm = create_dm(&app, &a, &[sam]).await;
    vjoin(&app, dm.id, alex).await;
    muted_talk(&app, &a).await;
    assert_eq!(stat_of(&app, alex, stat::DOOS).await, 0);
    vleave(&app, dm.id, alex).await;
    // in server voice
    vjoin(&app, l.id, alex).await;
    muted_talk(&app, &a).await;
    assert_eq!(stat_of(&app, alex, stat::DOOS).await, 1);
    // again straight away doesn't count
    muted_talk(&app, &a).await;
    assert_eq!(stat_of(&app, alex, stat::DOOS).await, 1);
    // the tenth counted is The Doos
    set_stat(&app, alex, stat::DOOS, 9).await;
    set_stat(&app, alex, stat::DOOS_LAST_AT, 0).await;
    muted_talk(&app, &a).await;
    assert!(holds(&app, &a, alex, "the_doos").await);
}

#[tokio::test]
async fn egg_logo_awards_hidden_badge() {
    let app = spawn().await;
    let (alex, a) = register(&app, "alex").await;
    let r = post_json(&app, &a, "/me/badges/egg", json!({"egg": "nope"})).await;
    assert_eq!(r.status(), 400);
    assert!(!holds(&app, &a, alex, "logo_egg").await);
    let r = post_json(&app, &a, "/me/badges/egg", json!({"egg": "logo"})).await;
    assert_eq!(r.status(), 204);
    assert_eq!(stat_of(&app, alex, stat::LOGO_EGG).await, 1);
    assert!(holds(&app, &a, alex, "logo_egg").await);
    // finding it again is fine
    let r = post_json(&app, &a, "/me/badges/egg", json!({"egg": "logo"})).await;
    assert_eq!(r.status(), 204);
}

#[tokio::test]
async fn early_adopter_needs_a_launch_date() {
    let app = spawn().await;
    let (alex, a) = register(&app, "alex").await;
    // no launch date: nobody, sweep or not
    assert!(
        sweep::run(&app.state(), Utc::now())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(!holds(&app, &a, alex, "early_adopter").await);

    let with_launch = |days_ago: i64| {
        let mut st = app.state();
        let today = (Utc::now() + chrono::Duration::hours(2)).date_naive();
        st.cfg = Arc::new(pulse_server::config::Config {
            launch_date: Some(today - chrono::Duration::days(days_ago)),
            ..(*app.cfg).clone()
        });
        st
    };
    // a launch eight days before alex joined: too late for alex
    assert!(
        sweep::run(&with_launch(8), Utc::now())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(!holds(&app, &a, alex, "early_adopter").await);
    // a launch tomorrow hasn't happened yet
    assert!(
        sweep::run(&with_launch(-1), Utc::now())
            .await
            .unwrap()
            .is_empty()
    );
    // launched today: alex joined in the first week
    assert_eq!(
        sweep::run(&with_launch(0), Utc::now()).await.unwrap(),
        vec![alex]
    );
    assert!(holds(&app, &a, alex, "early_adopter").await);
    // once only
    assert!(
        sweep::run(&with_launch(0), Utc::now())
            .await
            .unwrap()
            .is_empty()
    );
}
