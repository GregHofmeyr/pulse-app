//! DMs, groups, unread, mentions, mutes, presence.
mod common;

use std::time::Duration;

use common::*;
use pulse_protocol::gateway::{Event, Ready, ServerFrame};
use pulse_server::gateway::audience::{Audience, audience_for};

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
        mentions: None,
        unread: None,
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
    acquaint(&app, &a, &[&b]).await; // they share a server: presence reaches them (#80)
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
async fn ready_lists_who_you_share_something_with_with_presence() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let (c_id, _) = register(&app, "kim").await;
    acquaint(&app, &a, &[&b]).await;
    let (_ws, ready) = hello(&app, &a).await;
    assert!(
        ready.people.iter().all(|p| p.user.id != c_id),
        "kim shares nothing (#486)"
    );
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
    assert_eq!(join_server(&app, &b, s.id).await.status(), 200);
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
    // presence and the profile re-sync from becoming acquainted (#486) may still be arriving: skip those
    let next = wait_for(&mut alex, |e| {
        !matches!(e, Event::PresenceChanged { .. } | Event::UserCreated { .. })
    })
    .await;
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
    let (c_id, c) = register(&app, "jo").await;
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
    let m2 = send(&app, &a, g.id, "@jo look").await;
    assert!(
        m2.mentions.is_empty(),
        "server channels: its members only (#371)"
    );
    let r = join_server(&app, &c, s.id).await;
    assert!(r.status().is_success());
    let m2 = send(&app, &a, g.id, "@jo @jo look").await;
    assert_eq!(m2.mentions, vec![c_id], "a member; deduped");
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
    acquaint(&app, &a, &[&b]).await; // they share a server: presence reaches them (#80)
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
    let (a_id, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    acquaint(&app, &a, &[&b]).await; // they share a server: presence reaches them (#80)
    let hub = Hub::default();
    let mut watcher = hub.register(b_id, "w".into());
    let old = hub.register(a_id, "t".into());
    assert_eq!(hub.sync_presence(&app.db, a_id).await, Some(true));
    // The old socket drops, the client reconnects, and only then does the old socket's
    // disconnect handler get to announce: it must see the new connection and stay quiet.
    hub.unregister(old.id);
    let _new = hub.register(a_id, "t".into());
    // (the new connection had no Ready here, so it alone hears alex's own presence once)
    hub.sync_presence(&app.db, a_id).await;
    assert_eq!(hub.sync_presence(&app.db, a_id).await, None);
    let mut got = Vec::new();
    // (alex's profile comes first, #486: the watcher had no Ready)
    while let Ok(f) = watcher.rx.try_recv() {
        if let ServerFrame::Event(Event::PresenceChanged {
            user_id, online, ..
        }) = f
            && user_id == a_id
        {
            got.push(online);
        }
    }
    assert_eq!(got, vec![true], "sam still sees alex online");
}

