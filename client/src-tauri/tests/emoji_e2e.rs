//! Task 11's dev check, headless: two accounts upload emoji, react and watch the counts live,
//! then one emoji is deleted, and the images are read through per-account pulse-media caches.
//! REST goes through the client core's own `Api`; events are read from raw gateway sockets.

use std::io::Cursor;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use image::codecs::gif::GifEncoder;
use image::{Delay, Frame, ImageFormat, Rgba, RgbaImage};
use pulse_client::api::Api;
use pulse_client::media;
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_protocol::ids::{MessageId, UserId};
use pulse_protocol::rest::{ChannelKind, ReactionSummary, SendMessageRequest};
use pulse_server::testing;
use tokio_tungstenite::tungstenite::Message as Ws;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn hello(app: &testing::TestApp, token: &str) -> (Socket, Ready) {
    let (mut ws, _) = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap();
    let f = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
        client_version: pulse_protocol::PROTOCOL_VERSION,
    })
    .unwrap();
    ws.send(Ws::text(f)).await.unwrap();
    match next_frame(&mut ws).await {
        ServerFrame::Ready(r) => (ws, r),
        other => panic!("expected Ready, got {other:?}"),
    }
}

/// The next frame that isn't presence noise.
async fn next_frame(ws: &mut Socket) -> ServerFrame {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .expect("no frame in 3 s")
            .expect("socket closed")
            .unwrap();
        if let Ws::Text(t) = m {
            let f: ServerFrame = serde_json::from_str(&t).unwrap();
            if matches!(
                f,
                ServerFrame::Event(Event::PresenceChanged { .. } | Event::UserCreated { .. })
            ) {
                continue;
            }
            return f;
        }
    }
}

/// The next event the filter picks out, skipping the rest.
async fn next_matching<T>(ws: &mut Socket, pick: impl Fn(Event) -> Option<T>) -> T {
    loop {
        if let ServerFrame::Event(e) = next_frame(ws).await
            && let Some(t) = pick(e)
        {
            return t;
        }
    }
}

async fn emoji_created(ws: &mut Socket) -> pulse_protocol::rest::CustomEmoji {
    next_matching(ws, |e| match e {
        Event::EmojiCreated { emoji } => Some(emoji),
        _ => None,
    })
    .await
}

async fn thumbs_count(ws: &mut Socket, msg: MessageId, who: UserId) -> u32 {
    next_matching(ws, |e| match e {
        Event::ReactionAdded {
            message_id,
            emoji,
            user_id,
            count,
            ..
        } if message_id == msg && emoji == "👍" && user_id == who => Some(count),
        _ => None,
    })
    .await
}

fn png() -> Vec<u8> {
    let img = RgbaImage::from_pixel(64, 32, Rgba([10, 120, 240, 255]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

fn gif3() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut out);
        for i in 0..3u8 {
            let img = RgbaImage::from_pixel(32, 32, Rgba([i * 80, 0, 255 - i * 80, 255]));
            let delay = Delay::from_numer_denom_ms(50, 1);
            enc.encode_frame(Frame::from_parts(img, 0, 0, delay))
                .unwrap();
        }
    }
    out
}

fn summary(emoji: &str, count: u32, me: bool) -> ReactionSummary {
    ReactionSummary {
        emoji: emoji.into(),
        count,
        me,
    }
}

