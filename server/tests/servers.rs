mod common;

use common::*;
use pulse_protocol::rest::{Channel, ChannelKind, Member, Server};
use pulse_server::access::channel_for;
use pulse_server::error::AppError;
use serde_json::json;

#[tokio::test]
async fn create_server_seeds_general_and_lounge() {
    let app = spawn().await;
    let (_, t) = register(&app, "alex").await;
    let s = create_server(&app, &t, "Main Hangout").await;
    let chans: Vec<Channel> = get_json(&app, &t, &format!("/servers/{}/channels", s.id)).await;
    assert!(
        chans
            .iter()
            .any(|c| c.kind == ChannelKind::Text && c.name.as_deref() == Some("general"))
    );
    assert!(
        chans
            .iter()
            .any(|c| c.kind == ChannelKind::Voice && c.name.as_deref() == Some("Lounge"))
    );
    let members: Vec<Member> = get_json(&app, &t, &format!("/servers/{}/members", s.id)).await;
    assert_eq!(members.len(), 1);
}

#[tokio::test]
async fn server_name_validated() {
    let app = spawn().await;
    let (_, t) = register(&app, "alex").await;
    assert_eq!(
        post_json(&app, &t, "/servers", json!({"name": "  "}))
            .await
            .status(),
        400
    );
}

#[tokio::test]
async fn all_servers_visible_to_everyone() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (_, b) = register(&app, "sam").await;
    let s = create_server(&app, &a, "Main Hangout").await;
    let list: Vec<Server> = get_json(&app, &b, "/servers").await;
    assert_eq!(list, vec![s]);
}

#[tokio::test]
async fn join_is_idempotent() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (_, b) = register(&app, "sam").await;
    let s = create_server(&app, &a, "Main").await;
    for _ in 0..2 {
        let r = post_json(&app, &b, &format!("/servers/{}/join", s.id), json!({})).await;
        assert_eq!(r.status(), 204);
    }
    let members: Vec<Member> = get_json(&app, &a, &format!("/servers/{}/members", s.id)).await;
    assert_eq!(members.len(), 2);
}

#[tokio::test]
async fn join_unknown_server_404() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let r = post_json(
        &app,
        &a,
        &format!("/servers/{}/join", pulse_protocol::ids::ServerId::new()),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn non_member_cannot_create_channel_403() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (_, b) = register(&app, "sam").await;
    let s = create_server(&app, &a, "Main").await;
    let path = format!("/servers/{}/channels", s.id);
    assert_eq!(
        post_json(&app, &b, &path, json!({"kind": "text", "name": "memes"}))
            .await
            .status(),
        403
    );
    assert_eq!(
        post_json(&app, &a, &path, json!({"kind": "text", "name": "memes"}))
            .await
            .status(),
        200
    );
    assert_eq!(
        post_json(&app, &a, &path, json!({"kind": "dm", "name": "sneaky"}))
            .await
            .status(),
        400
    );
}

#[tokio::test]
async fn dm_reused_for_same_pair() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let d1 = create_dm(&app, &a, &[b_id]).await;
    let d2 = create_dm(&app, &b, &[a_id]).await;
    assert_eq!(d1.id, d2.id);
    assert_eq!(d1.kind, ChannelKind::Dm);
    assert_eq!(d1.server_id, None);
}

#[tokio::test]
async fn group_includes_creator() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (b_id, _) = register(&app, "sam").await;
    let (c_id, c) = register(&app, "jo").await;
    let g = create_dm(&app, &a, &[b_id, c_id]).await;
    assert_eq!(g.kind, ChannelKind::Group);
    let mine: Vec<Channel> = get_json(&app, &c, "/dms").await;
    assert_eq!(mine, vec![g.clone()]);
    assert!(channel_for(&app.db, a_id, g.id).await.is_ok());
}

#[tokio::test]
async fn dm_validation() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    assert_eq!(
        post_json(&app, &a, "/dms", json!({"user_ids": []}))
            .await
            .status(),
        400
    );
    assert_eq!(
        post_json(&app, &a, "/dms", json!({"user_ids": [a_id]}))
            .await
            .status(),
        400
    );
    let ghost = pulse_protocol::ids::UserId::new();
    assert_eq!(
        post_json(&app, &a, "/dms", json!({"user_ids": [ghost]}))
            .await
            .status(),
        400
    );
}

#[tokio::test]
async fn dms_list_only_mine() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, _) = register(&app, "sam").await;
    let (_, c) = register(&app, "jo").await;
    create_dm(&app, &a, &[b_id]).await;
    let theirs: Vec<Channel> = get_json(&app, &c, "/dms").await;
    assert!(theirs.is_empty());
}

#[tokio::test]
async fn access_rule_hides_private_channel() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, _) = register(&app, "sam").await;
    let (c_id, _) = register(&app, "jo").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    assert!(matches!(
        channel_for(&app.db, c_id, dm.id).await,
        Err(AppError::NotFound)
    ));
    assert!(channel_for(&app.db, b_id, dm.id).await.is_ok());
    // unknown channel looks exactly the same
    assert!(matches!(
        channel_for(&app.db, c_id, pulse_protocol::ids::ChannelId::new()).await,
        Err(AppError::NotFound)
    ));
}
