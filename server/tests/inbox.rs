//! The inbox (#306): mentions and replies to you, across every server, DM and group.
mod common;

use std::time::Duration;

use common::*;
use futures::SinkExt;
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_protocol::ids::{ChannelId, MessageId, UserId};
use pulse_protocol::rest::{Channel, InboxKind, InboxPage, InboxUnread, Message};
use serde_json::json;
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
    next_frame_beating(ws, Duration::from_secs(3)).await
}

/// The next inbox event (skips other frames); None when none comes within 3 s.
async fn next_inbox(ws: &mut Socket) -> Option<Event> {
    loop {
        match next_frame(ws).await? {
            ServerFrame::Event(e @ (Event::InboxItemAdded { .. } | Event::InboxUpdated { .. })) => {
                return Some(e);
            }
            _ => continue,
        }
    }
}

struct Fx {
    app: TestApp,
    alex: UserId,
    alex_t: String,
    sam: UserId,
    sam_t: String,
    kim: UserId,
    kim_t: String,
    ch: Channel,
}

/// alex, sam and kim in one server; its #general.
async fn fx() -> Fx {
    let app = spawn().await;
    let (alex, alex_t) = register(&app, "alex").await;
    let (sam, sam_t) = register(&app, "sam").await;
    let (kim, kim_t) = register(&app, "kim").await;
    let s = create_server(&app, &alex_t, "Main").await;
    for t in [&sam_t, &kim_t] {
        let r = post_json(&app, t, &format!("/servers/{}/join", s.id), json!({})).await;
        assert_eq!(r.status(), 204);
    }
    let ch = general(&app, &alex_t, s.id).await;
    Fx {
        app,
        alex,
        alex_t,
        sam,
        sam_t,
        kim,
        kim_t,
        ch,
    }
}

impl Fx {
    async fn inbox(&self, token: &str, query: &str) -> InboxPage {
        get_json(&self.app, token, &format!("/inbox{query}")).await
    }
    async fn unread(&self, token: &str) -> u32 {
        get_json::<InboxUnread>(&self.app, token, "/inbox/unread")
            .await
            .unread
    }
    async fn reply(&self, token: &str, ch: ChannelId, to: MessageId, text: &str) -> Message {
        let r = post_json(
            &self.app,
            token,
            &format!("/channels/{ch}/messages"),
            json!({ "content": text, "reply_to_id": to }),
        )
        .await;
        assert_eq!(r.status(), 200, "reply");
        r.json().await.unwrap()
    }
    async fn read_one(&self, token: &str, id: MessageId) -> u16 {
        post_json(&self.app, token, &format!("/inbox/{id}/read"), json!({}))
            .await
            .status()
            .as_u16()
    }
    async fn read_all(&self, token: &str, upto: Option<MessageId>) -> u16 {
        post_json(&self.app, token, "/inbox/read-all", json!({ "upto": upto }))
            .await
            .status()
            .as_u16()
    }
    async fn mark_read(&self, token: &str, ch: ChannelId, id: MessageId) {
        let r = post_json(
            &self.app,
            token,
            &format!("/channels/{ch}/read"),
            json!({ "message_id": id }),
        )
        .await;
        assert_eq!(r.status(), 204, "mark read");
    }
    async fn delete(&self, token: &str, id: MessageId) {
        let r = self
            .app
            .http
            .delete(self.app.url(&format!("/messages/{id}")))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 204, "delete");
    }
    async fn mute_channel(&self, token: &str, ch: ChannelId) {
        let r = self
            .app
            .http
            .put(self.app.url("/mutes"))
            .bearer_auth(token)
            .json(&json!({"target_kind": "channel", "target_id": ch, "until": null}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 204, "mute");
    }
}

fn ids(p: &InboxPage) -> Vec<MessageId> {
    p.items.iter().map(|i| i.message.id).collect()
}

