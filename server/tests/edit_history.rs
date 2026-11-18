//! Edit history (#301): an edit keeps the text it replaced, newest first after the current one, for anyone who can see
//! the message; capped; gone with the message's delete; never for someone who can't see it.
mod common;

use common::*;
use pulse_protocol::rest::{MAX_EDIT_VERSIONS, MessageVersion};
use serde_json::json;

async fn edit(app: &TestApp, token: &str, id: impl std::fmt::Display, content: &str) -> u16 {
    app.http
        .patch(app.url(&format!("/messages/{id}")))
        .bearer_auth(token)
        .json(&json!({ "content": content }))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn versions_status(app: &TestApp, token: &str, id: impl std::fmt::Display) -> u16 {
    app.http
        .get(app.url(&format!("/messages/{id}/edits")))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

#[tokio::test]
async fn edits_keep_earlier_versions_newest_first_for_anyone_who_can_see_it() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let s = create_server(&app, &alex, "Main").await;
    let r = join_server(&app, &sam, s.id).await;
    assert!(r.status().is_success());
    let g = general(&app, &alex, s.id).await;
    let m = send(&app, &alex, g.id, "first").await;

    // never edited: just the one version
    let v: Vec<MessageVersion> = get_json(&app, &sam, &format!("/messages/{}/edits", m.id)).await;
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].content, "first");
    assert_eq!(v[0].written_at, m.created_at);

    assert_eq!(edit(&app, &alex, m.id, "second").await, 200);
    // the same text again keeps nothing new
    assert_eq!(edit(&app, &alex, m.id, "second").await, 200);
    assert_eq!(edit(&app, &alex, m.id, "third").await, 200);
    let v: Vec<MessageVersion> = get_json(&app, &sam, &format!("/messages/{}/edits", m.id)).await;
    let texts: Vec<&str> = v.iter().map(|x| x.content.as_str()).collect();
    assert_eq!(texts, ["third", "second", "first"]);
    assert_eq!(v[2].written_at, m.created_at);
    assert!(v[0].written_at > v[1].written_at && v[1].written_at > v[2].written_at);
}

#[tokio::test]
async fn history_is_capped_private_to_the_chat_and_goes_with_the_message() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (_, kim) = register(&app, "kim").await;
    let dm = create_dm(&app, &alex, &[sam_id]).await;
    let m = send(&app, &alex, dm.id, "v0").await;
    for i in 1..=(MAX_EDIT_VERSIONS + 5) {
        assert_eq!(edit(&app, &alex, m.id, &format!("v{i}")).await, 200);
    }
    let v: Vec<MessageVersion> = get_json(&app, &sam, &format!("/messages/{}/edits", m.id)).await;
    // the current one and the newest MAX earlier ones
    assert_eq!(v.len() as i64, MAX_EDIT_VERSIONS + 1);
    assert_eq!(v[0].content, format!("v{}", MAX_EDIT_VERSIONS + 5));
    assert_eq!(v.last().unwrap().content, "v5");
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM message_edits WHERE message_id = ?")
        .bind(m.id.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(stored, MAX_EDIT_VERSIONS);

    // kim isn't in the DM: it doesn't exist for her
    assert_eq!(versions_status(&app, &kim, m.id).await, 404);

    // a delete takes the history with it
    let r = app
        .http
        .delete(app.url(&format!("/messages/{}", m.id)))
        .bearer_auth(&alex)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(versions_status(&app, &sam, m.id).await, 404);
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM message_edits WHERE message_id = ?")
        .bind(m.id.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(stored, 0);
}
