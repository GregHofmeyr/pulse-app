//! DMs, groups, unread, mentions, mutes, presence.
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_server::gateway::audience::{Audience, audience_for};
use tokio_tungstenite::tungstenite::Message as Ws;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn hello(app: &TestApp, token: &str) -> (Socket, Ready) {
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
        if let Some(ServerFrame::Ready(r)) = next_frame(&mut ws).await {
            return (ws, r);
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

/// Next event matching `pred` (skips others); panics after 3 s.
async fn wait_for(ws: &mut Socket, pred: impl Fn(&Event) -> bool) -> Event {
    loop {
        match next_frame(ws).await.expect("socket ended") {
            ServerFrame::Event(e) if pred(&e) => return e,
            _ => continue,
        }
    }
}

#[tokio::test]
async fn owner_only_events_reach_only_their_owner() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (b_id, _) = register(&app, "sam").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    let e = Event::ReadStateUpdated {
        user_id: a_id,
        channel_id: dm.id,
        last_read_message_id: None,
    };
    assert_eq!(
        audience_for(&app.db, &e).await.unwrap(),
        Audience::Users([a_id].into_iter().collect()),
        "sam must never learn what alex read"
    );
}

#[tokio::test]
async fn last_seen_only_when_last_connection_closes() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (_, b) = register(&app, "sam").await;
    let (mut watcher, _) = hello(&app, &b).await;
    let (ws1, _) = hello(&app, &a).await;
    let (ws2, _) = hello(&app, &a).await;
    // (sam's own "online" arrives first; wait for alex's)
    wait_for(
        &mut watcher,
        |e| matches!(e, Event::PresenceChanged { user_id, online: true, .. } if *user_id == a_id),
    )
    .await;
    drop(ws1);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let seen: Option<String> = sqlx::query_scalar("SELECT last_seen_at FROM users WHERE id = ?")
        .bind(a_id.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(seen.is_none(), "still connected on another device");
    drop(ws2);
    let ev = wait_for(
        &mut watcher,
        |e| matches!(e, Event::PresenceChanged { user_id, online: false, .. } if *user_id == a_id),
    )
    .await;
    assert!(matches!(
        ev,
        Event::PresenceChanged {
            last_seen_at: Some(_),
            ..
        }
    ));
}

#[tokio::test]
async fn ready_lists_everyone_with_presence() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (b_id, _) = register(&app, "sam").await;
    let (_ws, ready) = hello(&app, &a).await;
    let me = ready.people.iter().find(|p| p.user.id == a_id).unwrap();
    let sam = ready.people.iter().find(|p| p.user.id == b_id).unwrap();
    assert!(me.online && !sam.online);
}

async fn read_state(
    r: &Ready,
    ch: pulse_protocol::ids::ChannelId,
) -> pulse_protocol::rest::ReadState {
    r.read_states
        .iter()
        .find(|s| s.channel_id == ch)
        .cloned()
        .expect("read state present")
}

#[tokio::test]
async fn unread_counts_others_messages_since_read_point() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    send(&app, &a, dm.id, "one").await;
    send(&app, &a, dm.id, "two").await;
    send(&app, &b, dm.id, "mine").await; // sam's own message: never unread for sam, and it marks sam read
    let (_ws, ready) = hello(&app, &b).await;
    assert_eq!(
        read_state(&ready, dm.id).await.unread,
        0,
        "sending marks your own read point"
    );
    send(&app, &a, dm.id, "three").await;
    let (_ws2, ready) = hello(&app, &b).await;
    assert_eq!(read_state(&ready, dm.id).await.unread, 1);
    let _ = a_id;
}

#[tokio::test]
async fn joining_a_server_starts_all_read() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (_, b) = register(&app, "sam").await;
    let s = create_server(&app, &a, "Main").await;
    let g = general(&app, &a, s.id).await;
    for i in 0..5 {
        send(&app, &a, g.id, &format!("old {i}")).await;
    }
    assert_eq!(
        post_json(
            &app,
            &b,
            &format!("/servers/{}/join", s.id),
            serde_json::json!({})
        )
        .await
        .status(),
        204
    );
    let (_ws, ready) = hello(&app, &b).await;
    assert_eq!(
        read_state(&ready, g.id).await.unread,
        0,
        "no wall of old unreads"
    );
}

