//! `POST /auth/logout-others`: sign out every other session of the caller.

mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::ClientFrame;
use pulse_protocol::rest::{LogoutOthersResponse, SessionResponse};
use pulse_server::auth::session;
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
            client_version: pulse_protocol::PROTOCOL_VERSION,
        })
        .unwrap(),
    ))
    .await
    .unwrap();
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

async fn login(app: &TestApp, username: &str) -> String {
    let r: SessionResponse = app
        .http
        .post(app.url("/auth/login"))
        .json(&json!({"username": username, "password": "hunter2hunter2"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    r.token
}

#[tokio::test]
async fn logout_others_revokes_only_the_callers_other_sessions() {
    let app = spawn().await;
    let (a_id, a1) = register(&app, "alex").await;
    let a2 = login(&app, "alex").await;
    let a3 = login(&app, "alex").await;
    let (s_id, s) = register(&app, "sam").await;

    let mut ws_caller = hello(&app, &a1).await;
    let mut ws_other = hello(&app, &a2).await;
    let mut ws_sam = hello(&app, &s).await;

    let r = app
        .http
        .post(app.url("/auth/logout-others"))
        .bearer_auth(&a1)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body: LogoutOthersResponse = r.json().await.unwrap();
    assert_eq!(body.revoked, 2);

    // Only the caller's session still authenticates; sam is untouched.
    assert_eq!(
        session::authenticate(&app.db, &a1).await.unwrap(),
        Some(a_id)
    );
    assert_eq!(session::authenticate(&app.db, &a2).await.unwrap(), None);
    assert_eq!(session::authenticate(&app.db, &a3).await.unwrap(), None);
    assert_eq!(
        session::authenticate(&app.db, &s).await.unwrap(),
        Some(s_id)
    );

    // The other device is kicked to the login screen.
    assert_eq!(
        close_code(&mut ws_other, Duration::from_secs(2)).await,
        Some(CloseCode::Library(4001))
    );
    // The caller's and sam's sockets stay open.
    assert_eq!(
        close_code(&mut ws_caller, Duration::from_millis(300)).await,
        None
    );
    assert_eq!(
        close_code(&mut ws_sam, Duration::from_millis(100)).await,
        None
    );
    assert_eq!(app.hub.connections_for(a_id), 1);
    assert_eq!(app.hub.connections_for(s_id), 1);

    // The caller can still use the API.
    let me = app
        .http
        .get(app.url("/me"))
        .bearer_auth(&a1)
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), 200);
}

#[tokio::test]
async fn logout_others_with_no_other_sessions_revokes_zero() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let body: LogoutOthersResponse = app
        .http
        .post(app.url("/auth/logout-others"))
        .bearer_auth(&a)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body.revoked, 0);
    assert_eq!(
        session::authenticate(&app.db, &a).await.unwrap(),
        Some(a_id)
    );
}

#[tokio::test]
async fn logout_others_requires_auth() {
    let app = spawn().await;
    let (_, a) = register(&app, "alex").await;
    let none = app
        .http
        .post(app.url("/auth/logout-others"))
        .send()
        .await
        .unwrap();
    assert_eq!(none.status(), 401);
    let bad = app
        .http
        .post(app.url("/auth/logout-others"))
        .bearer_auth("garbage")
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 401);
    // Nothing was revoked.
    assert!(session::authenticate(&app.db, &a).await.unwrap().is_some());
}
