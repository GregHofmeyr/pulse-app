//! Scheduled messages (#299) and message reminders (#300): made, listed and changed only by their owner, posted or
//! sent by the sweep at their time from the database (once), with a scheduled message's checks run again then (a
//! failed one is kept, and its author told), and a due reminder in its person's inbox until seen.
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use jiff::{SignedDuration, Timestamp};
use pulse_protocol::gateway::{ClientFrame, ServerFrame};
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{
    Channel, InboxPage, Message, MessageReminder, ScheduledMessage, schedule_limits,
};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message as Ws;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn hello(app: &TestApp, token: &str) -> (Socket, Value) {
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
            let v: Value = serde_json::from_str(&t).unwrap();
            return (ws, v["d"].clone());
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

fn at(from_now: SignedDuration) -> String {
    pulse_server::times::rfc3339(Timestamp::now() + from_now)
}

async fn req(
    app: &TestApp,
    method: reqwest::Method,
    token: &str,
    path: &str,
    body: Option<Value>,
) -> reqwest::Response {
    let mut r = app.http.request(method, app.url(path)).bearer_auth(token);
    if let Some(b) = body {
        r = r.json(&b);
    }
    r.send().await.unwrap()
}

async fn schedule(
    app: &TestApp,
    token: &str,
    channel: ChannelId,
    content: &str,
    send_at: &str,
) -> reqwest::Response {
    post_json(
        app,
        token,
        &format!("/channels/{channel}/scheduled"),
        json!({"content": content, "send_at": send_at}),
    )
    .await
}

async fn schedule_ok(
    app: &TestApp,
    token: &str,
    channel: ChannelId,
    content: &str,
    send_at: &str,
) -> ScheduledMessage {
    let r = schedule(app, token, channel, content, send_at).await;
    assert_eq!(r.status(), 201);
    r.json().await.unwrap()
}

/// alex owns a server sam and kim are in; jo isn't in it. Returns (app, tokens [alex, sam, kim, jo], ids, #general).
async fn setup() -> (TestApp, [String; 4], [UserId; 4], Channel) {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (kim_id, kim) = register(&app, "kim").await;
    let (jo_id, jo) = register(&app, "jo").await;
    let s = create_server(&app, &alex, "Main").await;
    for t in [&sam, &kim] {
        let r = join_server(&app, t, s.id).await;
        assert!(r.status().is_success());
    }
    let g = general(&app, &alex, s.id).await;
    (
        app,
        [alex, sam, kim, jo],
        [alex_id, sam_id, kim_id, jo_id],
        g,
    )
}

async fn history(app: &TestApp, token: &str, channel: ChannelId) -> Vec<Message> {
    get_json(app, token, &format!("/channels/{channel}/messages")).await
}

#[tokio::test]
async fn a_scheduled_message_is_posted_once_at_its_time_as_its_author() {
    let (app, [alex, sam, ..], [alex_id, sam_id, ..], g) = setup().await;
    let (mut ws, ready) = hello(&app, &alex).await;
    assert_eq!(ready["scheduled"], json!([]));
    let made = schedule_ok(
        &app,
        &alex,
        g.id,
        "  hi @sam :wave: ",
        &at(SignedDuration::from_hours(1)),
    )
    .await;
    assert_eq!(made.content, "hi @sam :wave:");
    assert!(made.failed.is_none());
    let ev = next_raw(&mut ws, |d| d["t"] == "ScheduledSaved")
        .await
        .unwrap();
    assert_eq!(ev["d"]["scheduled"]["id"], made.id.to_string());
    // only alex sees it
    let mine: Vec<ScheduledMessage> = get_json(&app, &alex, "/scheduled").await;
    assert_eq!(mine, vec![made.clone()]);
    let theirs: Vec<ScheduledMessage> = get_json(&app, &sam, "/scheduled").await;
    assert!(theirs.is_empty());
    let (_ws2, ready) = hello(&app, &alex).await;
    assert_eq!(ready["scheduled"][0]["id"], made.id.to_string());

    let s = app.state();
    // not yet
    assert_eq!(
        pulse_server::scheduled::send_due(&s, Timestamp::now())
            .await
            .unwrap(),
        0
    );
    assert!(
        !history(&app, &sam, g.id)
            .await
            .iter()
            .any(|m| m.content.contains("hi"))
    );
    // its time: posted as alex, mentions resolved then
    let later = Timestamp::now() + SignedDuration::from_hours(2);
    assert_eq!(
        pulse_server::scheduled::send_due(&s, later).await.unwrap(),
        1
    );
    let posted: Vec<Message> = history(&app, &sam, g.id)
        .await
        .into_iter()
        .filter(|m| m.content == "hi @sam :wave:")
        .collect();
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].author_id, Some(alex_id));
    assert_eq!(posted[0].mentions, vec![sam_id]);
    let ev = next_raw(&mut ws, |d| d["t"] == "ScheduledRemoved")
        .await
        .unwrap();
    assert_eq!(ev["d"]["sent"], posted[0].id.to_string());
    // once
    assert_eq!(
        pulse_server::scheduled::send_due(&s, later).await.unwrap(),
        0
    );
    let mine: Vec<ScheduledMessage> = get_json(&app, &alex, "/scheduled").await;
    assert!(mine.is_empty());
}