#[tokio::test]
async fn presence_synced_while_a_logout_closes_sockets_goes_offline_once() {
    use pulse_server::gateway::hub::Hub;
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    acquaint(&app, &a, &[&b]).await; // they share a server: presence reaches them (#80)
    let hub = Hub::default();
    let mut watcher = hub.register(b_id, "w".into());
    let one = hub.register(a_id, "t".into());
    let two = hub.register(a_id, "t".into());
    hub.sync_presence(&app.db, a_id).await;
    // A logout takes both of alex's connections out of the hub. Before either socket has closed, something else
    // announces alex (#449: the second socket's sync after its Ready, running late). Last-seen isn't written yet.
    hub.drop_session("t");
    assert_eq!(hub.sync_presence(&app.db, a_id).await, None);
    // The sockets close: the first goes quietly, the last records last-seen and announces.
    hub.unregister(one.id);
    assert!(hub.is_closing(a_id));
    assert_eq!(hub.sync_presence(&app.db, a_id).await, None);
    let seen = "2026-11-18T10:00:00.000Z";
    sqlx::query("UPDATE users SET last_seen_at = ? WHERE id = ?")
        .bind(seen)
        .bind(a_id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    hub.unregister(two.id);
    assert!(!hub.is_closing(a_id));
    hub.sync_presence(&app.db, a_id).await;
    let mut got = Vec::new();
    while let Ok(f) = watcher.rx.try_recv() {
        if let ServerFrame::Event(Event::PresenceChanged {
            user_id,
            online,
            last_seen_at,
            ..
        }) = f
            && user_id == a_id
        {
            got.push((online, last_seen_at));
        }
    }
    assert_eq!(
        got,
        vec![(true, None), (false, Some(seen.to_string()))],
        "offline once, with the last-seen the last socket wrote"
    );
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

#[tokio::test]
async fn read_state_events_carry_the_absolute_unread_count() {
    let app = spawn().await;
    let (_a_id, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    let first = send(&app, &a, dm.id, "one").await;
    send(&app, &a, dm.id, "two").await;
    send(&app, &a, dm.id, "three").await;
    let (mut sam_other_device, _) = hello(&app, &b).await;
    // Reading up to the first leaves two unread (a client holding an older slice can't count them).
    post_json(
        &app,
        &b,
        &format!("/channels/{}/read", dm.id),
        serde_json::json!({"message_id": first.id}),
    )
    .await;
    let e = wait_for(&mut sam_other_device, |e| {
        matches!(e, Event::ReadStateUpdated { .. })
    })
    .await;
    assert!(
        matches!(
            e,
            Event::ReadStateUpdated {
                unread: Some(2),
                ..
            }
        ),
        "{e:?}"
    );
    // Sending moves the point past everything: none left.
    send(&app, &b, dm.id, "caught up").await;
    let e = wait_for(&mut sam_other_device, |e| {
        matches!(e, Event::ReadStateUpdated { .. })
    })
    .await;
    assert!(
        matches!(
            e,
            Event::ReadStateUpdated {
                unread: Some(0),
                ..
            }
        ),
        "{e:?}"
    );
}

#[tokio::test]
async fn read_state_events_carry_the_absolute_mention_count() {
    let app = spawn().await;
    let (_a_id, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    let first = send(&app, &a, dm.id, "hey @sam").await;
    let second = send(&app, &a, dm.id, "again @sam").await;
    let (mut sam_other_device, _) = hello(&app, &b).await;
    // Reading up to the first leaves the second's mention.
    post_json(
        &app,
        &b,
        &format!("/channels/{}/read", dm.id),
        serde_json::json!({"message_id": first.id}),
    )
    .await;
    let e = wait_for(&mut sam_other_device, |e| {
        matches!(e, Event::ReadStateUpdated { .. })
    })
    .await;
    assert!(
        matches!(
            e,
            Event::ReadStateUpdated {
                mentions: Some(1),
                ..
            }
        ),
        "{e:?}"
    );
    post_json(
        &app,
        &b,
        &format!("/channels/{}/read", dm.id),
        serde_json::json!({"message_id": second.id}),
    )
    .await;
    let e = wait_for(&mut sam_other_device, |e| {
        matches!(e, Event::ReadStateUpdated { .. })
    })
    .await;
    assert!(
        matches!(
            e,
            Event::ReadStateUpdated {
                mentions: Some(0),
                ..
            }
        ),
        "{e:?}"
    );
}

/// Notes to self (#308): asking for a DM with only yourself opens your one-person DM, made on first
/// open and reused after; it's never someone else's DM, nobody else can see it, and it has no calls.
#[tokio::test]
async fn notes_to_self_is_a_one_person_dm() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let (b_id, b) = register(&app, "sam").await;
    let dm = create_dm(&app, &a, &[b_id]).await;
    let notes = create_dm(&app, &a, &[a_id]).await;
    assert_ne!(notes.id, dm.id, "not the DM with sam");
    assert_eq!(notes.kind, pulse_protocol::rest::ChannelKind::Dm);
    assert_eq!(create_dm(&app, &a, &[a_id]).await.id, notes.id, "reused");
    assert_eq!(
        create_dm(&app, &a, &[b_id]).await.id,
        dm.id,
        "the DM still resolves"
    );
    let sams_notes = create_dm(&app, &b, &[b_id]).await;
    assert_ne!(sams_notes.id, notes.id, "everyone has their own");

    let m = send(&app, &a, notes.id, "buy milk").await;
    let (_ws, ready) = hello(&app, &a).await;
    let members = ready
        .dm_members
        .iter()
        .find(|m| m.channel_id == notes.id)
        .expect("listed");
    assert_eq!(members.user_ids, vec![a_id]);
    assert_eq!(read_state(&ready, notes.id).await.unread, 0);
    let r = app
        .http
        .get(app.url(&format!("/channels/{}/messages", notes.id)))
        .bearer_auth(&b)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404, "only you");
    assert_eq!(history(&app, &a, notes.id).await[0].id, m.id);

    let call = post_json(
        &app,
        &a,
        &format!("/calls/{}", notes.id),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(call.status(), 400, "nobody to call");

    // Closed, it comes back on the next open.
    let close = post_json(
        &app,
        &a,
        &format!("/channels/{}/close", notes.id),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(close.status(), 204);
    assert_eq!(create_dm(&app, &a, &[a_id]).await.id, notes.id);
    let (_ws2, ready) = hello(&app, &a).await;
    assert!(ready.hidden.is_empty());
}
