//! Regression tests for the final-review findings (I-1..I-6, M-1).

mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event};
use pulse_protocol::ids::{ChannelId, MessageId};
use pulse_protocol::rest::{Message, MessageKind};
use serde_json::json;
use tokio_tungstenite::tungstenite::Message as Ws;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn hello(app: &TestApp, token: &str) -> Socket {
    let mut ws = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap()
        .0;
    ws.send(Ws::text(
        serde_json::to_string(&ClientFrame::Hello {
            token: token.into(),
        })
        .unwrap(),
    ))
    .await
    .unwrap();
    // swallow Ready
    loop {
        if let Some(Ok(Ws::Text(t))) = ws.next().await
            && t.contains("\"Ready\"")
        {
            return ws;
        }
    }
}

/// Read until a close frame (or timeout); returns its code.
async fn close_code(ws: &mut Socket, within: Duration) -> Option<CloseCode> {
    tokio::time::timeout(within, async {
        while let Some(m) = ws.next().await {
            match m {
                Ok(Ws::Close(f)) => return f.map(|f| f.code),
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
}

// I-1
#[tokio::test]
async fn logout_closes_that_sessions_gateway_socket() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let mut ws = hello(&app, &a).await;
    let r = app
        .http
        .post(app.url("/auth/logout"))
        .bearer_auth(&a)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(
        close_code(&mut ws, Duration::from_secs(2)).await,
        Some(CloseCode::Library(4001))
    );
    assert_eq!(app.hub.connections_for(a_id), 0);
}

#[tokio::test]
async fn logout_leaves_other_sessions_of_same_user_open() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let login: pulse_protocol::rest::SessionResponse = app
        .http
        .post(app.url("/auth/login"))
        .json(&json!({"username": "alex", "password": "hunter2hunter2"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let _ws1 = hello(&app, &a).await;
    let _ws2 = hello(&app, &login.token).await;
    app.http
        .post(app.url("/auth/logout"))
        .bearer_auth(&a)
        .send()
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(app.hub.connections_for(a_id), 1);
}

// I-2 backstop: a deleted row never returns content, whatever is stored.
#[tokio::test]
async fn deleted_message_never_returns_content() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let s = create_server(&app, &a, "Main").await;
    let g = general(&app, &a, s.id).await;
    let m = send(&app, &a, g.id, "gone").await;
    // simulate the lost race: deleted_at set but content rewritten afterwards
    sqlx::query("UPDATE messages SET deleted_at = '2026-01-01T00:00:00.000Z', content = 'resurrected' WHERE id = ?")
        .bind(m.id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    let list: Vec<Message> = get_json(&app, &a, &format!("/channels/{}/messages", g.id)).await;
    assert!(list[0].deleted);
    assert_eq!(list[0].content, "");
}

// I-3 / M-1: invite checked before username or hashing.
#[tokio::test]
async fn bad_invite_with_taken_username_is_404_not_409() {
    let app = spawn().await;
    register(&app, "alex").await;
    let r = app
        .http
        .post(app.url("/auth/register"))
        .json(&json!({"invite_code": "nope", "username": "alex", "password": "hunter2hunter2"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

// I-4: outbound traffic must not keep a silent client alive.
#[tokio::test]
async fn silent_client_times_out_despite_outbound_traffic() {
    let mut cfg = test_config();
    cfg.heartbeat_timeout = Duration::from_millis(400);
    let app = spawn_with(cfg).await;
    let (_, a) = register(&app, "alex").await;
    let s = create_server(&app, &a, "Main").await;
    let g = general(&app, &a, s.id).await;
    let mut ws = hello(&app, &a).await;
    let db = app.db.clone();
    let hub = app.hub.clone();
    let pump = tokio::spawn(async move {
        loop {
            let m = Message {
                id: MessageId::new(),
                channel_id: g.id,
                author_id: None,
                kind: MessageKind::Normal,
                content: "tick".into(),
                reply_to_id: None,
                created_at: "2026-10-01T00:00:00.000Z".into(),
                edited_at: None,
                deleted: false,
            };
            hub.publish(
                &db,
                Event::MessageCreated {
                    message: m,
                    nonce: None,
                },
            )
            .await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    let code = close_code(&mut ws, Duration::from_secs(3)).await;
    pump.abort();
    assert_eq!(code, Some(CloseCode::Library(4002)));
    let _ = ChannelId::new();
}

// I-6: restart reconciliation + left without in-memory state.
#[tokio::test]
async fn dangling_voice_sessions_closed_on_startup() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let s = create_server(&app, &a, "Main").await;
    let chans: Vec<pulse_protocol::rest::Channel> =
        get_json(&app, &a, &format!("/servers/{}/channels", s.id)).await;
    let lounge = chans
        .iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();
    sqlx::query("INSERT INTO voice_sessions (id, user_id, channel_id, joined_at) VALUES ('VS1', ?, ?, '2026-01-01T00:00:00.000Z')")
        .bind(a_id.to_string())
        .bind(lounge.id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    pulse_server::voice::reconcile_on_startup(&app.db)
        .await
        .unwrap();
    let open: i64 = sqlx::query_scalar("SELECT count(*) FROM voice_sessions WHERE left_at IS NULL")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(open, 0);
}