#[tokio::test]
async fn mentions_and_replies_to_you_are_items_newest_first() {
    let f = fx().await;
    // (yours first: sending moves your read point past what's before it)
    let mine = send(&f.app, &f.alex_t, f.ch.id, "my take").await;
    let mention = send(&f.app, &f.sam_t, f.ch.id, "hey @alex look").await;
    let reply = f.reply(&f.sam_t, f.ch.id, mine.id, "agreed").await;
    send(&f.app, &f.sam_t, f.ch.id, "plain chatter").await;

    let page = f.inbox(&f.alex_t, "").await;
    assert_eq!(ids(&page), vec![reply.id, mention.id]);
    assert_eq!(page.unread, 2);
    let r = &page.items[0];
    assert_eq!(r.kind, InboxKind::Reply);
    assert!(!r.read);
    let snip = r.replied_to.as_ref().expect("the snippet it replied to");
    assert_eq!(
        (
            snip.message_id,
            snip.author_id,
            snip.content.as_str(),
            snip.deleted
        ),
        (mine.id, Some(f.alex), "my take", false)
    );
    assert_eq!(r.message.author_id, Some(f.sam));
    assert_eq!(r.message.content, "agreed");
    assert_eq!(page.items[1].kind, InboxKind::Mention);
    assert!(page.items[1].replied_to.is_none());

    // The Mentions tab holds only typed mentions.
    let mentions = f.inbox(&f.alex_t, "?mentions=true").await;
    assert_eq!(ids(&mentions), vec![mention.id]);
    assert_eq!(mentions.unread, 2, "the count is the whole inbox's");

    // Nobody else's inbox has them; your own messages are never yours to be told about.
    assert!(f.inbox(&f.sam_t, "").await.items.is_empty());
    assert!(f.inbox(&f.kim_t, "").await.items.is_empty());
    assert_eq!(f.unread(&f.alex_t).await, 2);
}

#[tokio::test]
async fn a_reply_that_also_mentions_you_is_a_mention_with_its_snippet() {
    let f = fx().await;
    let mine = send(&f.app, &f.alex_t, f.ch.id, "base").await;
    let both = f.reply(&f.sam_t, f.ch.id, mine.id, "@alex yes").await;
    let page = f.inbox(&f.alex_t, "?mentions=true").await;
    assert_eq!(ids(&page), vec![both.id]);
    assert_eq!(page.items[0].kind, InboxKind::Mention);
    assert!(page.items[0].replied_to.is_some());
}

#[tokio::test]
async fn reading_the_channel_reads_its_items() {
    let f = fx().await;
    let a = send(&f.app, &f.sam_t, f.ch.id, "@alex one").await;
    let b = send(&f.app, &f.sam_t, f.ch.id, "@alex two").await;
    f.mark_read(&f.alex_t, f.ch.id, a.id).await;
    let page = f.inbox(&f.alex_t, "").await;
    assert_eq!(page.unread, 1);
    let read: Vec<bool> = page.items.iter().map(|i| i.read).collect();
    assert_eq!(ids(&page), vec![b.id, a.id]);
    assert_eq!(read, vec![false, true]);
    // Sending a message moves your read point past everything before it, too.
    send(&f.app, &f.alex_t, f.ch.id, "caught up").await;
    assert_eq!(f.unread(&f.alex_t).await, 0);
}

#[tokio::test]
async fn opening_an_item_reads_only_it_and_only_yours() {
    let f = fx().await;
    let a = send(&f.app, &f.sam_t, f.ch.id, "@alex one").await;
    let b = send(&f.app, &f.sam_t, f.ch.id, "@alex two").await;
    assert_eq!(f.read_one(&f.alex_t, a.id).await, 204);
    assert_eq!(f.read_one(&f.alex_t, a.id).await, 204, "again is a no-op");
    let page = f.inbox(&f.alex_t, "").await;
    assert_eq!(page.unread, 1);
    assert_eq!(
        page.items
            .iter()
            .map(|i| (i.message.id, i.read))
            .collect::<Vec<_>>(),
        vec![(b.id, false), (a.id, true)]
    );
    // Not in your inbox: 404, as if it didn't exist.
    assert_eq!(f.read_one(&f.kim_t, b.id).await, 404);
    assert_eq!(f.read_one(&f.alex_t, MessageId::new()).await, 404);
    assert_eq!(f.unread(&f.alex_t).await, 1);
}

