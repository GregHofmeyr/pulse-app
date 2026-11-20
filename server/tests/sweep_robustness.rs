//! One bad row doesn't stall a sweep (reminders, scheduled messages), and a failed session-expiry slide doesn't fail
//! the request it rode on.
mod common;

use common::*;
use jiff::{SignedDuration, Timestamp};
use serde_json::json;

#[tokio::test]
async fn a_reminder_that_keeps_failing_is_parked_and_the_rest_still_go() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, _sam) = register(&app, "sam").await;
    let dm = create_dm(&app, &alex, &[sam_id]).await.id;
    let m = send(&app, &alex, dm, "remember this").await;
    let r = post_json(
        &app,
        &alex,
        &format!("/messages/{}/reminders", m.id),
        json!({"remind_at": pulse_server::times::rfc3339(Timestamp::now() + SignedDuration::from_mins(20))}),
    )
    .await;
    assert!(r.status().is_success());
    // a broken row due before it, at the head of the queue
    sqlx::query(
        "INSERT INTO message_reminders (id, user_id, message_id, remind_at, created_at)
         VALUES ('not-an-id', ?, ?, ?, ?)",
    )
    .bind(alex_id.to_string())
    .bind(m.id.to_string())
    .bind(pulse_server::db::now())
    .bind(pulse_server::db::now())
    .execute(&app.db)
    .await
    .unwrap();

    let s = app.state();
    let later = Timestamp::now() + SignedDuration::from_hours(1);
    assert_eq!(
        pulse_server::reminders::remind_due(&s, later)
            .await
            .unwrap(),
        1,
        "the good one goes past the bad one"
    );

    let failures = || async {
        count(
            &app,
            "SELECT failures FROM message_reminders WHERE id = 'not-an-id'",
        )
        .await
    };
    assert_eq!(failures().await, 1);
    for _ in 1..pulse_server::reminders::MAX_FAILURES {
        pulse_server::reminders::remind_due(&s, later)
            .await
            .unwrap();
    }
    assert_eq!(failures().await, pulse_server::reminders::MAX_FAILURES);
    // parked: not tried again
    pulse_server::reminders::remind_due(&s, later)
        .await
        .unwrap();
    assert_eq!(failures().await, pulse_server::reminders::MAX_FAILURES);
    // and gone with the fired ones after the inbox's days
    let much_later = Timestamp::now() + crate_days(40);
    pulse_server::reminders::remind_due(&s, much_later)
        .await
        .unwrap();
    assert_eq!(
        count(
            &app,
            "SELECT COUNT(*) FROM message_reminders WHERE id = 'not-an-id'"
        )
        .await,
        0
    );
}

fn crate_days(n: i64) -> SignedDuration {
    SignedDuration::from_hours(24 * n)
}

#[tokio::test]
async fn a_scheduled_message_row_that_doesn_t_parse_is_skipped() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, _sam) = register(&app, "sam").await;
    let dm = create_dm(&app, &alex, &[sam_id]).await.id;
    let r = post_json(
        &app,
        &alex,
        &format!("/channels/{dm}/scheduled"),
        json!({"content": "on time", "send_at": pulse_server::times::rfc3339(Timestamp::now() + SignedDuration::from_mins(20))}),
    )
    .await;
    assert!(r.status().is_success());
    sqlx::query(
        "INSERT INTO scheduled_messages (id, author_id, channel_id, content, send_at, created_at)
         VALUES ('not-an-id', ?, ?, 'broken', '2000-01-01T00:00:00.000Z', '2000-01-01T00:00:00.000Z')",
    )
    .bind(alex_id.to_string())
    .bind(dm.to_string())
    .execute(&app.db)
    .await
    .unwrap();
    let s = app.state();
    let later = Timestamp::now() + SignedDuration::from_hours(1);
    assert_eq!(
        pulse_server::scheduled::send_due(&s, later).await.unwrap(),
        1
    );
}

#[tokio::test]
async fn a_failed_expiry_slide_still_lets_the_session_in() {
    let app = spawn().await;
    let (_alex_id, alex) = register(&app, "alex").await;
    // due a slide, and the write refused
    sqlx::query("UPDATE sessions SET last_used_at = '2000-01-01T00:00:00.000Z'")
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER no_slide BEFORE UPDATE OF last_used_at ON sessions
         BEGIN SELECT RAISE(ABORT, 'busy'); END",
    )
    .execute(&app.db)
    .await
    .unwrap();
    let r = app
        .http
        .get(app.url("/me"))
        .bearer_auth(&alex)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}