#[tokio::test]
async fn a_scheduled_message_is_checked_again_and_kept_as_failed_when_it_cant_go() {
    let (app, [alex, _sam, kim, _], [_, sam_id, kim_id, _], _) = setup().await;
    let group = create_dm(&app, &alex, &[sam_id, kim_id]).await;
    let made = schedule_ok(
        &app,
        &kim,
        group.id,
        "see you all",
        &at(SignedDuration::from_mins(30)),
    )
    .await;
    let (mut ws, _) = hello(&app, &kim).await;
    // kim leaves the group before it goes
    let r = req(
        &app,
        reqwest::Method::DELETE,
        &kim,
        &format!("/channels/{}/members/{kim_id}", group.id),
        None,
    )
    .await;
    assert!(r.status().is_success());
    let s = app.state();
    let later = Timestamp::now() + SignedDuration::from_hours(1);
    assert_eq!(
        pulse_server::scheduled::send_due(&s, later).await.unwrap(),
        0
    );
    assert!(
        !history(&app, &alex, group.id)
            .await
            .iter()
            .any(|m| m.content == "see you all")
    );
    // kim is told, and it stays (failed) in their list until they delete it
    let ev = next_raw(&mut ws, |d| d["t"] == "ScheduledSaved")
        .await
        .unwrap();
    assert!(ev["d"]["scheduled"]["failed"].is_string());
    let mine: Vec<ScheduledMessage> = get_json(&app, &kim, "/scheduled").await;
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].id, made.id);
    assert!(mine[0].failed.is_some());
    // never tried again
    assert_eq!(
        pulse_server::scheduled::send_due(&s, later).await.unwrap(),
        0
    );
    let r = req(
        &app,
        reqwest::Method::DELETE,
        &kim,
        &format!("/scheduled/{}", made.id),
        None,
    )
    .await;
    assert_eq!(r.status(), 204);
    let mine: Vec<ScheduledMessage> = get_json(&app, &kim, "/scheduled").await;
    assert!(mine.is_empty());
}

