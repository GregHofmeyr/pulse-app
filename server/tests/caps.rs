//! Hard caps on rows a member can create: servers, channels, groups, invites, emoji.
mod common;

use common::*;
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{ApiError, Channel};
use serde_json::json;

async fn bad_request(r: reqwest::Response, what: &str) -> String {
    assert_eq!(r.status(), 400, "{what}");
    let e: ApiError = r.json().await.unwrap();
    assert_eq!(e.code, "bad_request");
    e.message
}

#[tokio::test]
async fn servers_per_user_cap_10() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    for i in 0..10 {
        create_server(&app, &alex, &format!("s{i}")).await;
    }
    let r = post_json(&app, &alex, "/servers", json!({ "name": "one more" })).await;
    let msg = bad_request(r, "11th server").await;
    assert!(msg.contains("10"), "{msg}");
    // Per creator: someone else still can.
    create_server(&app, &sam, "sam's").await;
}

#[tokio::test]
async fn channels_per_server_cap_100() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let srv = create_server(&app, &alex, "Main").await; // general + Lounge
    let path = format!("/servers/{}/channels", srv.id);
    for i in 0..98 {
        let kind = if i % 2 == 0 { "text" } else { "voice" };
        let r = post_json(
            &app,
            &alex,
            &path,
            json!({ "name": format!("c{i}"), "kind": kind }),
        )
        .await;
        assert_eq!(r.status(), 200, "channel {i}");
    }
    let r = post_json(&app, &alex, &path, json!({ "name": "c98", "kind": "text" })).await;
    let msg = bad_request(r, "101st channel").await;
    assert!(msg.contains("100"), "{msg}");
    // Another server is unaffected.
    let other = create_server(&app, &alex, "Other").await;
    let r = post_json(
        &app,
        &alex,
        &format!("/servers/{}/channels", other.id),
        json!({ "name": "x", "kind": "text" }),
    )
    .await;
    assert_eq!(r.status(), 200);
}

async fn new_invite(app: &TestApp, token: &str) -> reqwest::Response {
    post_json(app, token, "/invites", json!({})).await
}

