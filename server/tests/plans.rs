//! Plan something (#387): posting, answering (live over the gateway), its creator's edit and cancel, the reminder an
//! hour before (once, from the database, never for a cancelled plan), the calendar file and the cached forecast.
mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::{RawQuery, State};
use axum::routing::get;
use chrono::{TimeDelta, Utc};
use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, ServerFrame};
use pulse_protocol::ids::{ChannelId, MessageId, ServerId, UserId};
use pulse_protocol::rest::{Channel, Message, MessageKind, Plan, PlanForecast};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message as Ws;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

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
        let m = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Ws::Text(t) = m
            && let Ok(ServerFrame::Ready(_)) = serde_json::from_str(&t)
        {
            return ws;
        }
    }
}

/// The raw JSON of the next event `pick` takes (skipping others); None when none comes within 3 s.
async fn next_raw(ws: &mut Socket, pick: impl Fn(&Value) -> bool) -> Option<Value> {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .ok()??
            .ok()?;
        if let Ws::Text(t) = m {
            let v: Value = serde_json::from_str(&t).unwrap();
            if v["op"] == "Event" && pick(&v["d"]) {
                return Some(v["d"].clone());
            }
        }
    }
}

async fn put_json(app: &TestApp, token: &str, path: &str, body: Value) -> reqwest::Response {
    app.http
        .put(app.url(path))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

async fn join(app: &TestApp, token: &str, server: ServerId) {
    let r = post_json(app, token, &format!("/servers/{server}/join"), json!({})).await;
    assert!(r.status().is_success());
}

fn at(from_now: TimeDelta) -> String {
    (Utc::now() + from_now).to_rfc3339()
}

/// A day about two months ahead at 18:30 in UTC+2 (16:30 UTC): (as posted, as stored, as the fallback text says it).
fn later_day() -> (String, String, String) {
    let day = (Utc::now() + TimeDelta::days(60)).date_naive();
    (
        format!("{day}T18:30:00+02:00"),
        format!("{day}T16:30:00.000Z"),
        format!("{} 16:30 UTC", day.format("%a %-d %b %Y,")),
    )
}

fn body(title: &str, starts_at: &str) -> Value {
    json!({"title": title, "starts_at": starts_at})
}

async fn create(app: &TestApp, token: &str, channel: ChannelId, body: Value) -> reqwest::Response {
    post_json(app, token, &format!("/channels/{channel}/plans"), body).await
}

async fn create_ok(app: &TestApp, token: &str, channel: ChannelId, body: Value) -> Message {
    let r = create(app, token, channel, body).await;
    assert_eq!(r.status(), 201);
    r.json().await.unwrap()
}

async fn rsvp(app: &TestApp, token: &str, plan: MessageId, answer: Value) -> reqwest::Response {
    put_json(
        app,
        token,
        &format!("/messages/{plan}/plan/rsvp"),
        json!({ "rsvp": answer }),
    )
    .await
}

async fn rsvp_ok(app: &TestApp, token: &str, plan: MessageId, answer: Value) -> Plan {
    let r = rsvp(app, token, plan, answer).await;
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

/// alex owns a server sam and kim are in; jo isn't in it. Returns (app, tokens [alex, sam, kim, jo], ids, #general).
async fn setup() -> (TestApp, [String; 4], [UserId; 4], Channel) {
    setup_with(test_config()).await
}

async fn setup_with(
    cfg: pulse_server::config::Config,
) -> (TestApp, [String; 4], [UserId; 4], Channel) {
    let app = spawn_with(cfg).await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (kim_id, kim) = register(&app, "kim").await;
    let (jo_id, jo) = register(&app, "jo").await;
    let s = create_server(&app, &alex, "Main").await;
    join(&app, &sam, s.id).await;
    join(&app, &kim, s.id).await;
    let g = general(&app, &alex, s.id).await;
    (
        app,
        [alex, sam, kim, jo],
        [alex_id, sam_id, kim_id, jo_id],
        g,
    )
}

/// The chat's reminder lines, oldest first.
async fn reminders(app: &TestApp, token: &str, channel: ChannelId) -> Vec<Message> {
    let list: Vec<Message> = get_json(app, token, &format!("/channels/{channel}/messages")).await;
    let mut lines: Vec<Message> = list
        .into_iter()
        .filter(|m| m.plan_reminder.is_some())
        .collect();
    lines.sort_by_key(|m| m.id.to_string());
    lines
}

#[tokio::test]
async fn a_plan_is_a_message_with_its_text_for_older_clients() {
    let (app, [alex, sam, ..], _, g) = setup().await;
    let (posted, stored, said) = later_day();
    let m = create_ok(
        &app,
        &alex,
        g.id,
        json!({"title": "  Braai at kim's ", "starts_at": posted,
               "place": " Cape Town ", "note": "bring ice"}),
    )
    .await;
    assert_eq!(m.kind, MessageKind::Normal);
    assert_eq!(
        m.content,
        format!(
            "📅 Plan: Braai at kim's\n🕒 {said}\n📍 Cape Town\nbring ice\n(Update Pulse to RSVP)"
        )
    );
    let p = m.plan.as_deref().unwrap();
    assert_eq!(p.title, "Braai at kim's");
    assert_eq!(p.starts_at, stored, "stored in UTC");
    assert_eq!(p.place.as_deref(), Some("Cape Town"));
    assert!(p.going.is_empty() && p.cancelled_at.is_none());
    // history: the same card for everyone
    let list: Vec<Message> = get_json(&app, &sam, &format!("/channels/{}/messages", g.id)).await;
    assert_eq!(
        list.iter().find(|x| x.id == m.id).unwrap().plan.as_deref(),
        Some(p)
    );
    // and its text can't be edited like a message's
    let r = app
        .http
        .patch(app.url(&format!("/messages/{}", m.id)))
        .bearer_auth(&alex)
        .json(&json!({"content": "hi"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn posting_checks_the_plan_and_the_chat() {
    let (app, [alex, _, _, jo], _, g) = setup().await;
    for b in [
        body("", &at(TimeDelta::days(1))),
        body(&"t".repeat(101), &at(TimeDelta::days(1))),
        body("ok", "tomorrow"),
        body("ok", &at(-TimeDelta::hours(1))),
        body("ok", &at(TimeDelta::days(800))),
        json!({"title": "ok", "starts_at": at(TimeDelta::days(1)), "note": "n".repeat(501)}),
    ] {
        assert_eq!(
            create(&app, &alex, g.id, b.clone()).await.status(),
            400,
            "{b}"
        );
    }
    assert_eq!(
        create(&app, &jo, g.id, body("ok", &at(TimeDelta::days(1))))
            .await
            .status(),
        403
    );
    // not in a quotes channel
    let r = post_json(
        &app,
        &alex,
        &format!("/servers/{}/channels", g.server_id.unwrap()),
        json!({"kind": "quotes", "name": "quotes2"}),
    )
    .await;
    let q: Channel = r.json().await.unwrap();
    assert_eq!(
        create(&app, &alex, q.id, body("ok", &at(TimeDelta::days(1))))
            .await
            .status(),
        400
    );
}

#[tokio::test]
async fn answers_change_live_and_each_person_has_one() {
    let (app, [alex, sam, kim, jo], [_, sam_id, kim_id, _], g) = setup().await;
    let m = create_ok(
        &app,
        &alex,
        g.id,
        body("Games night", &at(TimeDelta::days(2))),
    )
    .await;
    let mut ws = hello(&app, &alex).await;

    let p = rsvp_ok(&app, &sam, m.id, json!("going")).await;
    assert_eq!(p.going, vec![sam_id]);
    let ev = next_raw(&mut ws, |d| d["t"] == "PlanChanged")
        .await
        .unwrap();
    assert_eq!(ev["d"]["message_id"], json!(m.id));
    assert_eq!(ev["d"]["plan"]["going"], json!([sam_id]));

    rsvp_ok(&app, &kim, m.id, json!("going")).await;
    // sam changes their mind: one answer each
    let p = rsvp_ok(&app, &sam, m.id, json!("maybe")).await;
    assert_eq!(
        (p.going.clone(), p.maybe.clone()),
        (vec![kim_id], vec![sam_id])
    );
    let p = rsvp_ok(&app, &kim, m.id, json!("cant")).await;
    assert_eq!((p.going.len(), p.cant.clone()), (0, vec![kim_id]));
    // taking it back
    let p = rsvp_ok(&app, &sam, m.id, Value::Null).await;
    assert!(p.maybe.is_empty());
    assert_eq!(rsvp(&app, &sam, m.id, json!("nope")).await.status(), 422);
    // jo isn't in the server
    assert_eq!(rsvp(&app, &jo, m.id, json!("going")).await.status(), 403);
    // the plan as fetched
    let p: Plan = get_json(&app, &sam, &format!("/messages/{}/plan", m.id)).await;
    assert_eq!(p.cant, vec![kim_id]);
}

#[tokio::test]
async fn only_its_creator_edits_or_cancels_and_a_cancelled_plan_takes_no_answers() {
    let (app, [alex, sam, ..], _, g) = setup().await;
    let m = create_ok(&app, &alex, g.id, body("Braai", &at(TimeDelta::days(2)))).await;
    let edit =
        json!({"title": "Braai (moved)", "starts_at": at(TimeDelta::days(3)), "place": "kim's"});
    let path = format!("/messages/{}/plan", m.id);
    assert_eq!(
        put_json(&app, &sam, &path, edit.clone()).await.status(),
        403
    );
    let r = put_json(&app, &alex, &path, edit).await;
    assert_eq!(r.status(), 200);
    let p: Plan = r.json().await.unwrap();
    assert_eq!(p.title, "Braai (moved)");
    assert_eq!(p.place.as_deref(), Some("kim's"));
    assert!(p.edited_at.is_some());
    // the fallback text follows
    let list: Vec<Message> = get_json(&app, &sam, &format!("/channels/{}/messages", g.id)).await;
    assert!(
        list.iter()
            .find(|x| x.id == m.id)
            .unwrap()
            .content
            .starts_with("📅 Plan: Braai (moved)")
    );

    let cancel = format!("/messages/{}/plan/cancel", m.id);
    assert_eq!(
        post_json(&app, &sam, &cancel, json!({})).await.status(),
        403
    );
    let mut ws = hello(&app, &sam).await;
    let r = post_json(&app, &alex, &cancel, json!({})).await;
    assert_eq!(r.status(), 200);
    let p: Plan = r.json().await.unwrap();
    assert!(p.cancelled_at.is_some());
    let ev = next_raw(&mut ws, |d| d["t"] == "PlanChanged")
        .await
        .unwrap();
    assert!(ev["d"]["plan"]["cancelled_at"].is_string());
    assert_eq!(rsvp(&app, &sam, m.id, json!("going")).await.status(), 409);
    assert_eq!(
        post_json(&app, &alex, &cancel, json!({})).await.status(),
        409
    );
    let again = json!({"title": "x", "starts_at": at(TimeDelta::days(3))});
    assert_eq!(put_json(&app, &alex, &path, again).await.status(), 409);
}

#[tokio::test]
async fn going_and_maybe_people_are_reminded_an_hour_before_once() {
    let (app, [alex, sam, kim, jo], [_, sam_id, kim_id, jo_id], g) = setup().await;
    // jo joins and answers, then is no longer in the server: not reminded
    join(&app, &jo, g.server_id.unwrap()).await;
    let m = create_ok(&app, &alex, g.id, body("Braai", &at(TimeDelta::hours(3)))).await;
    rsvp_ok(&app, &sam, m.id, json!("going")).await;
    rsvp_ok(&app, &kim, m.id, json!("maybe")).await;
    rsvp_ok(&app, &jo, m.id, json!("going")).await;
    // (there's no Leave server yet: as if jo had left)
    sqlx::query("DELETE FROM server_members WHERE user_id = ?")
        .bind(jo_id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    rsvp_ok(&app, &alex, m.id, json!("cant")).await;
    // sam has read the chat (the inbox lists mentions in chats you have a read point in)
    send(&app, &sam, g.id, "in!").await;

    let s = app.state();
    // not yet due
    assert_eq!(
        pulse_server::plans::remind_due(&s, Utc::now())
            .await
            .unwrap(),
        0
    );
    let mut ws = hello(&app, &sam).await;
    let later = Utc::now() + TimeDelta::hours(2) + TimeDelta::minutes(1);
    assert_eq!(pulse_server::plans::remind_due(&s, later).await.unwrap(), 1);
    // a second sweep (or a restart's boot sweep) finds it done: the database says so
    assert_eq!(pulse_server::plans::remind_due(&s, later).await.unwrap(), 0);
    let lines = reminders(&app, &alex, g.id).await;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].content, "⏰ Braai starts in an hour");
    assert_eq!(lines[0].kind, MessageKind::System);
    assert_eq!(lines[0].mentions, vec![sam_id, kim_id]);
    assert!(!lines[0].mentions.contains(&jo_id));
    assert_eq!(
        lines[0].plan_reminder.as_ref().unwrap().plan_message_id,
        m.id
    );
    let ev = next_raw(&mut ws, |d| {
        d["t"] == "MessageCreated" && d["d"]["message"]["plan_reminder"].is_object()
    })
    .await
    .unwrap();
    assert_eq!(ev["d"]["message"]["mentions"], json!([sam_id, kim_id]));
    // in sam's inbox, as a mention
    let inbox: Value = get_json(&app, &sam, "/inbox").await;
    assert!(
        inbox["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["message"]["id"] == json!(lines[0].id)),
        "{inbox}"
    );
}

#[tokio::test]
async fn a_cancelled_plan_is_never_reminded_and_a_new_time_sets_it_up_again() {
    let (app, [alex, sam, ..], _, g) = setup().await;
    let s = app.state();
    let cancelled = create_ok(&app, &alex, g.id, body("Off", &at(TimeDelta::hours(3)))).await;
    rsvp_ok(&app, &sam, cancelled.id, json!("going")).await;
    post_json(
        &app,
        &alex,
        &format!("/messages/{}/plan/cancel", cancelled.id),
        json!({}),
    )
    .await;

    let moved = create_ok(&app, &alex, g.id, body("Moved", &at(TimeDelta::hours(3)))).await;
    rsvp_ok(&app, &sam, moved.id, json!("going")).await;
    let soon = Utc::now() + TimeDelta::hours(2) + TimeDelta::minutes(1);
    assert_eq!(pulse_server::plans::remind_due(&s, soon).await.unwrap(), 1);
    // moved to tomorrow: reminded again before the new time
    let r = put_json(
        &app,
        &alex,
        &format!("/messages/{}/plan", moved.id),
        body("Moved", &at(TimeDelta::days(1))),
    )
    .await;
    assert_eq!(r.status(), 200);
    assert_eq!(pulse_server::plans::remind_due(&s, soon).await.unwrap(), 0);
    let tomorrow = Utc::now() + TimeDelta::hours(23) + TimeDelta::minutes(1);
    assert_eq!(
        pulse_server::plans::remind_due(&s, tomorrow).await.unwrap(),
        1
    );
    let lines = reminders(&app, &alex, g.id).await;
    assert_eq!(lines.len(), 2);
    assert!(
        lines
            .iter()
            .all(|l| l.plan_reminder.as_ref().unwrap().plan_message_id == moved.id)
    );

    // made less than an hour ahead: everyone just saw it, so no reminder
    let close = create_ok(
        &app,
        &alex,
        g.id,
        body("Now-ish", &at(TimeDelta::minutes(30))),
    )
    .await;
    rsvp_ok(&app, &sam, close.id, json!("going")).await;
    assert_eq!(
        pulse_server::plans::remind_due(&s, Utc::now())
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn a_deleted_plan_goes_with_its_message() {
    let (app, [alex, sam, ..], _, g) = setup().await;
    let m = create_ok(&app, &alex, g.id, body("Braai", &at(TimeDelta::hours(3)))).await;
    rsvp_ok(&app, &sam, m.id, json!("going")).await;
    let r = app
        .http
        .delete(app.url(&format!("/messages/{}", m.id)))
        .bearer_auth(&alex)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
    let left: i64 = sqlx::query_scalar(
        "SELECT (SELECT COUNT(*) FROM plans) + (SELECT COUNT(*) FROM plan_rsvps)",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(left, 0);
    let s = app.state();
    assert_eq!(
        pulse_server::plans::remind_due(&s, Utc::now() + TimeDelta::hours(3))
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn the_calendar_file_is_one_utc_event() {
    let (app, [alex, sam, _, jo], [_, sam_id, _, _], g) = setup().await;
    let (posted, stored, _) = later_day();
    let m = create_ok(
        &app,
        &alex,
        g.id,
        json!({"title": "Braai, games", "starts_at": posted, "place": "kim's; back yard"}),
    )
    .await;
    let r = app
        .http
        .get(app.url(&format!("/messages/{}/plan/ics", m.id)))
        .bearer_auth(&sam)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["content-type"], "text/calendar; charset=utf-8");
    assert!(
        r.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains("filename=\"Braai, games.ics\"")
    );
    let ics = r.text().await.unwrap();
    assert!(ics.contains(&format!("\r\nUID:{}@pulse\r\n", m.id)));
    let start = stored.replace(['-', ':'], "").replace(".000Z", "Z");
    assert!(ics.contains(&format!("\r\nDTSTART:{start}\r\n")), "{ics}");
    assert!(ics.contains("\r\nSUMMARY:Braai\\, games\r\n"));
    assert!(ics.contains("\r\nLOCATION:kim's\\; back yard\r\n"));
    assert!(ics.contains("\r\nDTSTAMP:"));
    // a plan in a DM: only its members get the file
    let dm = create_dm(&app, &alex, &[sam_id]).await;
    let m = create_ok(&app, &sam, dm.id, body("Coffee", &at(TimeDelta::days(1)))).await;
    let r = app
        .http
        .get(app.url(&format!("/messages/{}/plan/ics", m.id)))
        .bearer_auth(&jo)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404, "jo isn't in the DM");
}

#[tokio::test]
async fn an_end_time_is_kept_shown_and_given_to_calendars() {
    let (app, [alex, sam, _, _], _, g) = setup().await;
    let (posted, stored, _) = later_day();
    let day = &stored[..10];
    let ends = format!("{day}T21:00:00+02:00");
    let m = create_ok(
        &app,
        &alex,
        g.id,
        json!({"title": "Braai", "starts_at": posted, "ends_at": ends}),
    )
    .await;
    let plan = m.plan.as_ref().unwrap();
    assert_eq!(
        plan.ends_at.as_deref(),
        Some(&*format!("{day}T19:00:00.000Z"))
    );
    assert!(m.content.contains("16:30–19:00 UTC"), "{}", m.content);
    let read: Plan = get_json(&app, &sam, &format!("/messages/{}/plan", m.id)).await;
    assert_eq!(read.ends_at, plan.ends_at);
    let ics = app
        .http
        .get(app.url(&format!("/messages/{}/plan/ics", m.id)))
        .bearer_auth(&sam)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let compact = day.replace('-', "");
    assert!(
        ics.contains(&format!("\r\nDTEND:{compact}T190000Z\r\n")),
        "{ics}"
    );
    // an edit without one drops it (every field, as a new post): the calendar gets an hour
    let r = put_json(
        &app,
        &alex,
        &format!("/messages/{}/plan", m.id),
        body("Braai", &posted),
    )
    .await;
    assert_eq!(r.status(), 200);
    let edited: Plan = r.json().await.unwrap();
    assert_eq!(edited.ends_at, None);
    // an end before the start is refused
    let r = create(
        &app,
        &alex,
        g.id,
        json!({"title": "Braai", "starts_at": posted, "ends_at": format!("{day}T17:00:00+02:00")}),
    )
    .await;
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn people_who_turned_plan_reminders_off_are_not_pinged() {
    let (app, [alex, sam, kim, _], [_, sam_id, kim_id, _], g) = setup().await;
    let m = create_ok(&app, &alex, g.id, body("Braai", &at(TimeDelta::hours(3)))).await;
    rsvp_ok(&app, &sam, m.id, json!("going")).await;
    rsvp_ok(&app, &kim, m.id, json!("going")).await;
    // kim turns it off: her own sessions hear it, and her next Ready says so
    let mut ws = hello(&app, &kim).await;
    let r = put_json(&app, &kim, "/me/plan-reminders", json!({"on": false})).await;
    assert_eq!(r.status(), 204);
    let ev = next_raw(&mut ws, |d| d["t"] == "PlanRemindersChanged")
        .await
        .unwrap();
    assert_eq!(ev["d"], json!({"user_id": kim_id, "on": false}));
    assert!(
        !pulse_server::plans::routes::reminders_on(&app.db, kim_id)
            .await
            .unwrap()
    );
    assert!(
        pulse_server::plans::routes::reminders_on(&app.db, sam_id)
            .await
            .unwrap()
    );

    let s = app.state();
    let later = Utc::now() + TimeDelta::hours(2) + TimeDelta::minutes(1);
    assert_eq!(pulse_server::plans::remind_due(&s, later).await.unwrap(), 1);
    let lines = reminders(&app, &alex, g.id).await;
    assert_eq!(lines[0].mentions, vec![sam_id]);

    // with everyone off, the plan is marked done quietly: no line at all
    put_json(&app, &sam, "/me/plan-reminders", json!({"on": false})).await;
    let m = create_ok(&app, &alex, g.id, body("Darts", &at(TimeDelta::hours(3)))).await;
    rsvp_ok(&app, &sam, m.id, json!("going")).await;
    assert_eq!(pulse_server::plans::remind_due(&s, later).await.unwrap(), 0);
    assert_eq!(reminders(&app, &alex, g.id).await.len(), 1);
    // and back on
    let r = put_json(&app, &kim, "/me/plan-reminders", json!({"on": true})).await;
    assert_eq!(r.status(), 204);
    assert!(
        pulse_server::plans::routes::reminders_on(&app.db, kim_id)
            .await
            .unwrap()
    );
}

/// A stand-in for Open-Meteo: geocoding knows "Cape Town" only; the forecast covers the next 16 days from today.
/// Counts the calls of each.
#[derive(Clone, Default)]
struct FakeMeteo {
    geocode: Arc<AtomicUsize>,
    forecast: Arc<AtomicUsize>,
}

async fn fake_meteo() -> (FakeMeteo, String) {
    let calls = FakeMeteo::default();
    async fn geocode(State(c): State<FakeMeteo>, RawQuery(q): RawQuery) -> axum::Json<Value> {
        c.geocode.fetch_add(1, Ordering::SeqCst);
        let q = q.unwrap_or_default();
        if q.contains("name=cape+town") || q.contains("name=cape%20town") {
            axum::Json(
                json!({"results": [{"name": "Cape Town", "latitude": -33.92584, "longitude": 18.42322,
                                            "country": "South Africa"}]}),
            )
        } else {
            axum::Json(json!({"generationtime_ms": 0.1}))
        }
    }
    async fn forecast(State(c): State<FakeMeteo>, RawQuery(q): RawQuery) -> axum::Json<Value> {
        c.forecast.fetch_add(1, Ordering::SeqCst);
        let q = q.unwrap_or_default();
        assert!(
            q.contains("timezone=auto") && q.contains("forecast_days=16"),
            "{q}"
        );
        let today = (Utc::now() + TimeDelta::hours(2)).date_naive();
        let days: Vec<String> = (0..16)
            .map(|i| (today + TimeDelta::days(i)).to_string())
            .collect();
        axum::Json(json!({
            "utc_offset_seconds": 7200,
            "daily": {
                "time": days,
                "weather_code": vec![61; 16],
                "temperature_2m_max": vec![24.5; 16],
                "temperature_2m_min": vec![14.0; 16],
                "precipitation_probability_max": vec![70; 16],
            }
        }))
    }
    let router = Router::new()
        .route("/v1/search", get(geocode))
        .route("/v1/forecast", get(forecast))
        .with_state(calls.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (calls, base)
}

#[tokio::test]
async fn the_forecast_comes_through_the_server_and_is_cached() {
    let (meteo, base) = fake_meteo().await;
    let mut cfg = test_config();
    cfg.plans.geocode_base = base.clone();
    cfg.plans.forecast_base = base;
    let (app, [alex, sam, kim, _], _, g) = setup_with(cfg).await;
    let forecast = |token: String, id: MessageId| {
        let app = &app;
        async move {
            get_json::<Option<PlanForecast>>(app, &token, &format!("/messages/{id}/plan/forecast"))
                .await
        }
    };
    let m = create_ok(
        &app,
        &alex,
        g.id,
        json!({"title": "Braai", "starts_at": at(TimeDelta::days(3)), "place": "Cape Town"}),
    )
    .await;
    let f = forecast(sam.clone(), m.id).await.unwrap();
    assert_eq!(f.place, "Cape Town, South Africa");
    assert_eq!((f.code, f.rain_chance), (61, Some(70)));
    assert!((f.temp_max - 24.5).abs() < 0.01);
    // cached: nobody asks Open-Meteo again, and a second plan at the same place uses the same answers
    assert!(forecast(kim.clone(), m.id).await.is_some());
    let m2 = create_ok(
        &app,
        &alex,
        g.id,
        json!({"title": "Again", "starts_at": at(TimeDelta::days(5)), "place": " cape town "}),
    )
    .await;
    assert!(forecast(sam.clone(), m2.id).await.is_some());
    assert_eq!(meteo.geocode.load(Ordering::SeqCst), 1);
    assert_eq!(meteo.forecast.load(Ordering::SeqCst), 1);

    // a place it doesn't know: none, and not asked again
    let nowhere = create_ok(
        &app,
        &alex,
        g.id,
        json!({"title": "?", "starts_at": at(TimeDelta::days(3)), "place": "Nowhereville"}),
    )
    .await;
    assert_eq!(forecast(sam.clone(), nowhere.id).await, None);
    assert_eq!(forecast(sam.clone(), nowhere.id).await, None);
    assert_eq!(meteo.geocode.load(Ordering::SeqCst), 2);
    // beyond the forecast's reach, and no place: none, nothing asked
    let far = create_ok(
        &app,
        &alex,
        g.id,
        json!({"title": "Far", "starts_at": at(TimeDelta::days(40)), "place": "Cape Town"}),
    )
    .await;
    assert_eq!(forecast(sam.clone(), far.id).await, None);
    let bare = create_ok(&app, &alex, g.id, body("Bare", &at(TimeDelta::days(1)))).await;
    assert_eq!(forecast(sam.clone(), bare.id).await, None);
    assert_eq!(meteo.geocode.load(Ordering::SeqCst), 2);
    assert_eq!(meteo.forecast.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn no_forecast_when_open_meteo_is_down() {
    // test_config points Open-Meteo at a closed port
    let (app, [alex, sam, ..], _, g) = setup().await;
    let m = create_ok(
        &app,
        &alex,
        g.id,
        json!({"title": "Braai", "starts_at": at(TimeDelta::days(3)), "place": "Cape Town"}),
    )
    .await;
    let f: Option<PlanForecast> =
        get_json(&app, &sam, &format!("/messages/{}/plan/forecast", m.id)).await;
    assert_eq!(f, None);
    // nothing cached from a failure: the next ask tries again
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM plan_geocodes")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(n, 0);
}