#[tokio::test]
async fn only_its_author_edits_sends_now_or_deletes_a_scheduled_message() {
    let (app, [alex, sam, _, jo], _, g) = setup().await;
    let made = schedule_ok(
        &app,
        &alex,
        g.id,
        "draft",
        &at(SignedDuration::from_hours(3)),
    )
    .await;
    let path = format!("/scheduled/{}", made.id);
    // someone else's is never there
    for (m, p) in [
        (reqwest::Method::PATCH, path.clone()),
        (reqwest::Method::DELETE, path.clone()),
        (reqwest::Method::POST, format!("{path}/send")),
    ] {
        let r = req(&app, m, &sam, &p, Some(json!({"content": "x"}))).await;
        assert_eq!(r.status(), 404, "{p}");
    }
    // a new text and time
    let r = req(
        &app,
        reqwest::Method::PATCH,
        &alex,
        &path,
        Some(json!({"content": "final", "send_at": at(SignedDuration::from_hours(5))})),
    )
    .await;
    assert_eq!(r.status(), 200);
    let edited: ScheduledMessage = r.json().await.unwrap();
    assert_eq!(edited.content, "final");
    assert!(edited.edited_at.is_some() && edited.send_at > made.send_at);
    // send now
    let r = req(
        &app,
        reqwest::Method::POST,
        &alex,
        &format!("{path}/send"),
        None,
    )
    .await;
    assert_eq!(r.status(), 200);
    let sent: Message = r.json().await.unwrap();
    assert_eq!(sent.content, "final");
    assert!(
        history(&app, &sam, g.id)
            .await
            .iter()
            .any(|m| m.id == sent.id)
    );
    let r = req(&app, reqwest::Method::DELETE, &alex, &path, None).await;
    assert_eq!(r.status(), 404, "sent: gone");
    // somewhere jo can't post, a time gone or too far, nothing to say: refused
    assert_eq!(
        schedule(&app, &jo, g.id, "hi", &at(SignedDuration::from_hours(1)))
            .await
            .status(),
        404 // #80: a server you aren't in is hidden
    );
    for when in [
        at(SignedDuration::from_hours(-1)),
        at(pulse_server::times::days(400)),
        "soon".into(),
    ] {
        assert_eq!(schedule(&app, &alex, g.id, "hi", &when).await.status(), 400);
    }
    assert_eq!(
        schedule(&app, &alex, g.id, "  ", &at(SignedDuration::from_hours(1)))
            .await
            .status(),
        400
    );
}