#[tokio::test]
async fn mark_read_rejects_foreign_message_and_never_moves_back() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    let s = create_server(&app, &a, "Main").await;
    let g = general(&app, &a, s.id).await;
    let other = send(&app, &a, g.id, "elsewhere").await;
    let m1 = send(&app, &a, dm.id, "1").await;
    let m2 = send(&app, &a, dm.id, "2").await;
    let path = format!("/channels/{}/read", dm.id);
    assert_eq!(
        post_json(&app, &b, &path, serde_json::json!({"message_id": other.id}))
            .await
            .status(),
        400
    );
    assert_eq!(
        post_json(&app, &b, &path, serde_json::json!({"message_id": m2.id}))
            .await
            .status(),
        204
    );
    assert_eq!(
        post_json(&app, &b, &path, serde_json::json!({"message_id": m1.id}))
            .await
            .status(),
        204
    );
    let (_ws, ready) = hello(&app, &b).await;
    let st = read_state(&ready, dm.id).await;
    assert_eq!(st.last_read_message_id, Some(m2.id), "monotonic");
    assert_eq!(st.unread, 0);
}

#[tokio::test]
async fn mark_read_syncs_own_sessions_only() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    let m = send(&app, &a, dm.id, "hi").await;
    let (mut sam_other_device, _) = hello(&app, &b).await;
    let (mut alex, _) = hello(&app, &a).await;
    post_json(
        &app,
        &b,
        &format!("/channels/{}/read", dm.id),
        serde_json::json!({"message_id": m.id}),
    )
    .await;
    let e = wait_for(&mut sam_other_device, |e| {
        matches!(e, Event::ReadStateUpdated { .. })
    })
    .await;
    assert!(matches!(e, Event::ReadStateUpdated { user_id, .. } if user_id == b_id));
    // alex must not get it: send a marker and check it's the next thing alex sees
    send(&app, &a, dm.id, "marker").await;
    let next = wait_for(&mut alex, |e| !matches!(e, Event::PresenceChanged { .. })).await;
    assert!(
        matches!(next, Event::MessageCreated { .. }),
        "alex saw {next:?}"
    );
    let _ = a_id;
}

#[tokio::test]
async fn ready_has_latest_message_per_conversation() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    send(&app, &a, dm.id, "first").await;
    let last = send(&app, &a, dm.id, "last").await;
    let (_ws, ready) = hello(&app, &b).await;
    assert_eq!(
        ready
            .latest
            .iter()
            .find(|m| m.channel_id == dm.id)
            .map(|m| m.id),
        Some(last.id)
    );
}

#[tokio::test]
async fn mentions_only_people_who_can_see_the_channel() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let (c_id, _) = register(&app, "jo").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    let m = send(&app, &a, dm.id, "hey @Sam and @jo and @alex and @nobody").await;
    assert_eq!(
        m.mentions,
        vec![b_id],
        "jo can't see the DM; self and unknown don't count"
    );
    let (_ws, ready) = hello(&app, &b).await;
    assert_eq!(
        ready
            .read_states
            .iter()
            .find(|r| r.channel_id == dm.id)
            .unwrap()
            .mentions,
        1
    );
    let s = create_server(&app, &a, "Main").await;
    let g = general(&app, &a, s.id).await;
    let m2 = send(&app, &a, g.id, "@jo @jo look").await;
    assert_eq!(m2.mentions, vec![c_id], "server channels: anyone; deduped");
}

#[tokio::test]
async fn history_carries_mentions() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    send(&app, &a, dm.id, "@sam yo").await;
    let page: Vec<pulse_protocol::rest::Message> =
        get_json(&app, &b, &format!("/channels/{}/messages", dm.id)).await;
    assert_eq!(page[0].mentions, vec![b_id]);
}

async fn delete(app: &TestApp, token: &str, path: &str) -> u16 {
    app.http
        .delete(app.url(path))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}
async fn patch(
    app: &TestApp,
    token: &str,
    path: &str,
    body: serde_json::Value,
) -> reqwest::Response {
    app.http
        .patch(app.url(path))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
}
async fn history(
    app: &TestApp,
    token: &str,
    ch: pulse_protocol::ids::ChannelId,
) -> Vec<pulse_protocol::rest::Message> {
    get_json(app, token, &format!("/channels/{ch}/messages")).await
}

#[tokio::test]
async fn add_people_from_a_dm_makes_a_new_group_and_keeps_the_dm() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, _) = register(&app, "sam").await;
    let (c_id, _) = register(&app, "jo").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    let group = create_dm(&app, &a, &[b_id, c_id]).await;
    assert_ne!(dm.id, group.id);
    assert_eq!(
        create_dm(&app, &a, &[b_id]).await.id,
        dm.id,
        "the DM still exists"
    );
    let h = history(&app, &a, group.id).await;
    assert!(
        h.iter()
            .any(|m| m.kind == pulse_protocol::rest::MessageKind::System
                && m.content.contains("created the group"))
    );
}

