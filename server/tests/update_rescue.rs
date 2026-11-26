//! Apps before 0.2.20 and the forced update screen. The straggler rescue (an app that sends no `X-Pulse-Version`,
//! every app before 0.2.17, closed with 4005 once it signs in) is superseded since #634: every app before 0.2.20 speaks
//! protocol 4 or older and [`pulse_server::gateway::socket::MIN_CLIENT_VERSION`] 5 closes it with 4005, whatever its
//! headers, its platform, the rescue switch or the manifest. A signed-out one still gets 4001 (its login screen) first.
//! Each turned-away connect is noted in `app_versions`, so `versions` shows who is still on an old app.
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

/// The protocol every app before 0.2.20 speaks (0.2.17 to 0.2.19: 4; older ones less).
const OLD: u32 = 4;

/// How the gateway answers a Hello with `token` from an old app (protocol [`OLD`]) sending these headers: `Ok(())`
/// for Ready, or the close code.
async fn answer(
    app: &TestApp,
    token: &str,
    headers: &[(&'static str, &'static str)],
) -> Result<(), u16> {
    answer_as(app, token, OLD, headers).await
}

/// [`answer`] for a Hello that says protocol `version`.
async fn answer_as(
    app: &TestApp,
    token: &str,
    version: u32,
    headers: &[(&'static str, &'static str)],
) -> Result<(), u16> {
    let mut req = app.ws_url("/gateway").into_client_request().unwrap();
    for (k, v) in headers {
        req.headers_mut().insert(*k, v.parse().unwrap());
    }
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    let hello = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
        client_version: version,
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
    assert_eq!(noted(&app, alex).await, None);
}

/// The version `user`'s last gateway connect was noted with (waits for the note, written off the connect's path).
async fn noted(app: &TestApp, user: pulse_protocol::ids::UserId) -> Option<String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let row: Option<(Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT connect_version, connected_at FROM app_versions WHERE user_id = ?",
        )
        .bind(user.to_string())
        .fetch_optional(&app.db)
        .await
        .unwrap();
        if let Some((v, Some(_))) = row {
            return v;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the connect wasn't noted"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// #634: an outdated app that says its version is noted with it, so the rollout's stragglers show in `versions`.
#[tokio::test]
async fn an_outdated_app_s_connect_is_noted_with_its_version() {
    let (app, _dir) = app(true).await;
    let (sam, token) = register(&app, "sam").await;
    assert_eq!(
        answer(
            &app,
            &token,
            &[
                ("X-Pulse-Version", "0.2.19"),
                ("X-Pulse-Platform", "windows")
            ]
        )
        .await,
        Err(4005)
    );
    assert_eq!(noted(&app, sam).await.as_deref(), Some("0.2.19"));
}

/// Told to update with a dead session, its update screen would ask for the manifest with it, get 401 and wait for good.
#[tokio::test]
async fn a_signed_out_app_is_sent_to_login_first() {
    let (app, _dir) = app(true).await;
    assert_eq!(answer(&app, "not-a-token", &[]).await, Err(4001));
    // one whose session was revoked too
    let (_, token) = register(&app, "kim").await;
    let r = post_json(&app, &token, "/auth/logout", serde_json::json!({})).await;
    assert!(r.status().is_success(), "logout {}", r.status());
    assert_eq!(
        answer(&app, &token, &[("X-Pulse-Version", "0.2.19")]).await,
        Err(4001)
    );
}

/// 0.2.17 to 0.2.19 say their version, but speak protocol 4: told to update (#634). 0.2.20 connects.
#[tokio::test]
async fn old_apps_that_say_their_version_are_told_to_update_too() {
    let (app, _dir) = app(true).await;
    let (_, token) = register(&app, "sam").await;
    for version in ["0.2.17", "0.2.18", "0.2.19"] {
        assert_eq!(
            answer(
                &app,
                &token,
                &[
                    ("X-Pulse-Version", version),
                    ("X-Pulse-Platform", "windows")
                ]
            )
            .await,
            Err(4005),
            "{version}"
        );
    }
    assert_eq!(
        answer_as(
            &app,
            &token,
            pulse_protocol::PROTOCOL_VERSION,
            &[
                ("X-Pulse-Version", "0.2.20"),
                ("X-Pulse-Platform", "windows")
            ]
        )
        .await,
        Ok(())
    );
}

/// The rescue spared Linux and macOS, the rescue switch off, and a server with no Windows update published. The
/// protocol doesn't: an old app is told to update in each (every tester is on Windows, and 0.2.20 is published).
#[tokio::test]
async fn protocol_4_is_told_to_update_whatever_the_rescue_would_have_spared() {
    let (app, _dir) = app(true).await;
    let (_, token) = register(&app, "kim").await;
    for platform in ["linux", "Linux", "macos"] {
        assert_eq!(
            answer(&app, &token, &[("X-Pulse-Platform", platform)]).await,
            Err(4005),
            "{platform}"
        );
    }
    let (app, _dir) = app_with(false, Some(MANIFEST)).await;
    let (_, token) = register(&app, "jo").await;
    assert_eq!(answer(&app, &token, &[]).await, Err(4005), "rescue off");
    for manifest in [
        None,
        Some(r#"{"version":"0.2.19","platforms":{}}"#),
        Some("{nope"),
    ] {
        let (app, _dir) = app_with(true, manifest).await;
        let (_, token) = register(&app, "alex").await;
        assert_eq!(answer(&app, &token, &[]).await, Err(4005), "{manifest:?}");
    }
}

/// The rescue's own check, which only a protocol-5 Hello without the version header can still reach (no real app
/// sends that): it waits for a Windows update to be published. Goes with the rescue.
#[tokio::test]
async fn publishing_an_update_turns_the_rescue_on_for_the_next_connect() {
    let (app, dir) = app_with(true, None).await;
    let (_, token) = register(&app, "sam").await;
    let v = pulse_protocol::PROTOCOL_VERSION;
    assert_eq!(answer_as(&app, &token, v, &[]).await, Ok(()));
    std::fs::write(dir.path().join("latest.json"), MANIFEST).unwrap();
    assert_eq!(answer_as(&app, &token, v, &[]).await, Err(4005));
}