#[tokio::test]
async fn outstanding_invites_per_user_cap_20() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    // Admin-minted invites (no creator) don't count against anyone.
    for _ in 0..3 {
        invite(&app).await;
    }
    let mut codes = Vec::new();
    for i in 0..20 {
        let r = new_invite(&app, &alex).await;
        assert_eq!(r.status(), 200, "invite {i}");
        let v: serde_json::Value = r.json().await.unwrap();
        codes.push(v["code"].as_str().unwrap().to_string());
    }
    let msg = bad_request(new_invite(&app, &alex).await, "21st invite").await;
    assert!(msg.contains("20"), "{msg}");

    // A used invite frees a slot.
    let r = app
        .http
        .post(app.url("/auth/register"))
        .json(&json!({"invite_code": codes[0], "username": "sam", "password": "hunter2hunter2"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(new_invite(&app, &alex).await.status(), 200);
    assert_eq!(new_invite(&app, &alex).await.status(), 400);

    // So does an expired one.
    sqlx::query("UPDATE invites SET created_at = '2020-01-01T00:00:00.000Z' WHERE code = ?")
        .bind(&codes[1])
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(new_invite(&app, &alex).await.status(), 200);
    assert_eq!(new_invite(&app, &alex).await.status(), 400);
}

#[tokio::test]
async fn emoji_rows_per_server_cap_500_counts_deleted() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let srv = create_server(&app, &alex, "Main").await;
    let media: pulse_protocol::rest::MediaInfo = {
        let img = image::RgbaImage::from_pixel(32, 32, image::Rgba([1, 2, 3, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        let r = app
            .http
            .post(app.url("/media/emoji"))
            .bearer_auth(&alex)
            .body(out.into_inner())
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        r.json().await.unwrap()
    };
    // 499 soft-deleted rows: none live, so the live cap of 100 is far away.
    for i in 0..499 {
        sqlx::query(
            "INSERT INTO custom_emoji (id, server_id, name, media_hash, animated, added_by, created_at, deleted_at)
             VALUES (?, ?, ?, NULL, 0, ?, '2026-10-01T00:00:00.000Z', '2026-10-01T00:00:01.000Z')",
        )
        .bind(format!("D{i:03}"))
        .bind(srv.id.to_string())
        .bind(format!("d{i:03}"))
        .bind(alex_id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    }
    let path = format!("/servers/{}/emoji", srv.id);
    // The 500th row is fine.
    let r = post_json(
        &app,
        &alex,
        &path,
        json!({ "name": "pog", "media_hash": media.hash }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let e: pulse_protocol::rest::CustomEmoji = r.json().await.unwrap();
    let r = app
        .http
        .delete(app.url(&format!("/emoji/{}", e.id)))
        .bearer_auth(&alex)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    // Deleting doesn't free a row: the 501st is refused.
    let r = post_json(
        &app,
        &alex,
        &path,
        json!({ "name": "pog2", "media_hash": media.hash }),
    )
    .await;
    let msg = bad_request(r, "501st emoji row").await;
    assert!(msg.contains("500"), "{msg}");
}

async fn add(app: &TestApp, token: &str, ch: ChannelId, users: &[UserId]) -> reqwest::Response {
    post_json(
        app,
        token,
        &format!("/channels/{ch}/members"),
        json!({ "user_ids": users }),
    )
    .await
}

async fn groups_of(app: &TestApp, u: UserId) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM channel_members cm JOIN channels c ON c.id = cm.channel_id
         WHERE cm.user_id = ? AND c.kind = 'group'",
    )
    .bind(u.to_string())
    .fetch_one(&app.db)
    .await
    .unwrap()
}

#[tokio::test]
async fn group_memberships_per_user_cap_200() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, _) = register(&app, "sam").await;
    let (riley_id, riley) = register(&app, "riley").await;

    // A group of alex and sam only (riley left it).
    let g: Channel = create_dm(&app, &alex, &[sam_id, riley_id]).await;
    let r = app
        .http
        .delete(app.url(&format!("/channels/{}/members/{riley_id}", g.id)))
        .bearer_auth(&riley)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);

    // alex and sam reach 200, riley 199.
    for _ in 0..199 {
        create_dm(&app, &alex, &[sam_id, riley_id]).await;
    }
    assert_eq!(groups_of(&app, alex_id).await, 200);
    assert_eq!(groups_of(&app, sam_id).await, 200);
    assert_eq!(groups_of(&app, riley_id).await, 199);

    // The creator is at the cap.
    let r = post_json(
        &app,
        &alex,
        "/dms",
        json!({ "user_ids": [sam_id, riley_id] }),
    )
    .await;
    let msg = bad_request(r, "alex's 201st group").await;
    assert!(msg.contains("200"), "{msg}");
    // The creator is under it, but the others aren't: the whole request fails.
    let r = post_json(
        &app,
        &riley,
        "/dms",
        json!({ "user_ids": [alex_id, sam_id] }),
    )
    .await;
    bad_request(r, "riley's group with two full members").await;
    assert_eq!(groups_of(&app, riley_id).await, 199);

    // A 1:1 DM is not a group.
    let dm = create_dm(&app, &alex, &[riley_id]).await;
    assert_eq!(dm.kind, pulse_protocol::rest::ChannelKind::Dm);

    // Adding riley to G takes riley to exactly 200: fine.
    assert_eq!(add(&app, &alex, g.id, &[riley_id]).await.status(), 204);
    assert_eq!(groups_of(&app, riley_id).await, 200);
    // riley leaves G again (199), then joins one more group elsewhere (200, seeded).
    let r = app
        .http
        .delete(app.url(&format!("/channels/{}/members/{riley_id}", g.id)))
        .bearer_auth(&riley)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let extra = ChannelId::new();
    sqlx::query("INSERT INTO channels (id, server_id, kind, name, position, created_at) VALUES (?, NULL, 'group', NULL, 0, '2026-10-01T00:00:00.000Z')")
        .bind(extra.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO channel_members (channel_id, user_id, added_by, added_at) VALUES (?, ?, NULL, '2026-10-01T00:00:00.000Z')")
        .bind(extra.to_string())
        .bind(riley_id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(groups_of(&app, riley_id).await, 200);
    // Adding someone already in 200 groups fails, and nobody is added.
    let r = add(&app, &alex, g.id, &[riley_id]).await;
    bad_request(r, "add riley at the cap").await;
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM channel_members WHERE channel_id = ?")
        .bind(g.id.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(n, 2);
}
