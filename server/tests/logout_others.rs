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

/// Review fix: a session revoked between the gateway's authenticate and its register must not
/// end up with a live socket (the revoke's kick ran before the socket was in the hub). Simulated
/// deterministically: a trigger deletes the session row during authenticate's sliding-expiry
/// UPDATE, so authenticate still says "ok" but the row is gone before register.
#[tokio::test]
async fn session_revoked_between_auth_and_register_gets_no_socket() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    sqlx::query("UPDATE sessions SET last_used_at = '2000-01-01T00:00:00.000Z'")
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER revoke_mid_auth AFTER UPDATE OF last_used_at ON sessions
         BEGIN DELETE FROM sessions WHERE token_hash = NEW.token_hash; END",
    )
    .execute(&app.db)
    .await
    .unwrap();

    let mut ws = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap()
        .0;
    ws.send(Ws::text(
        serde_json::to_string(&ClientFrame::Hello {
            token: a.clone(),
            client_version: pulse_protocol::PROTOCOL_VERSION,
        })
        .unwrap(),
    ))
    .await
    .unwrap();
    let got = tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(m) = ws.next().await {
            match m {
                Ok(Ws::Text(t)) if t.contains("\"Ready\"") => return "ready".to_string(),
                Ok(Ws::Close(f)) => return format!("close {:?}", f.map(|f| f.code)),
                Ok(_) => continue,
                Err(e) => return format!("error {e}"),
            }
        }
        "eof".to_string()
    })
    .await
    .unwrap();
    assert_eq!(got, format!("close {:?}", Some(CloseCode::Library(4001))));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(app.hub.connections_for(a_id), 0);
}

/// #466: the owner tools (`reset-password`, `revoke-sessions`) run in their own process. The
/// running server's revoked-session sweeper closes those sessions' sockets (4001, so the app goes
/// to sign-in); everyone else's stay open.
#[tokio::test]
async fn an_owner_reset_or_revoke_closes_that_persons_sockets_at_the_next_sweep() {
    let app = spawn().await;
    let (a_id, a1) = register(&app, "alex").await;
    let a2 = login(&app, "alex").await;
    let (s_id, s) = register(&app, "sam").await;
    let mut ws_a1 = hello(&app, &a1).await;
    let mut ws_a2 = hello(&app, &a2).await;
    let mut ws_sam = hello(&app, &s).await;
    pulse_server::streams::ghosts::spawn_revoked_sweeper(app.state(), Duration::from_millis(20));

    // Only the database hears of it.
    pulse_server::auth::routes::reset_password(&app.db, "alex", "a brand new passphrase")
        .await
        .unwrap();
    for ws in [&mut ws_a1, &mut ws_a2] {
        assert_eq!(
            close_code(ws, Duration::from_secs(2)).await,
            Some(CloseCode::Library(4001))
        );
    }
    assert_eq!(app.hub.connections_for(a_id), 0);
    assert_eq!(
        close_code(&mut ws_sam, Duration::from_millis(200)).await,
        None
    );

    // `revoke-sessions` too.
    pulse_server::auth::routes::revoke_sessions(&app.db, "sam")
        .await
        .unwrap();
    assert_eq!(
        close_code(&mut ws_sam, Duration::from_secs(2)).await,
        Some(CloseCode::Library(4001))
    );
    assert_eq!(app.hub.connections_for(s_id), 0);
}

/// #466: a session that merely expires (and is pruned) keeps its socket: nobody is kicked by
/// expiry, only by a revoke.
#[tokio::test]
async fn an_expired_session_keeps_its_socket_through_the_sweep() {
    let app = spawn().await;
    let (a_id, a) = register(&app, "alex").await;
    let mut ws = hello(&app, &a).await;
    pulse_server::streams::ghosts::spawn_revoked_sweeper(app.state(), Duration::from_millis(20));
    sqlx::query("UPDATE sessions SET expires_at = '2000-01-01T00:00:00.000Z'")
        .execute(&app.db)
        .await
        .unwrap();
    pulse_server::hygiene::prune(&app.db, chrono::Utc::now())
        .await
        .unwrap();
    assert!(
        !session::exists(&app.db, &session::hash_token(&a))
            .await
            .unwrap()
    );
    assert_eq!(close_code(&mut ws, Duration::from_millis(300)).await, None);
    assert_eq!(app.hub.connections_for(a_id), 1);
}