#[tokio::test]
async fn emoji_reactions_and_media_end_to_end() {
    let app = testing::spawn().await;
    let api = Api::new(&format!("http://{}", app.addr));
    let (alex_id, alex) = testing::register(&app, "alex").await;
    let (sam_id, sam) = testing::register(&app, "sam").await;

    // 1. one server, a text channel, a message from alex
    let srv = api.create_server(&alex, "Main").await.unwrap();
    api.join_server(&sam, srv.id).await.unwrap();
    let chan = api
        .create_channel(&alex, srv.id, ChannelKind::Text, "emoji-test")
        .await
        .unwrap();
    let req = SendMessageRequest {
        content: "hello".into(),
        reply_to_id: None,
        nonce: None,
    };
    let msg = api.send_message(&alex, chan.id, &req).await.unwrap();

    let (mut alex_ws, _) = hello(&app, &alex).await;
    let (mut sam_ws, _) = hello(&app, &sam).await;

    // 2. a still and a 3-frame animation
    let still = api.upload_emoji_media(&alex, png()).await.unwrap();
    assert!(!still.animated);
    assert_eq!(still.still_hash, None);
    let anim = api.upload_emoji_media(&alex, gif3()).await.unwrap();
    assert!(anim.animated);
    assert_eq!(anim.frames, 3);
    assert!(anim.still_hash.is_some());
    let blue = api
        .create_emoji(&alex, srv.id, "blue", &still.hash)
        .await
        .unwrap();
    let spin = api
        .create_emoji(&alex, srv.id, "spin", &anim.hash)
        .await
        .unwrap();

    // 3. sam sees both appear live, and a fresh Ready lists them with their hashes
    assert_eq!(emoji_created(&mut sam_ws).await, blue);
    assert_eq!(emoji_created(&mut sam_ws).await, spin);
    let (_, ready) = hello(&app, &sam).await;
    for (e, hash) in [(&blue, &still.hash), (&spin, &anim.hash)] {
        let listed = ready.custom_emoji.iter().find(|c| c.id == e.id).unwrap();
        assert_eq!(listed.hash.as_ref(), Some(hash));
    }
    assert_eq!(
        ready
            .custom_emoji
            .iter()
            .find(|c| c.id == spin.id)
            .unwrap()
            .still_hash,
        anim.still_hash
    );

    // 4. both sockets see absolute counts 1 then 2
    let custom = format!("custom:{}", blue.id);
    api.add_reaction(&alex, msg.id, "👍").await.unwrap();
    api.add_reaction(&alex, msg.id, &custom).await.unwrap();
    api.add_reaction(&sam, msg.id, "👍").await.unwrap();
    for ws in [&mut alex_ws, &mut sam_ws] {
        assert_eq!(thumbs_count(ws, msg.id, alex_id).await, 1);
        assert_eq!(thumbs_count(ws, msg.id, sam_id).await, 2);
    }
    let want = |me_thumb, me_custom| [summary("👍", 2, me_thumb), summary(&custom, 1, me_custom)];
    let find = |msgs: Vec<pulse_protocol::rest::Message>| {
        msgs.into_iter().find(|m| m.id == msg.id).unwrap().reactions
    };
    assert_eq!(
        find(api.messages(&alex, chan.id, None).await.unwrap()),
        want(true, true)
    );
    assert_eq!(
        find(api.messages(&sam, chan.id, None).await.unwrap()),
        want(true, false)
    );

    // 5. deleting the emoji keeps its name and the reaction, drops the hash
    api.delete_emoji(&alex, &blue.id).await.unwrap();
    let (_, ready) = hello(&app, &sam).await;
    let gone = ready
        .custom_emoji
        .iter()
        .find(|c| c.id == blue.id)
        .expect("a deleted emoji stays listed");
    assert_eq!(gone.hash, None);
    assert_eq!(gone.name, "blue");
    assert_eq!(
        find(api.messages(&sam, chan.id, None).await.unwrap()),
        want(true, false)
    );

    // 6. the pulse-media path: each account fetches into its own cache once, then the cache
    // answers (even with the server gone); another account's cache never answers for it
    let base = format!("http://{}", app.addr);
    let root = tempfile::tempdir().unwrap();
    let offline = Api::new("http://localhost:9");
    let session = |api: &Api, token: &str, me: &UserId| {
        Some(media::Session {
            api: api.clone(),
            token: token.into(),
            cache: Some(media::scope_dir(root.path(), &base, me)),
        })
    };
    for hash in [&anim.hash, anim.still_hash.as_ref().unwrap()] {
        let path = format!("/{hash}");
        let fetched = media::serve(session(&api, &sam, &sam_id), &path)
            .await
            .unwrap();
        assert!(media::verify(hash, &fetched));
        let cached = media::serve(session(&offline, &sam, &sam_id), &path).await;
        assert_eq!(cached.as_deref(), Ok(&fetched[..]));
        assert_eq!(
            media::serve(session(&offline, &alex, &alex_id), &path).await,
            Err(tauri::http::StatusCode::BAD_GATEWAY),
            "alex's cache is not sam's"
        );
    }
}