#[tokio::test]
async fn flat_group_management_with_system_lines() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let (c_id, _) = register(&app, "jo").await;
    let (d_id, _) = register(&app, "riley").await;
    let g = create_dm(&app, &a, &[b_id, c_id]).await;
    // sam (not the creator) adds riley, removes jo, renames
    assert_eq!(
        post_json(
            &app,
            &b,
            &format!("/channels/{}/members", g.id),
            serde_json::json!({"user_ids": [d_id]})
        )
        .await
        .status(),
        204
    );
    assert_eq!(
        delete(&app, &b, &format!("/channels/{}/members/{}", g.id, c_id)).await,
        204
    );
    let r = patch(
        &app,
        &b,
        &format!("/channels/{}", g.id),
        serde_json::json!({"name": "  raid squad  "}),
    )
    .await;
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.json::<pulse_protocol::rest::Channel>()
            .await
            .unwrap()
            .name
            .as_deref(),
        Some("raid squad")
    );
    let lines: Vec<String> = history(&app, &a, g.id)
        .await
        .into_iter()
        .filter(|m| m.kind == pulse_protocol::rest::MessageKind::System)
        .map(|m| m.content)
        .collect();
    assert!(lines.iter().any(|l| l == "sam added riley"), "{lines:?}");
    assert!(lines.iter().any(|l| l == "sam removed jo"), "{lines:?}");
    assert!(
        lines
            .iter()
            .any(|l| l == "sam renamed the group to raid squad"),
        "{lines:?}"
    );
}

#[tokio::test]
async fn removed_member_cannot_post_or_read() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let (c_id, c) = register(&app, "jo").await;
    let g = create_dm(&app, &a, &[b_id, c_id]).await;
    let (mut jo_ws, _) = hello(&app, &c).await;
    assert_eq!(
        delete(&app, &a, &format!("/channels/{}/members/{}", g.id, c_id)).await,
        204
    );
    let e = wait_for(&mut jo_ws, |e| matches!(e, Event::ChannelRemoved { .. })).await;
    assert!(matches!(e, Event::ChannelRemoved { channel_id, .. } if channel_id == g.id));
    let r = post_json(
        &app,
        &c,
        &format!("/channels/{}/messages", g.id),
        serde_json::json!({"content": "late", "reply_to_id": null, "nonce": null}),
    )
    .await;
    assert_eq!(r.status(), 404);
    let r = app
        .http
        .get(app.url(&format!("/channels/{}/messages", g.id)))
        .bearer_auth(&c)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    let _ = b;
}

#[tokio::test]
async fn group_cap_dm_rename_and_last_leave() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let mut others = vec![b_id];
    for i in 0..8 {
        others.push(register(&app, &format!("u{i}")).await.0);
    }
    let g = create_dm(&app, &a, &others).await; // 10 people
    let (extra, eleven_tok) = register(&app, "eleven").await;
    assert_eq!(
        post_json(
            &app,
            &a,
            &format!("/channels/{}/members", g.id),
            serde_json::json!({"user_ids": [extra]})
        )
        .await
        .status(),
        400
    );
    let dm = create_dm(&app, &a, &[b_id]).await;
    assert_eq!(
        patch(
            &app,
            &a,
            &format!("/channels/{}", dm.id),
            serde_json::json!({"name": "x"})
        )
        .await
        .status(),
        400
    );
    let small = create_dm(&app, &a, &[b_id, extra]).await;
    for (id, tok) in [(b_id, &b)] {
        assert_eq!(
            delete(&app, tok, &format!("/channels/{}/members/{}", small.id, id)).await,
            204
        );
    }
    // eleven and alex leave → empty → deleted
    assert_eq!(
        delete(
            &app,
            &eleven_tok,
            &format!("/channels/{}/members/{}", small.id, extra)
        )
        .await,
        204
    );
    let me: pulse_protocol::rest::User = get_json(&app, &a, "/me").await;
    assert_eq!(
        delete(
            &app,
            &a,
            &format!("/channels/{}/members/{}", small.id, me.id)
        )
        .await,
        204
    );
    let left: Option<String> = sqlx::query_scalar("SELECT id FROM channels WHERE id = ?")
        .bind(small.id.to_string())
        .fetch_optional(&app.db)
        .await
        .unwrap();
    assert!(left.is_none(), "last person out deletes the group");
}

