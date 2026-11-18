//! Forwarding a message (#302): to a chat you can post in, from one you can see, with an optional comment; the forward
//! says where the original was said (a server channel by name, never a DM), by whom and when, and keeps its words as
//! forwarded; forwarding a forward forwards its original; a deleted original leaves nothing in its forwards; files are
//! the original's, by reference; quotes and system lines can't be forwarded; it is rate limited.
mod common;

use common::*;
use pulse_protocol::rest::{ChannelKind, Message, SearchResults};
use serde_json::{Value, json};

async fn forward(
    app: &TestApp,
    token: &str,
    id: impl std::fmt::Display,
    body: Value,
) -> reqwest::Response {
    post_json(app, token, &format!("/messages/{id}/forward"), body).await
}

async fn search(
    app: &TestApp,
    token: &str,
    server: impl std::fmt::Display,
    q: &str,
) -> SearchResults {
    get_json(app, token, &format!("/servers/{server}/search?q={q}")).await
}

async fn history(app: &TestApp, token: &str, channel: impl std::fmt::Display) -> Vec<Message> {
    get_json(app, token, &format!("/channels/{channel}/messages")).await
}

#[tokio::test]
async fn a_forward_says_where_from_and_keeps_the_words_it_forwarded() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (kim_id, kim) = register(&app, "kim").await;
    let s = create_server(&app, &alex, "Main").await;
    let r = post_json(&app, &sam, &format!("/servers/{}/join", s.id), json!({})).await;
    assert!(r.status().is_success());
    let g = general(&app, &alex, s.id).await;
    let original = send(&app, &alex, g.id, "movie night at 8, ||it's a surprise||").await;
    let dm = create_dm(&app, &sam, &[kim_id]).await;

    // sam forwards it to kim, with a comment
    let r = forward(
        &app,
        &sam,
        original.id,
        json!({"channel_id": dm.id, "comment": "you in @kim?"}),
    )
    .await;
    assert_eq!(r.status(), 200);
    let fwd: Message = r.json().await.unwrap();
    assert_eq!(fwd.author_id, Some(sam_id));
    assert_eq!(fwd.channel_id, dm.id);
    assert_eq!(fwd.content, "you in @kim?");
    assert!(!fwd.caption_is_fallback);
    assert_eq!(fwd.mentions, vec![kim_id]);
    let f = fwd.forwarded.clone().unwrap();
    assert_eq!(f.message_id, original.id);
    assert_eq!(f.channel_id, g.id);
    assert_eq!(f.channel_kind, ChannelKind::Text);
    assert_eq!(f.channel_name.as_deref(), Some("general"));
    assert_eq!(f.author_id, Some(alex_id));
    assert_eq!(f.created_at, original.created_at);
    // the spoiler stays a spoiler
    assert_eq!(f.content, "movie night at 8, ||it's a surprise||");
    assert!(!f.unavailable);

    // kim sees it in the DM as sent
    let seen = history(&app, &kim, dm.id).await;
    let got = seen.iter().find(|m| m.id == fwd.id).unwrap();
    assert_eq!(got.forwarded.as_ref().unwrap().content, f.content);

    // alex edits the original: the forward keeps what was forwarded
    let r = app
        .http
        .patch(app.url(&format!("/messages/{}", original.id)))
        .bearer_auth(&alex)
        .json(&json!({"content": "movie night cancelled"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let seen = history(&app, &kim, dm.id).await;
    let got = seen.iter().find(|m| m.id == fwd.id).unwrap();
    assert_eq!(
        got.forwarded.as_ref().unwrap().content,
        "movie night at 8, ||it's a surprise||"
    );

    // no comment: a stand-in, marked so
    let r = forward(&app, &sam, original.id, json!({"channel_id": dm.id})).await;
    let bare: Message = r.json().await.unwrap();
    assert!(bare.caption_is_fallback);
    assert!(bare.mentions.is_empty());
    // its text names the original's first line as it is now (notifications, previews, older clients; #415), on
    // read too
    assert_eq!(bare.content, "↪ Forwarded: movie night cancelled");
    let seen = history(&app, &kim, dm.id).await;
    let got = seen.iter().find(|m| m.id == bare.id).unwrap();
    assert_eq!(got.content, bare.content);

    // a forward can't be edited
    let r = app
        .http
        .patch(app.url(&format!("/messages/{}", bare.id)))
        .bearer_auth(&sam)
        .json(&json!({"content": "changed"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);

    // kim forwards the forward back to the server: it points at the original, not at sam's DM
    let r = post_json(&app, &kim, &format!("/servers/{}/join", s.id), json!({})).await;
    assert!(r.status().is_success());
    let r = forward(&app, &kim, fwd.id, json!({"channel_id": g.id})).await;
    assert_eq!(r.status(), 200);
    let again: Message = r.json().await.unwrap();
    let a = again.forwarded.unwrap();
    assert_eq!(a.message_id, original.id);
    assert_eq!(a.channel_id, g.id);
    assert_eq!(a.content, "movie night at 8, ||it's a surprise||");
}

#[tokio::test]
async fn a_dm_is_never_named_and_a_deleted_original_leaves_nothing() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let s = create_server(&app, &sam, "Main").await;
    let g = general(&app, &sam, s.id).await;
    let dm = create_dm(&app, &alex, &[sam_id]).await;
    let original = send(&app, &alex, dm.id, "my new number is 555").await;
    let r = forward(&app, &sam, original.id, json!({"channel_id": g.id})).await;
    assert_eq!(r.status(), 200);
    let fwd: Message = r.json().await.unwrap();
    let f = fwd.forwarded.unwrap();
    assert_eq!(f.channel_kind, ChannelKind::Dm);
    assert_eq!(f.channel_name, None);
    assert_eq!(fwd.content, "↪ Forwarded: my new number is 555");
    // search finds it by the words it forwarded (#415), and shows them
    let found = search(&app, &sam, s.id, "number").await;
    assert_eq!(found.hits.len(), 1);
    assert_eq!(found.hits[0].message.id, fwd.id);
    assert_eq!(found.hits[0].text, "my new number is 555");

    // alex deletes it: the forward shows nothing of it
    let r = app
        .http
        .delete(app.url(&format!("/messages/{}", original.id)))
        .bearer_auth(&alex)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let seen = history(&app, &sam, g.id).await;
    let got = seen.iter().find(|m| m.id == fwd.id).unwrap();
    let f = got.forwarded.as_ref().unwrap();
    assert!(f.unavailable);
    assert_eq!(f.content, "");
    assert_eq!(got.content, "↪ Forwarded a message");
    let raw = serde_json::to_string(&seen).unwrap();
    assert!(!raw.contains("555"), "{raw}");
    // nor does search find it by them any more
    assert!(search(&app, &sam, s.id, "number").await.hits.is_empty());
    // and it can't be forwarded on
    let r = forward(&app, &sam, fwd.id, json!({"channel_id": g.id})).await;
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn only_what_you_can_see_goes_only_where_you_can_post() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (_, kim) = register(&app, "kim").await;
    let (jo_id, jo) = register(&app, "jo").await;
    let dm = create_dm(&app, &alex, &[sam_id]).await;
    let secret = send(&app, &alex, dm.id, "between us").await;
    let kim_jo = create_dm(&app, &kim, &[jo_id]).await;
    let s = create_server(&app, &alex, "Main").await;
    let g = general(&app, &alex, s.id).await;

    // kim can't see the DM: it doesn't exist for her
    let r = forward(&app, &kim, secret.id, json!({"channel_id": kim_jo.id})).await;
    assert_eq!(r.status(), 404);
    // sam can see it, but can't post in kim and jo's DM, nor in a server he isn't in
    let r = forward(&app, &sam, secret.id, json!({"channel_id": kim_jo.id})).await;
    assert_eq!(r.status(), 404);
    let r = forward(&app, &sam, secret.id, json!({"channel_id": g.id})).await;
    assert_eq!(r.status(), 403);
    // nor can jo forward a server message she isn't a member for
    let public = send(&app, &alex, g.id, "welcome").await;
    let r = forward(&app, &jo, public.id, json!({"channel_id": kim_jo.id})).await;
    assert_eq!(r.status(), 404);
    // a system line can't be forwarded
    let r = app
        .http
        .put(app.url(&format!("/messages/{}/pin", public.id)))
        .bearer_auth(&alex)
        .json(&json!({"duration": "forever"}))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "pin: {}", r.status());
    let lines = history(&app, &alex, g.id).await;
    let line = lines
        .iter()
        .find(|m| m.pin_line.is_some())
        .expect("a pin line");
    let r = forward(&app, &alex, line.id, json!({"channel_id": dm.id})).await;
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn forwards_are_rate_limited() {
    let mut cfg = test_config();
    cfg.limits.forward_user = pulse_server::limits::Rate::new(2, 0.0001);
    let app = spawn_with(cfg).await;
    let (_, alex) = register(&app, "alex").await;
    let (sam_id, _) = register(&app, "sam").await;
    let dm = create_dm(&app, &alex, &[sam_id]).await;
    let m = send(&app, &alex, dm.id, "hi").await;
    for _ in 0..2 {
        let r = forward(&app, &alex, m.id, json!({"channel_id": dm.id})).await;
        assert_eq!(r.status(), 200);
    }
    let r = forward(&app, &alex, m.id, json!({"channel_id": dm.id})).await;
    assert_eq!(r.status(), 429);
}

/// #415: a deleted original's forwards are all told, past the first batch ([`TELL_MAX`] at a time).
#[tokio::test]
async fn every_forward_hears_its_original_was_deleted() {
    use pulse_protocol::gateway::{Event, ServerFrame};
    use pulse_protocol::ids::MessageId;
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let s = create_server(&app, &sam, "Main").await;
    let g = general(&app, &sam, s.id).await;
    let dm = create_dm(&app, &alex, &[sam_id]).await;
    let original = send(&app, &alex, dm.id, "see you there").await;
    let r = forward(&app, &sam, original.id, json!({"channel_id": g.id})).await;
    assert_eq!(r.status(), 200);
    let first: Message = r.json().await.unwrap();
    // more forwards than one batch, copied in the database (the forward route is rate limited)
    let total = pulse_server::forwards::TELL_MAX as usize + 5;
    for _ in 1..total {
        let id = MessageId::new().to_string();
        sqlx::query(
            "INSERT INTO messages (id, channel_id, author_id, kind, content, created_at)
             SELECT ?, channel_id, author_id, kind, content, created_at FROM messages WHERE id = ?",
        )
        .bind(&id)
        .bind(first.id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO message_forwards (message_id, source_id, source_channel_id, source_author_id,
                 source_created_at, content, gif, has_comment)
             SELECT ?, source_id, source_channel_id, source_author_id, source_created_at, content, gif, has_comment
             FROM message_forwards WHERE message_id = ?",
        )
        .bind(&id)
        .bind(first.id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    }

    let mut rx = app.hub.register(sam_id, String::new()).rx;
    let r = app
        .http
        .delete(app.url(&format!("/messages/{}", original.id)))
        .bearer_auth(&alex)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let mut told = std::collections::HashSet::new();
    while told.len() < total {
        let f = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("every forward is told")
            .expect("hub open");
        if let ServerFrame::Event(Event::MessageUpdated { message }) = f
            && message.forwarded.as_ref().is_some_and(|f| f.unavailable)
        {
            told.insert(message.id);
        }
    }
    assert_eq!(told.len(), total);
}
