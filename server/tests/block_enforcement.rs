//! Where a block (#474) is enforced beyond 1:1 messages: groups, calls, edits and the reads that go straight to a
//! message (its files, view-once); invites that outlive their maker's membership or ownership; and a few writes an
//! outsider must not make (the access matrix covers reads).
mod common;

use std::time::Duration;

use base64::Engine;
use common::*;
use futures::{SinkExt, StreamExt};
use livekit_api::access_token::AccessToken;
use pulse_protocol::gateway::{ClientFrame, Event, ServerFrame};
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{Message, ServerInvite};
use serde_json::json;
use sha2::{Digest, Sha256};
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
        if let Some(ServerFrame::Ready(_)) = next_frame(&mut ws).await {
            return ws;
        }
    }
}

async fn next_frame(ws: &mut Socket) -> Option<ServerFrame> {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .ok()??
            .ok()?;
        match m {
            Ws::Text(t) => return Some(serde_json::from_str(&t).unwrap()),
            Ws::Close(_) => return None,
            _ => continue,
        }
    }
}

async fn drain(ws: &mut Socket, ms: u64) -> Vec<Event> {
    let mut got = vec![];
    while let Ok(Some(f)) = tokio::time::timeout(Duration::from_millis(ms), next_frame(ws)).await {
        if let ServerFrame::Event(e) = f {
            got.push(e);
        }
    }
    got
}

async fn block(app: &TestApp, token: &str, who: UserId) {
    let r = app
        .http
        .put(app.url(&format!("/me/blocks/{who}")))
        .bearer_auth(token)
        .json(&json!({ "mode": "line" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}

async fn error_message(r: reqwest::Response) -> String {
    let v: serde_json::Value = r.json().await.unwrap();
    v["message"].as_str().unwrap_or_default().to_string()
}

async fn history(app: &TestApp, token: &str, ch: ChannelId) -> Vec<String> {
    let list: Vec<Message> = get_json(app, token, &format!("/channels/{ch}/messages")).await;
    list.into_iter().map(|m| m.content).collect()
}

fn signed(app: &TestApp, body: &str) -> String {
    let sum = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(body.as_bytes()));
    AccessToken::with_api_key(&app.cfg.livekit_key, &app.cfg.livekit_secret)
        .with_sha256(&sum)
        .to_jwt()
        .unwrap()
}

/// A LiveKit join webhook for `user` in `room`, carrying a live grant (as a real join would).
async fn lk_join(app: &TestApp, room: ChannelId, user: UserId, sid: &str) {
    let grant = grant_livekit(app, &room.to_string(), &user.to_string()).await;
    let body = json!({"event": "participant_joined", "id": "EV", "room": {"name": room.to_string()},
        "participant": {"identity": user.to_string(), "sid": sid, "attributes": {"pulse_grant": grant}}})
    .to_string();
    let r = app
        .http
        .post(app.url("/livekit/webhook"))
        .header("Authorization", signed(app, &body))
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}

async fn voice_token(app: &TestApp, token: &str, ch: ChannelId) -> u16 {
    post_json(app, token, &format!("/voice/{ch}/token"), json!({}))
        .await
        .status()
        .as_u16()
}

#[tokio::test]
async fn nobody_puts_you_in_a_group_with_someone_who_blocked_them() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (kim_id, kim) = register(&app, "kim").await;
    let (jo_id, _jo) = register(&app, "jo").await;
    // a group from before the block stays
    let before = create_dm(&app, &sam, &[alex_id, kim_id]).await;
    block(&app, &alex, sam_id).await;

    // a new group with alex: refused, as for someone who isn't there
    let r = post_json(&app, &sam, "/dms", json!({ "user_ids": [alex_id, kim_id] })).await;
    assert_eq!(r.status(), 400);
    let refused = error_message(r).await;
    let r = post_json(
        &app,
        &sam,
        "/dms",
        json!({ "user_ids": [UserId::new(), kim_id] }),
    )
    .await;
    assert_eq!(r.status(), 400);
    assert_eq!(refused, error_message(r).await, "no different from nobody");
    assert!(!refused.contains("block"));
    // adding alex to a group sam's in: the same
    let group = create_dm(&app, &sam, &[kim_id, jo_id]).await;
    let r = post_json(
        &app,
        &sam,
        &format!("/channels/{}/members", group.id),
        json!({ "user_ids": [alex_id] }),
    )
    .await;
    assert_eq!(r.status(), 400);
    assert_eq!(error_message(r).await, refused);
    assert_eq!(
        count(
            &app,
            &format!(
                "SELECT COUNT(*) FROM channel_members WHERE channel_id = '{}' AND user_id = '{alex_id}'",
                group.id
            )
        )
        .await,
        0
    );
    // anyone else may still add alex, and alex may still group with sam (only the blocked person is refused)
    let r = post_json(
        &app,
        &kim,
        &format!("/channels/{}/members", group.id),
        json!({ "user_ids": [alex_id] }),
    )
    .await;
    assert_eq!(r.status(), 204);
    let r = post_json(&app, &alex, "/dms", json!({ "user_ids": [sam_id, jo_id] })).await;
    assert_eq!(r.status(), 200);
    // the old group is untouched
    assert_eq!(
        count(
            &app,
            &format!(
                "SELECT COUNT(*) FROM channel_members WHERE channel_id = '{}'",
                before.id
            )
        )
        .await,
        3
    );
}

#[tokio::test]
async fn a_blocked_person_never_shares_a_dm_call_with_the_blocker() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let dm = create_dm(&app, &sam, &[alex_id]).await.id;
    block(&app, &alex, sam_id).await;

    // the blocker can't call or join (as they can't message)
    let r = post_json(&app, &alex, &format!("/calls/{dm}"), json!({})).await;
    assert_eq!(r.status(), 403);
    assert!(error_message(r).await.contains("Unblock"));
    assert_eq!(voice_token(&app, &alex, dm).await, 403);
    // the blocked person can still call: it rings out (alex isn't rung), as it did
    let r = post_json(&app, &sam, &format!("/calls/{dm}"), json!({})).await;
    assert_eq!(r.status(), 200);
    assert_eq!(voice_token(&app, &sam, dm).await, 200);
    lk_join(&app, dm, sam_id, "PS1").await;
    assert!(app.voice_members(dm).contains(&sam_id));
    // and a join by alex with a token from before is refused at LiveKit's webhook
    lk_join(&app, dm, alex_id, "PA1").await;
    assert!(!app.voice_members(dm).contains(&alex_id));
}