async fn put(app: &TestApp, token: &str, path: &str, body: serde_json::Value) -> u16 {
    app.http
        .put(app.url(path))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

#[tokio::test]
async fn mutes_are_private_and_expire() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    assert_eq!(
        put(
            &app,
            &b,
            "/mutes",
            serde_json::json!({"target_kind":"channel","target_id": dm.id,"until": null})
        )
        .await,
        204
    );
    assert_eq!(put(&app, &b, "/mutes", serde_json::json!({"target_kind":"server","target_id":"01J00000000000000000000000","until": "2000-01-01T00:00:00Z"})).await, 404, "unknown target");
    let (_ws, ready) = hello(&app, &b).await;
    assert_eq!(ready.mutes.len(), 1);
    let (_ws2, ready_a) = hello(&app, &a).await;
    assert!(ready_a.mutes.is_empty(), "alex never sees sam's mutes");
    assert_eq!(
        app.http
            .delete(app.url(&format!("/mutes/channel/{}", dm.id)))
            .bearer_auth(&b)
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    let (_ws3, ready) = hello(&app, &b).await;
    assert!(ready.mutes.is_empty());
}

#[tokio::test]
async fn closed_conversation_reappears_on_new_message() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    assert_eq!(
        post_json(
            &app,
            &b,
            &format!("/channels/{}/close", dm.id),
            serde_json::json!({})
        )
        .await
        .status(),
        204
    );
    let (_ws, ready) = hello(&app, &b).await;
    assert_eq!(ready.hidden, vec![dm.id]);
    send(&app, &a, dm.id, "you there?").await;
    let (_ws2, ready) = hello(&app, &b).await;
    assert!(ready.hidden.is_empty());
}

#[tokio::test]
async fn last_leave_deletes_a_group_with_replies() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let (c_id, c) = register(&app, "jo").await;
    let g = create_dm(&app, &a, &[b_id, c_id]).await;
    let first = send(&app, &a, g.id, "first").await;
    let r = post_json(
        &app,
        &b,
        &format!("/channels/{}/messages", g.id),
        serde_json::json!({"content": "reply", "reply_to_id": first.id, "nonce": null}),
    )
    .await;
    assert_eq!(r.status(), 200);
    for (id, tok) in [(b_id, &b), (c_id, &c), (a_id, &a)] {
        assert_eq!(
            delete(&app, tok, &format!("/channels/{}/members/{}", g.id, id)).await,
            204
        );
    }
    let left: Option<String> = sqlx::query_scalar("SELECT id FROM channels WHERE id = ?")
        .bind(g.id.to_string())
        .fetch_optional(&app.db)
        .await
        .unwrap();
    assert!(left.is_none());
}

#[tokio::test]
async fn logout_with_two_sockets_announces_offline_once() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (_, b) = register(&app, "sam").await;
    let (mut watcher, _) = hello(&app, &b).await;
    let (_ws1, _) = hello(&app, &a).await;
    let (_ws2, _) = hello(&app, &a).await;
    wait_for(
        &mut watcher,
        |e| matches!(e, Event::PresenceChanged { user_id, online: true, .. } if *user_id == a_id),
    )
    .await;
    let r = app
        .http
        .post(app.url("/auth/logout"))
        .bearer_auth(&a)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    // Collect alex's presence events for a while: exactly one "offline", nothing after it.
    let mut seen = Vec::new();
    while let Ok(Some(f)) =
        tokio::time::timeout(Duration::from_millis(600), next_frame(&mut watcher)).await
    {
        if let ServerFrame::Event(Event::PresenceChanged {
            user_id, online, ..
        }) = f
            && user_id == a_id
        {
            seen.push(online);
        }
    }
    assert_eq!(seen, vec![false]);
}

#[tokio::test]
async fn late_offline_after_a_reconnect_is_not_announced() {
    use pulse_server::gateway::hub::Hub;
    let app = spawn().await;
    let (a_id, _) = register(&app, "alex").await;
    let (b_id, _) = register(&app, "sam").await;
    let hub = Hub::default();
    let mut watcher = hub.register(b_id, "w".into());
    let old = hub.register(a_id, "t".into());
    assert_eq!(
        hub.sync_presence(&app.db, a_id, "x".into()).await,
        Some(true)
    );
    // The old socket drops, the client reconnects, and only then does the old socket's
    // disconnect handler get to announce: it must see the new connection and stay quiet.
    hub.unregister(old.id);
    let _new = hub.register(a_id, "t".into());
    assert_eq!(hub.sync_presence(&app.db, a_id, "x".into()).await, None);
    assert_eq!(hub.sync_presence(&app.db, a_id, "x".into()).await, None);
    let mut got = Vec::new();
    while let Ok(ServerFrame::Event(Event::PresenceChanged {
        user_id, online, ..
    })) = watcher.rx.try_recv()
    {
        if user_id == a_id {
            got.push(online);
        }
    }
    assert_eq!(got, vec![true], "sam still sees alex online");
}