#[tokio::test]
async fn mark_all_read_reads_what_was_there_and_new_items_stay_unread() {
    let f = fx().await;
    let a = send(&f.app, &f.sam_t, f.ch.id, "@alex one").await;
    let b = send(&f.app, &f.sam_t, f.ch.id, "@alex two").await;
    // Up to the newest you were shown: one that came after it stays unread.
    assert_eq!(f.read_all(&f.alex_t, Some(a.id)).await, 204);
    assert_eq!(f.unread(&f.alex_t).await, 1);
    assert_eq!(f.read_all(&f.alex_t, None).await, 204);
    let page = f.inbox(&f.alex_t, "").await;
    assert_eq!(page.unread, 0);
    assert!(page.items.iter().all(|i| i.read));
    assert_eq!(ids(&page), vec![b.id, a.id], "read items stay listed");
    // An older `upto` never takes it back.
    assert_eq!(f.read_all(&f.alex_t, Some(a.id)).await, 204);
    assert_eq!(f.unread(&f.alex_t).await, 0);
    send(&f.app, &f.sam_t, f.ch.id, "@alex three").await;
    assert_eq!(f.unread(&f.alex_t).await, 1);
}

#[tokio::test]
async fn mentions_in_a_quotes_thread_are_items_read_by_its_seen_point() {
    let f = fx().await;
    let r = post_json(
        &f.app,
        &f.alex_t,
        &format!("/servers/{}/channels", f.ch.server_id.unwrap()),
        json!({ "kind": "quotes", "name": "quotes" }),
    )
    .await;
    let quotes: Channel = r.json().await.unwrap();
    let r = post_json(
        &f.app,
        &f.alex_t,
        &format!("/channels/{}/quotes", quotes.id),
        json!({ "lines": [{ "speaker_id": f.sam, "text": "x" }] }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let quote: Message = r.json().await.unwrap();
    let r = post_json(
        &f.app,
        &f.sam_t,
        &format!("/channels/{}/messages", quotes.id),
        json!({ "content": "@alex look", "thread_root_id": quote.id }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let reply: Message = r.json().await.unwrap();
    let page = f.inbox(&f.alex_t, "").await;
    assert_eq!(ids(&page), vec![reply.id]);
    assert_eq!(page.items[0].message.thread_root_id, Some(quote.id));
    assert_eq!(page.unread, 1);
    let r = post_json(
        &f.app,
        &f.alex_t,
        &format!("/messages/{}/replies/seen", quote.id),
        json!({ "message_id": reply.id }),
    )
    .await;
    assert_eq!(r.status(), 204);
    assert_eq!(f.unread(&f.alex_t).await, 0);
    assert!(f.inbox(&f.alex_t, "").await.items[0].read);
}

#[tokio::test]
async fn a_muted_chat_adds_mentions_but_not_plain_replies() {
    let f = fx().await;
    let mine = send(&f.app, &f.alex_t, f.ch.id, "base").await;
    f.mute_channel(&f.alex_t, f.ch.id).await;
    f.reply(&f.sam_t, f.ch.id, mine.id, "a plain reply").await;
    let mention = send(&f.app, &f.sam_t, f.ch.id, "@alex still you").await;
    let page = f.inbox(&f.alex_t, "").await;
    assert_eq!(ids(&page), vec![mention.id]);
    assert_eq!(page.unread, 1);
}

#[tokio::test]
async fn deleted_messages_leave_the_inbox() {
    let f = fx().await;
    let a = send(&f.app, &f.sam_t, f.ch.id, "@alex one").await;
    let b = send(&f.app, &f.sam_t, f.ch.id, "@alex two").await;
    f.delete(&f.sam_t, a.id).await;
    let page = f.inbox(&f.alex_t, "").await;
    assert_eq!(ids(&page), vec![b.id]);
    assert_eq!(page.unread, 1);
}

#[tokio::test]
async fn a_conversation_you_left_leaves_the_inbox() {
    let f = fx().await;
    let group = create_dm(&f.app, &f.sam_t, &[f.alex, f.kim]).await;
    let m = send(&f.app, &f.sam_t, group.id, "@alex in the group").await;
    let dm = create_dm(&f.app, &f.sam_t, &[f.alex]).await;
    let d = send(&f.app, &f.sam_t, dm.id, "@alex in the dm").await;
    assert_eq!(ids(&f.inbox(&f.alex_t, "").await), vec![d.id, m.id]);
    let r = f
        .app
        .http
        .delete(
            f.app
                .url(&format!("/channels/{}/members/{}", group.id, f.alex)),
        )
        .bearer_auth(&f.alex_t)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "leave: {}", r.status());
    assert_eq!(ids(&f.inbox(&f.alex_t, "").await), vec![d.id]);
    assert_eq!(f.unread(&f.alex_t).await, 1);
}

#[tokio::test]
async fn items_older_than_30_days_are_gone() {
    let f = fx().await;
    let old = MessageId::floor(
        (chrono::Utc::now() - chrono::Duration::days(31)).timestamp_millis() as u64,
    );
    sqlx::query("INSERT INTO messages (id, channel_id, author_id, content, created_at) VALUES (?, ?, ?, '@alex old', '2020-01-01T00:00:00.000Z')")
        .bind(old.to_string())
        .bind(f.ch.id.to_string())
        .bind(f.sam.to_string())
        .execute(&f.app.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO mentions (message_id, user_id) VALUES (?, ?)")
        .bind(old.to_string())
        .bind(f.alex.to_string())
        .execute(&f.app.db)
        .await
        .unwrap();
    // its read point starts before it, so only the age keeps it out
    sqlx::query("UPDATE read_states SET last_read_message_id = NULL WHERE user_id = ?")
        .bind(f.alex.to_string())
        .execute(&f.app.db)
        .await
        .unwrap();
    let fresh = send(&f.app, &f.sam_t, f.ch.id, "@alex new").await;
    let page = f.inbox(&f.alex_t, "").await;
    assert_eq!(ids(&page), vec![fresh.id]);
    assert_eq!(page.unread, 1);
}

#[tokio::test]
async fn pages_newest_first_with_a_before_cursor() {
    let f = fx().await;
    // sends are rate limited per user: write the rows directly
    let mut all = vec![];
    for i in 0..35 {
        let id = MessageId::new();
        sqlx::query("INSERT INTO messages (id, channel_id, author_id, content, created_at) VALUES (?, ?, ?, ?, ?)")
            .bind(id.to_string())
            .bind(f.ch.id.to_string())
            .bind(f.sam.to_string())
            .bind(format!("@alex {i}"))
            .bind(pulse_server::db::now())
            .execute(&f.app.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO mentions (message_id, user_id) VALUES (?, ?)")
            .bind(id.to_string())
            .bind(f.alex.to_string())
            .execute(&f.app.db)
            .await
            .unwrap();
        all.push(id);
    }
    all.reverse();
    let first = f.inbox(&f.alex_t, "").await;
    assert_eq!(ids(&first), all[..30].to_vec());
    assert_eq!(first.unread, 35);
    let next = f.inbox(&f.alex_t, &format!("?before={}", all[29])).await;
    assert_eq!(ids(&next), all[30..].to_vec());
    let few = f.inbox(&f.alex_t, "?limit=2").await;
    assert_eq!(ids(&few), all[..2].to_vec());
}

#[tokio::test]
async fn ready_carries_the_unread_count() {
    let f = fx().await;
    send(&f.app, &f.sam_t, f.ch.id, "@alex hi").await;
    let (_ws, ready) = hello(&f.app, &f.alex_t).await;
    assert_eq!(ready.inbox_unread, Some(1));
    let (_ws, ready) = hello(&f.app, &f.kim_t).await;
    assert_eq!(ready.inbox_unread, Some(0));
}

#[tokio::test]
async fn events_bring_new_items_and_keep_every_device_in_step() {
    let f = fx().await;
    let (mut ws, _) = hello(&f.app, &f.alex_t).await;
    let (mut kim_ws, _) = hello(&f.app, &f.kim_t).await;
    let m = send(&f.app, &f.sam_t, f.ch.id, "@alex hi").await;
    match next_inbox(&mut ws).await {
        Some(Event::InboxItemAdded {
            user_id,
            item,
            unread,
        }) => {
            assert_eq!((user_id, item.message.id, unread), (f.alex, m.id, 1));
            assert_eq!(item.kind, InboxKind::Mention);
            assert!(!item.read);
        }
        other => panic!("expected InboxItemAdded, got {other:?}"),
    }
    // Opened on another device: this one hears it.
    assert_eq!(f.read_one(&f.alex_t, m.id).await, 204);
    match next_inbox(&mut ws).await {
        Some(Event::InboxUpdated { unread, read, .. }) => {
            assert_eq!((unread, read), (0, vec![m.id]));
        }
        other => panic!("expected InboxUpdated, got {other:?}"),
    }
    // A second one, then mark all read.
    let n = send(&f.app, &f.sam_t, f.ch.id, "@alex again").await;
    assert!(matches!(
        next_inbox(&mut ws).await,
        Some(Event::InboxItemAdded { unread: 1, .. })
    ));
    assert_eq!(f.read_all(&f.alex_t, None).await, 204);
    match next_inbox(&mut ws).await {
        Some(Event::InboxUpdated {
            unread, read_upto, ..
        }) => {
            assert_eq!(unread, 0);
            assert!(read_upto.is_some_and(|u| u >= n.id));
        }
        other => panic!("expected InboxUpdated, got {other:?}"),
    }
    // A third, read in its channel: the count follows the read point.
    let o = send(&f.app, &f.sam_t, f.ch.id, "@alex once more").await;
    assert!(matches!(
        next_inbox(&mut ws).await,
        Some(Event::InboxItemAdded { unread: 1, .. })
    ));
    f.mark_read(&f.alex_t, f.ch.id, o.id).await;
    assert!(matches!(
        next_inbox(&mut ws).await,
        Some(Event::InboxUpdated { unread: 0, .. })
    ));
    // Deleted: it's taken back out.
    f.delete(&f.sam_t, o.id).await;
    match next_inbox(&mut ws).await {
        Some(Event::InboxUpdated {
            unread, removed, ..
        }) => assert_eq!((unread, removed), (0, vec![o.id])),
        other => panic!("expected InboxUpdated, got {other:?}"),
    }
    // kim heard none of it.
    assert!(next_inbox(&mut kim_ws).await.is_none());
}

#[tokio::test]
async fn a_muted_plain_reply_sends_no_item_event() {
    let f = fx().await;
    let mine = send(&f.app, &f.alex_t, f.ch.id, "base").await;
    f.mute_channel(&f.alex_t, f.ch.id).await;
    let (mut ws, _) = hello(&f.app, &f.alex_t).await;
    f.reply(&f.sam_t, f.ch.id, mine.id, "plain").await;
    assert!(
        !matches!(
            next_inbox(&mut ws).await,
            Some(Event::InboxItemAdded { .. })
        ),
        "no item for a muted plain reply"
    );
}

#[tokio::test]
async fn the_backfill_tells_old_reply_pings_from_typed_mentions() {
    let f = fx().await;
    let mine = send(&f.app, &f.alex_t, f.ch.id, "base").await;
    let plain = f.reply(&f.sam_t, f.ch.id, mine.id, "plain").await;
    let typed = f.reply(&f.sam_t, f.ch.id, mine.id, "@Alex typed").await;
    // as rows from before the column were: all 1
    sqlx::query("UPDATE mentions SET explicit = 1")
        .execute(&f.app.db)
        .await
        .unwrap();
    let sql = include_str!("../migrations/0036_inbox.sql");
    let start = sql.find("UPDATE mentions SET explicit = 0").unwrap();
    let end = start + sql[start..].find(");").unwrap() + 2;
    sqlx::query(&sql[start..end])
        .execute(&f.app.db)
        .await
        .unwrap();
    let explicit = |id: MessageId| {
        let db = f.app.db.clone();
        async move {
            sqlx::query_scalar::<_, i64>("SELECT explicit FROM mentions WHERE message_id = ?")
                .bind(id.to_string())
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };
    assert_eq!(explicit(plain.id).await, 0);
    assert_eq!(explicit(typed.id).await, 1);
}

#[tokio::test]
async fn muting_tells_your_devices_the_new_count_and_an_unchanged_read_tells_nothing() {
    let f = fx().await;
    let mine = send(&f.app, &f.alex_t, f.ch.id, "base").await;
    let plain = f.reply(&f.sam_t, f.ch.id, mine.id, "plain").await;
    let (mut ws, ready) = hello(&f.app, &f.alex_t).await;
    assert_eq!(ready.inbox_unread, Some(1));
    f.mute_channel(&f.alex_t, f.ch.id).await;
    assert!(matches!(
        next_inbox(&mut ws).await,
        Some(Event::InboxUpdated { unread: 0, .. })
    ));
    // reading past it changes nothing now (the item is hidden): no event
    f.mark_read(&f.alex_t, f.ch.id, plain.id).await;
    assert!(next_inbox(&mut ws).await.is_none());
}

impl Fx {
    async fn unmute_channel(&self, token: &str, ch: ChannelId) {
        let r = self
            .app
            .http
            .delete(self.app.url(&format!("/mutes/channel/{ch}")))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 204, "unmute");
    }
    async fn leave(&self, token: &str, ch: ChannelId, who: UserId) {
        let r = self
            .app
            .http
            .delete(self.app.url(&format!("/channels/{ch}/members/{who}")))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert!(r.status().is_success(), "leave: {}", r.status());
    }
    async fn new_channel(&self, kind: &str, name: &str) -> Channel {
        let r = post_json(
            &self.app,
            &self.alex_t,
            &format!("/servers/{}/channels", self.ch.server_id.unwrap()),
            json!({ "kind": kind, "name": name }),
        )
        .await;
        assert_eq!(r.status(), 200, "new channel");
        r.json().await.unwrap()
    }
}

/// #327: the inbox work runs after the publish, not inside it: a send that mentions someone completes while
/// their inbox is busy, and its item reaches them once it is free.

#[tokio::test]
async fn a_mention_is_sent_without_waiting_on_the_inbox() {
    let f = fx().await;
    let (mut ws, _) = hello(&f.app, &f.alex_t).await;
    let slot = f.app.hub.inbox_slot(f.alex).expect("alex is connected");
    let held = slot.lock().await;
    let m = tokio::time::timeout(
        Duration::from_secs(2),
        send(&f.app, &f.sam_t, f.ch.id, "@alex hi"),
    )
    .await
    .expect("the send isn't held up by the inbox");
    drop(held);
    match next_inbox(&mut ws).await {
        Some(Event::InboxItemAdded { item, unread, .. }) => {
            assert_eq!((item.message.id, unread), (m.id, 1));
        }
        other => panic!("expected InboxItemAdded, got {other:?}"),
    }
}

/// #327: deleting a quote takes all its replies with it; each person mentioned hears it once, with every
/// item it took.
#[tokio::test]
async fn deleting_a_quote_with_replies_tells_each_person_once() {
    let f = fx().await;
    let quotes = f.new_channel("quotes", "quotes").await;
    let r = post_json(
        &f.app,
        &f.alex_t,
        &format!("/channels/{}/quotes", quotes.id),
        json!({ "lines": [{ "speaker_id": f.sam, "text": "x" }] }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let quote: Message = r.json().await.unwrap();
    let mut replies = vec![];
    for n in 0..3 {
        let r = post_json(
            &f.app,
            &f.sam_t,
            &format!("/channels/{}/messages", quotes.id),
            json!({ "content": format!("@alex look {n}"), "thread_root_id": quote.id }),
        )
        .await;
        assert_eq!(r.status(), 200);
        replies.push(r.json::<Message>().await.unwrap().id);
    }
    let (mut ws, ready) = hello(&f.app, &f.alex_t).await;
    assert_eq!(ready.inbox_unread, Some(3));
    f.delete(&f.alex_t, quote.id).await;
    match next_inbox(&mut ws).await {
        Some(Event::InboxUpdated {
            unread,
            mut removed,
            ..
        }) => {
            removed.sort();
            assert_eq!((unread, removed), (0, replies));
        }
        other => panic!("expected one InboxUpdated, got {other:?}"),
    }
    assert!(next_inbox(&mut ws).await.is_none(), "only one");
}

/// #327: counts are read and sent under the person's own lock, so whatever order mutes and reads land in, the
/// last count a device hears is the true one.

#[tokio::test]
async fn the_last_count_a_device_hears_is_the_true_one() {
    let f = fx().await;
    let mine = send(&f.app, &f.alex_t, f.ch.id, "base").await;
    f.reply(&f.sam_t, f.ch.id, mine.id, "plain").await;
    let (mut ws, ready) = hello(&f.app, &f.alex_t).await;
    assert_eq!(ready.inbox_unread, Some(1));
    let slot = f.app.hub.inbox_slot(f.alex).unwrap();
    let held = slot.lock().await;
    // both land while the inbox is busy: the counts that follow are read after, from where they left it
    f.mute_channel(&f.alex_t, f.ch.id).await;
    f.unmute_channel(&f.alex_t, f.ch.id).await;
    drop(held);
    let mut last = 1;
    while let Some(e) = next_inbox(&mut ws).await {
        if let Event::InboxUpdated { unread, .. } = e {
            last = unread;
        }
    }
    assert_eq!(last, f.unread(&f.alex_t).await);
    assert_eq!(last, 1);
}

/// #327: a deleted (then restored) channel changes the count of whoever had items in it: they're told the fresh
/// count (a count read before the delete can't stick), and nobody else hears anything.
#[tokio::test]
async fn a_count_after_a_channel_goes_and_comes_back_is_told() {
    let f = fx().await;
    let two = f.new_channel("text", "two").await;
    send(&f.app, &f.sam_t, two.id, "@alex in two").await;
    let (mut ws, ready) = hello(&f.app, &f.alex_t).await;
    let (mut kim_ws, _) = hello(&f.app, &f.kim_t).await;
    assert_eq!(ready.inbox_unread, Some(1));
    let r = f
        .app
        .http
        .delete(f.app.url(&format!("/channels/{}", two.id)))
        .bearer_auth(&f.alex_t)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert!(
        matches!(
            next_inbox(&mut ws).await,
            Some(Event::InboxUpdated { unread: 0, .. })
        ),
        "the count without it is told"
    );
    let r = post_json(
        &f.app,
        &f.alex_t,
        &format!("/channels/{}/restore", two.id),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 200);
    assert!(
        matches!(
            next_inbox(&mut ws).await,
            Some(Event::InboxUpdated { unread: 1, .. })
        ),
        "the count it's back to is told"
    );
    assert!(
        next_inbox(&mut kim_ws).await.is_none(),
        "kim had nothing there"
    );
}

/// #327 review: deleting a server takes all its channels at once: whoever had items in them hears the new count
/// once.
#[tokio::test]
async fn deleting_a_server_tells_the_count_once() {
    let f = fx().await;
    let two = f.new_channel("text", "two").await;
    send(&f.app, &f.sam_t, f.ch.id, "@alex in general").await;
    send(&f.app, &f.sam_t, two.id, "@alex in two").await;
    let (mut ws, ready) = hello(&f.app, &f.alex_t).await;
    assert_eq!(ready.inbox_unread, Some(2));
    let r = f
        .app
        .http
        .delete(f.app.url(&format!("/servers/{}", f.ch.server_id.unwrap())))
        .bearer_auth(&f.alex_t)
        .json(&json!({ "name": "Main" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert!(matches!(
        next_inbox(&mut ws).await,
        Some(Event::InboxUpdated { unread: 0, .. })
    ));
    assert!(next_inbox(&mut ws).await.is_none(), "only once");
}

/// #327: likewise for a group you leave.
#[tokio::test]
async fn a_count_after_leaving_a_group_is_told() {
    let f = fx().await;
    let mine = send(&f.app, &f.alex_t, f.ch.id, "base").await;
    f.reply(&f.sam_t, f.ch.id, mine.id, "plain").await;
    f.mute_channel(&f.alex_t, f.ch.id).await;
    let group = create_dm(&f.app, &f.sam_t, &[f.alex, f.kim]).await;
    send(&f.app, &f.sam_t, group.id, "@alex in the group").await;
    let (mut ws, ready) = hello(&f.app, &f.alex_t).await;
    assert_eq!(ready.inbox_unread, Some(1));
    f.leave(&f.alex_t, group.id, f.alex).await;
    assert!(
        matches!(
            next_inbox(&mut ws).await,
            Some(Event::InboxUpdated { unread: 0, .. })
        ),
        "the count without the group is told"
    );
    f.unmute_channel(&f.alex_t, f.ch.id).await;
    assert!(
        matches!(
            next_inbox(&mut ws).await,
            Some(Event::InboxUpdated { unread: 1, .. })
        ),
        "the muted reply counts again, and that's told"
    );
}

/// #327: the hygiene prune of inbox reads (by message id) has an index to use.
#[tokio::test]
async fn inbox_reads_are_indexed_by_message() {
    let f = fx().await;
    let plan: Vec<(i64, i64, i64, String)> =
        sqlx::query_as("EXPLAIN QUERY PLAN DELETE FROM inbox_reads WHERE message_id < 'x'")
            .fetch_all(&f.app.db)
            .await
            .unwrap();
    assert!(
        plan.iter().any(|(_, _, _, d)| d.starts_with("SEARCH")),
        "{plan:?}"
    );
}