#[tokio::test]
async fn blocking_mid_call_takes_the_blocked_person_out_and_keeps_them_out() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let dm = create_dm(&app, &sam, &[alex_id]).await.id;
    lk_join(&app, dm, alex_id, "PA1").await;
    lk_join(&app, dm, sam_id, "PS1").await;
    assert!(app.voice_members(dm).contains(&sam_id));

    block(&app, &alex, sam_id).await;
    let mut gone = false;
    for _ in 0..50 {
        if !app.voice_members(dm).contains(&sam_id) {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(gone, "sam is taken out of the call");
    assert!(app.voice_members(dm).contains(&alex_id), "alex stays");
    // and can't come back while alex is in it
    assert_eq!(voice_token(&app, &sam, dm).await, 403);
    let r = post_json(&app, &sam, &format!("/calls/{dm}"), json!({})).await;
    assert_eq!(r.status(), 404);
    lk_join(&app, dm, sam_id, "PS2").await;
    assert!(!app.voice_members(dm).contains(&sam_id));
}

#[tokio::test]
async fn edits_after_a_block_never_reach_the_blocker() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let dm = create_dm(&app, &sam, &[alex_id]).await.id;
    let m = send(&app, &sam, dm, "before").await;
    block(&app, &alex, sam_id).await;
    let mut alex_ws = hello(&app, &alex).await;
    drain(&mut alex_ws, 200).await;

    let r = app
        .http
        .patch(app.url(&format!("/messages/{}", m.id)))
        .bearer_auth(&sam)
        .json(&json!({ "content": "after" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "looks edited to sam");
    assert_eq!(r.json::<Message>().await.unwrap().content, "after");
    assert_eq!(history(&app, &sam, dm).await, vec!["after"]);
    // nothing reaches alex: no update, and not the new text in history
    assert!(
        !drain(&mut alex_ws, 300)
            .await
            .iter()
            .any(|e| matches!(e, Event::MessageUpdated { .. }))
    );
    assert!(history(&app, &alex, dm).await.is_empty());
    // nor after an unblock: what was withheld stays withheld
    let r = app
        .http
        .delete(app.url(&format!("/me/blocks/{sam_id}")))
        .bearer_auth(&alex)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert!(history(&app, &alex, dm).await.is_empty());
}

#[tokio::test]
async fn a_withheld_message_s_files_and_view_once_stay_withheld() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let dm = create_dm(&app, &sam, &[alex_id]).await.id;
    block(&app, &alex, sam_id).await;
    let m = send(&app, &sam, dm, "withheld").await;
    // a view-once file on it, as a send with one leaves it
    let att = pulse_protocol::ids::next_ulid().to_string();
    sqlx::query(
        "INSERT INTO attachments (id, key, user_id, channel_id, message_id, name, content_type, kind, size, created_at, view_once)
         VALUES (?, ?, ?, ?, ?, 'a.png', 'image/png', 'image', 10, ?, 1)",
    )
    .bind(att.clone())
    .bind(format!("k/{att}"))
    .bind(sam_id.to_string())
    .bind(dm.to_string())
    .bind(m.id.to_string())
    .bind(pulse_server::db::now())
    .execute(&app.db)
    .await
    .unwrap();

    let urls = |token: String| {
        let app = &app;
        async move {
            app.http
                .get(app.url(&format!("/messages/{}/attachments/urls", m.id)))
                .bearer_auth(token)
                .send()
                .await
                .unwrap()
                .status()
                .as_u16()
        }
    };
    assert_eq!(urls(sam.clone()).await, 200);
    assert_eq!(urls(alex.clone()).await, 404);
    let view_once = |token: String| {
        let app = &app;
        async move {
            app.http
                .get(app.url(&format!("/messages/{}/view-once", m.id)))
                .bearer_auth(token)
                .send()
                .await
                .unwrap()
                .status()
                .as_u16()
        }
    };
    assert_eq!(view_once(sam.clone()).await, 200);
    assert_eq!(view_once(alex.clone()).await, 404);
    let r = post_json(
        &app,
        &alex,
        &format!("/messages/{}/view-once/open", m.id),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 404);
}

async fn make_invite(app: &TestApp, token: &str, server: impl std::fmt::Display) -> ServerInvite {
    let r = post_json(app, token, &format!("/servers/{server}/invites"), json!({})).await;
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

async fn accept(app: &TestApp, token: &str, code: &str) -> u16 {
    post_json(app, token, &format!("/invites/{code}/accept"), json!({}))
        .await
        .status()
        .as_u16()
}

#[tokio::test]
async fn invites_stop_working_when_their_maker_leaves_or_is_removed() {
    let app = spawn().await;
    let (_alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (_kim_id, kim) = register(&app, "kim").await;
    let (_jo_id, jo) = register(&app, "jo").await;
    let server = create_server(&app, &alex, "Den").await.id;
    assert_eq!(join_server(&app, &sam, server).await.status(), 200);
    assert_eq!(join_server(&app, &kim, server).await.status(), 200);

    // sam leaves: sam's invite goes with them
    let sams = make_invite(&app, &sam, server).await;
    let r = post_json(&app, &sam, &format!("/servers/{server}/leave"), json!({})).await;
    assert_eq!(r.status(), 204);
    assert_eq!(accept(&app, &jo, &sams.code).await, 404);
    // kim is removed: the same
    let kims = make_invite(&app, &kim, server).await;
    let r = app
        .http
        .delete(app.url(&format!("/servers/{server}/members/{_kim_id}")))
        .bearer_auth(&alex)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(
        count(
            &app,
            &format!(
                "SELECT COUNT(*) FROM server_invites WHERE code IN ('{}', '{}') AND revoked_at IS NOT NULL",
                sams.code, kims.code
            )
        )
        .await,
        2
    );
    assert_eq!(accept(&app, &jo, &kims.code).await, 404);
    // one left live from before this rule (its maker gone): refused all the same
    sqlx::query("UPDATE server_invites SET revoked_at = NULL WHERE code = ?")
        .bind(&sams.code)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(accept(&app, &jo, &sams.code).await, 404);
    // the owner's still works
    let alexs = make_invite(&app, &alex, server).await;
    assert_eq!(accept(&app, &jo, &alexs.code).await, 200);
    let _ = sam_id;
}

#[tokio::test]
async fn a_hand_over_stops_the_old_owner_s_links_making_accounts() {
    let app = spawn().await;
    let (_alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let server = create_server(&app, &alex, "Den").await.id;
    assert_eq!(join_server(&app, &sam, server).await.status(), 200);
    let link = make_invite(&app, &alex, server).await;
    let signup = |code: String| {
        let app = &app;
        async move {
            count(
                app,
                &format!("SELECT signup_ok FROM server_invites WHERE code = '{code}'"),
            )
            .await
        }
    };
    assert_eq!(signup(link.code.clone()).await, 1);

    let r = app
        .http
        .put(app.url(&format!("/servers/{server}/owner")))
        .bearer_auth(&alex)
        .json(&json!({ "user_id": sam_id }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(signup(link.code.clone()).await, 0);
    // one from before this rule, still marked: no sign-up through it either (its maker no longer owns the server)
    sqlx::query("UPDATE server_invites SET signup_ok = 1 WHERE code = ?")
        .bind(&link.code)
        .execute(&app.db)
        .await
        .unwrap();
    let r = app
        .http
        .post(app.url("/auth/register"))
        .json(&json!({"invite_code": link.code, "username": "kim", "password": "hunter2hunter2"}))
        .send()
        .await
        .unwrap();
    assert_ne!(r.status(), 200);
    // still a member's link to join by
    let (_jo_id, jo) = register(&app, "jo").await;
    assert_eq!(accept(&app, &jo, &link.code).await, 200);
}

/// Writes someone outside a conversation or server must not make (a few, beside the access matrix's reads).
#[tokio::test]
async fn outsiders_can_t_write_into_what_isn_t_theirs() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (kim_id, kim) = register(&app, "kim").await;
    let (_jo_id, jo) = register(&app, "jo").await;
    let server = create_server(&app, &alex, "Den").await.id;
    let general = general(&app, &alex, server).await.id;
    let m = send(&app, &alex, general, "hello").await;
    let r = post_json(
        &app,
        &alex,
        &format!("/channels/{general}/polls"),
        json!({"question": "Tonight?", "options": ["Yes", "No"], "ends_in": "day"}),
    )
    .await;
    assert_eq!(r.status(), 201);
    let poll: Message = r.json().await.unwrap();
    let group = create_dm(&app, &alex, &[sam_id, kim_id]).await.id;
    let r = post_json(
        &app,
        &alex,
        &format!("/channels/{general}/scheduled"),
        json!({"content": "later", "send_at": pulse_server::times::rfc3339(
            jiff::Timestamp::now() + jiff::SignedDuration::from_hours(5))}),
    )
    .await;
    assert!(r.status().is_success());
    let scheduled: serde_json::Value = r.json().await.unwrap();
    let scheduled_id = scheduled["id"].as_str().unwrap().to_string();

    // jo is in none of it
    let r = app
        .http
        .put(app.url(&format!("/messages/{}/reactions/%F0%9F%91%8D", m.id)))
        .bearer_auth(&jo)
        .send()
        .await
        .unwrap();
    assert!(
        [403, 404].contains(&r.status().as_u16()),
        "reaction: {}",
        r.status()
    );
    let r = app
        .http
        .put(app.url(&format!("/messages/{}/poll/votes", poll.id)))
        .bearer_auth(&jo)
        .json(&json!({ "options": [0] }))
        .send()
        .await
        .unwrap();
    assert!(
        [403, 404].contains(&r.status().as_u16()),
        "vote: {}",
        r.status()
    );
    let r = post_json(
        &app,
        &jo,
        &format!("/channels/{group}/members"),
        json!({ "user_ids": [_jo_id] }),
    )
    .await;
    assert!(
        [403, 404].contains(&r.status().as_u16()),
        "members add: {}",
        r.status()
    );
    // someone else's scheduled message isn't yours to change, even from inside
    for token in [&jo, &sam] {
        let r = app
            .http
            .patch(app.url(&format!("/scheduled/{scheduled_id}")))
            .bearer_auth(token)
            .json(&json!({ "content": "mine now" }))
            .send()
            .await
            .unwrap();
        assert!(
            [403, 404].contains(&r.status().as_u16()),
            "scheduled edit: {}",
            r.status()
        );
    }
    // nothing changed
    assert_eq!(count(&app, "SELECT COUNT(*) FROM reactions").await, 0);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM poll_votes").await, 0);
    assert_eq!(
        count(
            &app,
            &format!("SELECT COUNT(*) FROM channel_members WHERE channel_id = '{group}'")
        )
        .await,
        3
    );
    assert_eq!(
        count(
            &app,
            "SELECT COUNT(*) FROM scheduled_messages WHERE content = 'later'"
        )
        .await,
        1
    );
    let _ = (alex_id, kim);
}