#[tokio::test]
async fn concurrent_adds_never_exceed_the_group_cap() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let mut ids = Vec::new();
    for i in 0..12 {
        ids.push(register(&app, &format!("u{i}")).await.0);
    }
    let g = create_dm(&app, &a, &ids[0..2]).await; // 3 people
    let path = format!("/channels/{}/members", g.id);
    // Each add fits on its own (3 + 5 = 8); together they would make 13.
    for _ in 0..5 {
        let (r1, r2) = tokio::join!(
            post_json(
                &app,
                &a,
                &path,
                serde_json::json!({ "user_ids": &ids[2..7] })
            ),
            post_json(
                &app,
                &a,
                &path,
                serde_json::json!({ "user_ids": &ids[7..12] })
            ),
        );
        let n: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM channel_members WHERE channel_id = ?")
                .bind(g.id.to_string())
                .fetch_one(&app.db)
                .await
                .unwrap();
        assert!(
            n <= 10,
            "group grew to {n} ({} / {})",
            r1.status(),
            r2.status()
        );
        assert!(
            r1.status() == 204 || r2.status() == 204,
            "one of them still succeeds"
        );
        // reset to 3 for the next round
        sqlx::query("DELETE FROM channel_members WHERE channel_id = ? AND user_id NOT IN (SELECT id FROM users WHERE username IN ('alex','u0','u1'))")
            .bind(g.id.to_string())
            .execute(&app.db)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn removal_forgets_the_removed_members_mute() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let (c_id, _) = register(&app, "jo").await;
    let g = create_dm(&app, &a, &[b_id, c_id]).await;
    let mute = serde_json::json!({"target_kind":"channel","target_id": g.id,"until": null});
    assert_eq!(put(&app, &b, "/mutes", mute).await, 204);
    let (mut ws, _) = hello(&app, &b).await;
    assert_eq!(
        delete(&app, &a, &format!("/channels/{}/members/{}", g.id, b_id)).await,
        204
    );
    // sam's client is told the mute is gone, not just the channel
    wait_for(
        &mut ws,
        |e| matches!(e, Event::MutesChanged { mutes, .. } if mutes.is_empty()),
    )
    .await;
    let r = post_json(
        &app,
        &a,
        &format!("/channels/{}/members", g.id),
        serde_json::json!({ "user_ids": [b_id] }),
    )
    .await;
    assert_eq!(r.status(), 204);
    let (_, ready) = hello(&app, &b).await;
    assert!(ready.mutes.is_empty(), "re-added sam starts unmuted");
}

#[tokio::test]
async fn a_reply_mentions_the_author_it_replies_to() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let (c_id, c) = register(&app, "jo").await;
    let g = create_dm(&app, &a, &[b_id, c_id]).await;
    let reply = |token: &str, to: pulse_protocol::ids::MessageId, text: &str| {
        let (token, text) = (token.to_string(), text.to_string());
        let path = format!("/channels/{}/messages", g.id);
        let app = &app;
        async move {
            let r = post_json(
                app,
                &token,
                &path,
                serde_json::json!({ "content": text, "reply_to_id": to }),
            )
            .await;
            assert_eq!(r.status(), 200);
            r.json::<pulse_protocol::rest::Message>().await.unwrap()
        }
    };
    let from_sam = send(&app, &b, g.id, "hi").await;
    assert_eq!(reply(&a, from_sam.id, "yo").await.mentions, vec![b_id]);
    // also @jo in the text: both, sam once
    let both = reply(&a, from_sam.id, "@jo @sam look").await;
    assert_eq!(both.mentions.len(), 2);
    assert!(both.mentions.contains(&b_id) && both.mentions.contains(&c_id));
    // replying to yourself pings nobody
    assert!(reply(&b, from_sam.id, "me again").await.mentions.is_empty());
    // jo was removed: a reply to her old message doesn't ping her
    let from_jo = send(&app, &c, g.id, "bye").await;
    assert_eq!(
        delete(&app, &a, &format!("/channels/{}/members/{}", g.id, c_id)).await,
        204
    );
    assert!(reply(&a, from_jo.id, "she left").await.mentions.is_empty());
}