#[tokio::test]
async fn a_person_has_at_most_so_many_scheduled_messages() {
    let (app, [alex, ..], _, g) = setup().await;
    let when = at(SignedDuration::from_hours(1));
    for i in 0..schedule_limits::MAX_SCHEDULED {
        schedule_ok(&app, &alex, g.id, &format!("m{i}"), &when).await;
    }
    let r = schedule(&app, &alex, g.id, "one more", &when).await;
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn a_reminder_lands_in_its_persons_inbox_once_at_its_time() {
    let (app, [alex, sam, _, jo], [alex_id, ..], g) = setup().await;
    let m = send(&app, &sam, g.id, "the plan for friday").await;
    let (mut ws, _) = hello(&app, &alex).await;
    let r = post_json(
        &app,
        &alex,
        &format!("/messages/{}/reminders", m.id),
        json!({"remind_at": at(SignedDuration::from_mins(20))}),
    )
    .await;
    assert_eq!(r.status(), 201);
    let made: MessageReminder = r.json().await.unwrap();
    assert!(!made.due && !made.seen);
    assert_eq!(made.message.content, "the plan for friday");
    assert_eq!(made.channel_id, g.id);
    next_raw(&mut ws, |d| d["t"] == "ReminderSaved")
        .await
        .unwrap();
    // private: sam sees none
    let theirs: Vec<MessageReminder> = get_json(&app, &sam, "/reminders").await;
    assert!(theirs.is_empty());
    // a message you can't see can't be a reminder
    let dm = create_dm(&app, &sam, &[alex_id]).await;
    let private = send(&app, &sam, dm.id, "just us").await;
    let r = post_json(
        &app,
        &jo,
        &format!("/messages/{}/reminders", private.id),
        json!({"remind_at": at(SignedDuration::from_mins(20))}),
    )
    .await;
    assert_eq!(r.status(), 404);

    let s = app.state();
    assert_eq!(
        pulse_server::reminders::remind_due(&s, Timestamp::now())
            .await
            .unwrap(),
        0
    );
    let later = Timestamp::now() + SignedDuration::from_hours(1);
    assert_eq!(
        pulse_server::reminders::remind_due(&s, later)
            .await
            .unwrap(),
        1
    );
    let ev = next_raw(&mut ws, |d| d["t"] == "ReminderSaved")
        .await
        .unwrap();
    assert_eq!(ev["d"]["reminder"]["due"], true);
    let ev = next_raw(&mut ws, |d| d["t"] == "InboxUpdated")
        .await
        .unwrap();
    assert_eq!(ev["d"]["unread"], 1);
    let page: InboxPage = get_json(&app, &alex, "/inbox").await;
    assert_eq!(page.unread, 1);
    // alex was connected, so it was told live: not missed (#412), and nothing to claim
    let mine: Vec<MessageReminder> = get_json(&app, &alex, "/reminders").await;
    assert!(!mine[0].missed);
    let claimed: bool = post_json(
        &app,
        &alex,
        &format!("/reminders/{}/alerted", made.id),
        json!({}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert!(!claimed);
    // once
    assert_eq!(
        pulse_server::reminders::remind_due(&s, later)
            .await
            .unwrap(),
        0
    );
    // seen: read on every device, still listed until done
    let r = post_json(
        &app,
        &alex,
        &format!("/reminders/{}/seen", made.id),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 204);
    let ev = next_raw(&mut ws, |d| d["t"] == "InboxUpdated")
        .await
        .unwrap();
    assert_eq!(ev["d"]["unread"], 0);
    let mine: Vec<MessageReminder> = get_json(&app, &alex, "/reminders").await;
    assert!(mine[0].due && mine[0].seen);
    let r = req(
        &app,
        reqwest::Method::DELETE,
        &alex,
        &format!("/reminders/{}", made.id),
        None,
    )
    .await;
    assert_eq!(r.status(), 204);
    let mine: Vec<MessageReminder> = get_json(&app, &alex, "/reminders").await;
    assert!(mine.is_empty());
}

#[tokio::test]
async fn mark_all_read_reads_due_reminders_and_a_deleted_message_reminds_no_one() {
    let (app, [alex, sam, ..], _, g) = setup().await;
    let a = send(&app, &sam, g.id, "first").await;
    let b = send(&app, &sam, g.id, "second").await;
    for m in [&a, &b] {
        let r = post_json(
            &app,
            &alex,
            &format!("/messages/{}/reminders", m.id),
            json!({"remind_at": at(SignedDuration::from_mins(5))}),
        )
        .await;
        assert_eq!(r.status(), 201);
    }
    // sam deletes the second before it's due
    let r = req(
        &app,
        reqwest::Method::DELETE,
        &sam,
        &format!("/messages/{}", b.id),
        None,
    )
    .await;
    assert!(r.status().is_success());
    let s = app.state();
    let later = Timestamp::now() + SignedDuration::from_hours(1);
    assert_eq!(
        pulse_server::reminders::remind_due(&s, later)
            .await
            .unwrap(),
        1
    );
    let mine: Vec<MessageReminder> = get_json(&app, &alex, "/reminders").await;
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].message_id, a.id);
    let page: InboxPage = get_json(&app, &alex, "/inbox").await;
    assert_eq!(page.unread, 1);
    let r = post_json(&app, &alex, "/inbox/read-all", json!({})).await;
    assert_eq!(r.status(), 204);
    let page: InboxPage = get_json(&app, &alex, "/inbox").await;
    assert_eq!(page.unread, 0);
    let mine: Vec<MessageReminder> = get_json(&app, &alex, "/reminders").await;
    assert!(mine[0].seen);
}

#[tokio::test]
async fn one_due_while_no_device_was_connected_is_told_once_by_the_first_to_claim_it() {
    let (app, [alex, sam, kim, _], _, g) = setup().await;
    let m = send(&app, &sam, g.id, "the plan for friday").await;
    let r = post_json(
        &app,
        &alex,
        &format!("/messages/{}/reminders", m.id),
        json!({"remind_at": at(SignedDuration::from_mins(5))}),
    )
    .await;
    let made: MessageReminder = r.json().await.unwrap();
    assert!(!made.missed);
    // alex has no device connected when it comes due
    let s = app.state();
    let later = Timestamp::now() + SignedDuration::from_hours(1);
    assert_eq!(
        pulse_server::reminders::remind_due(&s, later)
            .await
            .unwrap(),
        1
    );
    let (mut ws, ready) = hello(&app, &alex).await;
    assert_eq!(ready["reminders"][0]["missed"], true);
    let claim = |token: String| {
        let app = &app;
        let path = format!("/reminders/{}/alerted", made.id);
        async move {
            let r = post_json(app, &token, &path, json!({})).await;
            (r.status().as_u16(), r.json::<bool>().await.ok())
        }
    };
    // not kim's to claim
    assert_eq!(claim(kim.clone()).await.0, 404);
    // the first device to claim it tells alex; the next doesn't
    assert_eq!(claim(alex.clone()).await, (200, Some(true)));
    let ev = next_raw(&mut ws, |d| d["t"] == "ReminderSaved")
        .await
        .unwrap();
    assert_eq!(ev["d"]["reminder"]["due"], true);
    assert!(ev["d"]["reminder"].get("missed").is_none());
    assert_eq!(claim(alex.clone()).await, (200, Some(false)));
    let mine: Vec<MessageReminder> = get_json(&app, &alex, "/reminders").await;
    assert!(mine[0].due && !mine[0].missed && !mine[0].seen);
}

async fn block(app: &TestApp, token: &str, who: UserId) {
    let r = req(
        app,
        reqwest::Method::PUT,
        token,
        &format!("/me/blocks/{who}"),
        Some(json!({ "mode": "line" })),
    )
    .await;
    assert!(r.status().is_success(), "{}", r.status());
}

#[tokio::test]
async fn scheduled_messages_respect_blocks() {
    // #412 (re-check DM blocking, #474): a DM with someone you've blocked takes no scheduled message; blocking them
    // before one goes fails it; one to someone who has blocked you is posted withheld from them, never refused
    let (app, [alex, _, kim, _], [alex_id, sam_id, kim_id, _], _) = setup().await;
    let dm = create_dm(&app, &alex, &[sam_id]).await;
    let made = schedule_ok(
        &app,
        &alex,
        dm.id,
        "later, sam",
        &at(SignedDuration::from_mins(30)),
    )
    .await;
    block(&app, &alex, sam_id).await;
    let r = schedule(
        &app,
        &alex,
        dm.id,
        "another",
        &at(SignedDuration::from_mins(30)),
    )
    .await;
    assert_eq!(r.status(), 403);
    let s = app.state();
    let later = Timestamp::now() + SignedDuration::from_hours(1);
    assert_eq!(
        pulse_server::scheduled::send_due(&s, later).await.unwrap(),
        0
    );
    let mine: Vec<ScheduledMessage> = get_json(&app, &alex, "/scheduled").await;
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].id, made.id);
    assert!(mine[0].failed.as_deref().unwrap().contains("blocked"));

    // kim blocks alex: alex's scheduled message to kim still goes, and kim never sees it
    let dm2 = create_dm(&app, &alex, &[kim_id]).await;
    schedule_ok(
        &app,
        &alex,
        dm2.id,
        "hi kim",
        &at(SignedDuration::from_mins(30)),
    )
    .await;
    block(&app, &kim, alex_id).await;
    assert_eq!(
        pulse_server::scheduled::send_due(&s, later).await.unwrap(),
        1
    );
    assert!(
        history(&app, &alex, dm2.id)
            .await
            .iter()
            .any(|m| m.content == "hi kim")
    );
    assert!(
        !history(&app, &kim, dm2.id)
            .await
            .iter()
            .any(|m| m.content == "hi kim")
    );
}
