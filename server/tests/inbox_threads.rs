//! The inbox's Threads (#326): quote and clip threads you started or replied in, with replies by others you haven't
//! seen; gone once you look (your seen point, or a reply of yours); never a muted chat's.
mod common;

use common::*;
use pulse_protocol::ids::{ChannelId, MessageId};
use pulse_protocol::rest::{Channel, InboxThreads, Message};
use serde_json::json;

struct Fx {
    app: TestApp,
    alex: String,
    sam: String,
    kim: String,
    jo: String,
    quotes: ChannelId,
    quote: Message,
}

/// alex, sam, kim and jo in one server with a quotes channel; alex posted a quote of sam's.
async fn fx() -> Fx {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (_, kim) = register(&app, "kim").await;
    let (_, jo) = register(&app, "jo").await;
    let s = create_server(&app, &alex, "Main").await;
    for t in [&sam, &kim, &jo] {
        let r = join_server(&app, t, s.id).await;
        assert_eq!(r.status(), 200);
    }
    let r = post_json(
        &app,
        &alex,
        &format!("/servers/{}/channels", s.id),
        json!({ "kind": "quotes", "name": "quotes" }),
    )
    .await;
    let quotes: Channel = r.json().await.unwrap();
    let r = post_json(
        &app,
        &alex,
        &format!("/channels/{}/quotes", quotes.id),
        json!({ "lines": [{ "speaker_id": sam_id, "text": "x" }] }),
    )
    .await;
    assert_eq!(r.status(), 200);
    Fx {
        app,
        alex,
        sam,
        kim,
        jo,
        quotes: quotes.id,
        quote: r.json().await.unwrap(),
    }
}

impl Fx {
    async fn reply(&self, token: &str, text: &str) -> Message {
        let r = post_json(
            &self.app,
            token,
            &format!("/channels/{}/messages", self.quotes),
            json!({ "content": text, "thread_root_id": self.quote.id }),
        )
        .await;
        assert_eq!(r.status(), 200);
        r.json().await.unwrap()
    }

    async fn threads(&self, token: &str) -> InboxThreads {
        get_json(&self.app, token, "/inbox/threads").await
    }

    async fn seen(&self, token: &str, upto: MessageId) {
        let r = post_json(
            &self.app,
            token,
            &format!("/messages/{}/replies/seen", self.quote.id),
            json!({ "message_id": upto }),
        )
        .await;
        assert_eq!(r.status(), 204);
    }
}

#[tokio::test]
async fn threads_you_started_or_replied_in_list_new_replies_until_you_look() {
    let f = fx().await;
    // nobody has replied: alex watches the thread they started, with nothing new
    let t = f.threads(&f.alex).await;
    assert_eq!(t.watching, vec![f.quote.id]);
    assert!(t.threads.is_empty());
    // kim isn't part of it yet
    assert!(f.threads(&f.kim).await.watching.is_empty());

    f.reply(&f.sam, "first").await;
    let kims = f.reply(&f.kim, "second").await;
    let t = f.threads(&f.alex).await;
    assert_eq!(t.threads.len(), 1);
    let th = &t.threads[0];
    assert_eq!(
        (th.new_replies, th.started, th.latest.id, th.channel_id),
        (2, true, kims.id, f.quotes)
    );
    assert_eq!(th.root.message_id, f.quote.id);
    // kim replied: she's part of it, but her own reply isn't new to her (nor is sam's, before it)
    let k = f.threads(&f.kim).await;
    assert_eq!(k.watching, vec![f.quote.id]);
    assert!(k.threads.is_empty());
    // sam's reply came before kim's: for sam, kim's is new; he only replied, he didn't start it
    let s = f.threads(&f.sam).await;
    assert_eq!((s.threads[0].new_replies, s.threads[0].started), (1, false));
    // jo isn't part of it
    assert!(f.threads(&f.jo).await.threads.is_empty());

    // alex looks: gone
    f.seen(&f.alex, kims.id).await;
    assert!(f.threads(&f.alex).await.threads.is_empty());
    // a later reply brings it back, counting only what's new
    f.reply(&f.jo, "third").await;
    assert_eq!(f.threads(&f.alex).await.threads[0].new_replies, 1);
    // replying counts as looking
    f.reply(&f.alex, "mine").await;
    assert!(f.threads(&f.alex).await.threads.is_empty());
}

