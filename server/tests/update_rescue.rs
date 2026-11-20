//! The straggler rescue: with it on, the gateway closes an app that sends no `X-Pulse-Version` (every app before
//! 0.2.17) with 4005 once it signs in, so it lands on its forced update screen. Never an app that says it isn't
//! Windows (no updater there), never an app that says its version, never while no Windows update is published (the
//! forced update screen would have nothing to install), and nothing at all with the switch off.
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, ServerFrame};
use tokio_tungstenite::tungstenite::Message as Ws;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const MANIFEST: &str = r#"{"version":"0.2.19","platforms":{"windows-x86_64":{"url":"https://example.invalid/updates/Pulse_0.2.19_x64-setup.exe","signature":"SIG"}}}"#;

/// A server with the rescue `rescue`, and its updates dir holding `manifest` as `latest.json` (`None`: nothing
/// published). The temp dir lives as long as the app.
async fn app_with(rescue: bool, manifest: Option<&str>) -> (TestApp, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    if let Some(m) = manifest {
        std::fs::write(dir.path().join("latest.json"), m).unwrap();
    }
    let mut cfg = test_config();
    cfg.update_rescue = rescue;
    cfg.updates_dir = dir.path().to_path_buf();
    (spawn_with(cfg).await, dir)
}

/// With a Windows update published.
async fn app(rescue: bool) -> (TestApp, tempfile::TempDir) {
    app_with(rescue, Some(MANIFEST)).await
}

/// How the gateway answers a Hello with `token` from an app sending these headers: `Ok(())` for Ready, or the close
/// code.
async fn answer(
    app: &TestApp,
    token: &str,
    headers: &[(&'static str, &'static str)],
) -> Result<(), u16> {
    let mut req = app.ws_url("/gateway").into_client_request().unwrap();
    for (k, v) in headers {
        req.headers_mut().insert(*k, v.parse().unwrap());
    }
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    let hello = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
        client_version: pulse_protocol::PROTOCOL_VERSION,
    })
    .unwrap();
    ws.send(Ws::text(hello)).await.unwrap();
    loop {
        match tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .expect("an answer")
        {
            Some(Ok(Ws::Text(t))) => {
                if matches!(serde_json::from_str(&t), Ok(ServerFrame::Ready(_))) {
                    return Ok(());
                }
            }
            Some(Ok(Ws::Close(Some(f)))) => return Err(u16::from(f.code)),
            Some(Ok(_)) => continue,
            other => panic!("expected Ready or a close frame, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn an_app_without_a_version_is_told_to_update() {
    let (app, _dir) = app(true).await;
    let (alex, token) = register(&app, "alex").await;
    assert_eq!(answer(&app, &token, &[]).await, Err(4005));
    // and one that says it's Windows but no version
    assert_eq!(
        answer(&app, &token, &[("X-Pulse-Platform", "windows")]).await,
        Err(4005)
    );
    // its connect is still noted (no version), so `versions` shows who's stuck
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let row: Option<(Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT connect_version, connected_at FROM app_versions WHERE user_id = ?",
        )
        .bind(alex.to_string())
        .fetch_optional(&app.db)
        .await
        .unwrap();
        if let Some((v, at)) = row {
            assert_eq!(v, None);
            assert!(at.is_some());
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the connect wasn't noted"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_signed_out_app_is_sent_to_login_first() {
    let (app, _dir) = app(true).await;
    assert_eq!(answer(&app, "not-a-token", &[]).await, Err(4001));
}

#[tokio::test]
async fn apps_that_say_their_version_connect() {
    let (app, _dir) = app(true).await;
    let (_, token) = register(&app, "sam").await;
    assert_eq!(
        answer(&app, &token, &[("X-Pulse-Version", "0.2.17")]).await,
        Ok(())
    );
    assert_eq!(
        answer(
            &app,
            &token,
            &[
                ("X-Pulse-Version", "0.2.18"),
                ("X-Pulse-Platform", "windows")
            ]
        )
        .await,
        Ok(())
    );
}

#[tokio::test]
async fn linux_and_macos_are_never_told_to_update() {
    let (app, _dir) = app(true).await;
    let (_, token) = register(&app, "kim").await;
    for platform in ["linux", "Linux", "macos"] {
        assert_eq!(
            answer(&app, &token, &[("X-Pulse-Platform", platform)]).await,
            Ok(()),
            "{platform}"
        );
    }
}

#[tokio::test]
async fn switched_off_nobody_is_told_to_update() {
    let (app, _dir) = app(false).await;
    let (_, token) = register(&app, "jo").await;
    assert_eq!(answer(&app, &token, &[]).await, Ok(()));
}

#[tokio::test]
async fn nobody_is_told_to_update_while_no_windows_update_is_published() {
    // nothing published, a manifest without a Windows installer, an unreadable one: let them in
    for manifest in [
        None,
        Some(r#"{"version":"0.2.19","platforms":{}}"#),
        Some("{nope"),
    ] {
        let (app, _dir) = app_with(true, manifest).await;
        let (_, token) = register(&app, "alex").await;
        assert_eq!(answer(&app, &token, &[]).await, Ok(()), "{manifest:?}");
    }
}

#[tokio::test]
async fn publishing_an_update_turns_the_rescue_on_for_the_next_connect() {
    let (app, dir) = app_with(true, None).await;
    let (_, token) = register(&app, "sam").await;
    assert_eq!(answer(&app, &token, &[]).await, Ok(()));
    std::fs::write(dir.path().join("latest.json"), MANIFEST).unwrap();
    assert_eq!(answer(&app, &token, &[]).await, Err(4005));
}