#[tokio::test]
async fn a_muted_chat_or_a_deleted_root_lists_nothing() {
    let f = fx().await;
    f.reply(&f.sam, "hi").await;
    assert_eq!(f.threads(&f.alex).await.threads.len(), 1);
    let r = f
        .app
        .http
        .put(f.app.url("/mutes"))
        .bearer_auth(&f.alex)
        .json(&json!({ "target_kind": "channel", "target_id": f.quotes, "until": null }))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.status());
    let t = f.threads(&f.alex).await;
    assert!(t.threads.is_empty() && t.watching.is_empty());
    // the quote deleted: sam (who replied) watches nothing
    let r = f
        .app
        .http
        .delete(f.app.url(&format!("/messages/{}", f.quote.id)))
        .bearer_auth(&f.alex)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.status());
    f.reply_none().await;
    assert!(f.threads(&f.sam).await.watching.is_empty());
}

impl Fx {
    /// Nothing: replies to a deleted quote are refused, which is the point.
    async fn reply_none(&self) {
        let r = post_json(
            &self.app,
            &self.kim,
            &format!("/channels/{}/messages", self.quotes),
            json!({ "content": "late", "thread_root_id": self.quote.id }),
        )
        .await;
        assert!(!r.status().is_success());
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// A connection past its Ready (inbox events reach it from then on), and the Ready's inbox count.
async fn connect(app: &TestApp, token: &str) -> (Socket, u64) {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as Ws;
    let mut ws = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap()
        .0;
    let hello = serde_json::to_string(&pulse_protocol::gateway::ClientFrame::Hello {
        token: token.into(),
        client_version: pulse_protocol::PROTOCOL_VERSION,
    })
    .unwrap();
    ws.send(Ws::text(hello)).await.unwrap();
    loop {
        let m = tokio::time::timeout(std::time::Duration::from_secs(3), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Ws::Text(t) = m {
            let v: serde_json::Value = serde_json::from_str(&t).unwrap();
            if v["op"] == "Ready" {
                return (ws, v["d"]["inbox_unread"].as_u64().unwrap());
            }
        }
    }
}

/// The count in the next `InboxUpdated` on `ws`; None when none comes within 3 s.
async fn next_count(ws: &mut Socket) -> Option<u64> {
    use futures::StreamExt;
    use tokio_tungstenite::tungstenite::Message as Ws;
    loop {
        let m = tokio::time::timeout(std::time::Duration::from_secs(3), ws.next())
            .await
            .ok()??
            .ok()?;
        if let Ws::Text(t) = m {
            let v: serde_json::Value = serde_json::from_str(&t).unwrap();
            if v["op"] == "Event" && v["d"]["t"] == "InboxUpdated" {
                return v["d"]["d"]["unread"].as_u64();
            }
        }
    }
}

async fn unread(app: &TestApp, token: &str) -> u32 {
    let n: pulse_protocol::rest::InboxUnread = get_json(app, token, "/inbox/unread").await;
    n.unread
}

#[tokio::test]
async fn threads_with_new_replies_count_in_the_inbox_and_their_people_hear_it() {
    let f = fx().await;
    assert_eq!(unread(&f.app, &f.alex).await, 0);
    let (mut ws, at_ready) = connect(&f.app, &f.alex).await;
    assert_eq!(at_ready, 0);

    // a reply in alex's quote's thread: one thread unread, and alex's devices hear the count
    f.reply(&f.sam, "first").await;
    assert_eq!(next_count(&mut ws).await, Some(1));
    // a second reply in the same thread is still one thread
    let kims = f.reply(&f.kim, "second").await;
    assert_eq!(unread(&f.app, &f.alex).await, 1);
    // jo isn't part of it: nothing counted for her
    assert_eq!(unread(&f.app, &f.jo).await, 0);
    let (_, ready) = connect(&f.app, &f.alex).await;
    assert_eq!(ready, 1);

    // alex looks: gone from the count, and told so
    f.seen(&f.alex, kims.id).await;
    assert_eq!(unread(&f.app, &f.alex).await, 0);
    let mut last = None;
    while let Some(n) = next_count(&mut ws).await {
        last = Some(n);
        if n == 0 {
            break;
        }
    }
    assert_eq!(last, Some(0));
}

/// alex, sam, kim and jo in one server with a builds channel; alex saved a build (#616).
async fn builds_fx() -> (Fx, ChannelId, Message) {
    let f = fx().await;
    let server_id: String = sqlx::query_scalar("SELECT server_id FROM channels WHERE id = ?")
        .bind(f.quotes.to_string())
        .fetch_one(&f.app.db)
        .await
        .unwrap();
    let r = post_json(
        &f.app,
        &f.alex,
        &format!("/servers/{server_id}/channels"),
        json!({ "kind": "builds", "name": "builds" }),
    )
    .await;
    let builds: Channel = r.json().await.unwrap();
    let r = post_json(
        &f.app,
        &f.alex,
        &format!("/channels/{}/builds", builds.id),
        json!({ "title": "Raven crit" }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let build: Message = r.json().await.unwrap();
    (f, builds.id, build)
}

async fn comment(f: &Fx, token: &str, channel: ChannelId, root: MessageId, text: &str) -> Message {
    let r = post_json(
        &f.app,
        token,
        &format!("/channels/{channel}/messages"),
        json!({ "content": text, "thread_root_id": root }),
    )
    .await;
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

#[tokio::test]
async fn a_comment_on_a_build_reaches_its_author_and_earlier_commenters() {
    let (f, ch, build) = builds_fx().await;
    let has = |t: &InboxThreads| t.threads.iter().any(|th| th.root.message_id == build.id);
    comment(&f, &f.sam, ch, build.id, "try the other module").await;
    assert!(has(&f.threads(&f.alex).await), "the author sees it");
    comment(&f, &f.kim, ch, build.id, "agreed").await;
    assert!(has(&f.threads(&f.alex).await));
    assert!(
        has(&f.threads(&f.sam).await),
        "an earlier commenter sees the next one"
    );
    assert!(
        !has(&f.threads(&f.jo).await),
        "someone who never took part doesn't"
    );
}

#[tokio::test]
async fn your_own_comment_doesnt_notify_you() {
    let (f, ch, build) = builds_fx().await;
    comment(&f, &f.sam, ch, build.id, "mine").await;
    let t = f.threads(&f.sam).await;
    assert!(t.watching.contains(&build.id), "sam takes part now");
    assert!(!t.threads.iter().any(|th| th.root.message_id == build.id));
}

#[tokio::test]
async fn comments_dont_mark_the_builds_channel_unread() {
    let (f, ch, build) = builds_fx().await;
    // jo has read the channel up to the build
    let r = post_json(
        &f.app,
        &f.jo,
        &format!("/channels/{ch}/read"),
        json!({ "message_id": build.id }),
    )
    .await;
    assert!(r.status().is_success(), "{}", r.status());
    comment(&f, &f.sam, ch, build.id, "a comment").await;
    let jo: String = sqlx::query_scalar("SELECT id FROM users WHERE username = 'jo'")
        .fetch_one(&f.app.db)
        .await
        .unwrap();
    let states = pulse_server::reads::states_for(&f.app.db, jo.parse().unwrap())
        .await
        .unwrap();
    let st = states.iter().find(|s| s.channel_id == ch).unwrap();
    assert_eq!(st.unread, 0);
}
