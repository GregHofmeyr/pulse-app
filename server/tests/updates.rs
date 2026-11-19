//! App updates (#82): `latest.json` and the signed installer, only for signed-in people.
mod common;

use common::*;
use pulse_server::updates::installer_name_ok;

/// The updates dir the test fills: `updates/` inside a temp dir (so its parent, which a `..` would reach, is the
/// test's own too).
struct UpdatesDir {
    _root: tempfile::TempDir,
    updates: std::path::PathBuf,
}

impl UpdatesDir {
    fn path(&self) -> &std::path::Path {
        &self.updates
    }
}

/// A server whose updates dir the test fills.
async fn app_with_updates() -> (TestApp, UpdatesDir) {
    let root = tempfile::tempdir().unwrap();
    let updates = root.path().join("updates");
    std::fs::create_dir(&updates).unwrap();
    let mut cfg = test_config();
    cfg.updates_dir = updates.clone();
    (
        spawn_with(cfg).await,
        UpdatesDir {
            _root: root,
            updates,
        },
    )
}

async fn get(app: &TestApp, path: &str, token: Option<&str>) -> reqwest::Response {
    let mut req = app.http.get(app.url(path));
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    req.send().await.unwrap()
}

#[tokio::test]
async fn manifest_needs_a_session() {
    let (app, dir) = app_with_updates().await;
    let manifest = br#"{"version":"0.2.1","platforms":{}}"#;
    std::fs::write(dir.path().join("latest.json"), manifest).unwrap();
    let (_, token) = register(&app, "alex").await;

    assert_eq!(get(&app, "/updates/latest.json", None).await.status(), 401);
    assert_eq!(
        get(&app, "/updates/latest.json", Some("not-a-token"))
            .await
            .status(),
        401
    );

    let r = get(&app, "/updates/latest.json", Some(&token)).await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["cache-control"], "no-store");
    assert_eq!(r.headers()["content-type"], "application/json");
    assert_eq!(&r.bytes().await.unwrap()[..], &manifest[..]);
}

#[tokio::test]
async fn installer_is_streamed_to_signed_in_people() {
    let (app, dir) = app_with_updates().await;
    let body: Vec<u8> = (0..3 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
    std::fs::write(dir.path().join("Pulse_0.2.0_x64-setup.exe"), &body).unwrap();
    let (_, token) = register(&app, "sam").await;

    let path = "/updates/Pulse_0.2.0_x64-setup.exe";
    assert_eq!(get(&app, path, None).await.status(), 401);
    let r = get(&app, path, Some(&token)).await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["content-type"], "application/octet-stream");
    let got = r.bytes().await.unwrap();
    assert_eq!(got.len(), body.len());
    assert!(got[..] == body[..], "the installer's bytes, unchanged");
}

#[tokio::test]
async fn only_installer_names_are_served() {
    let (app, dir) = app_with_updates().await;
    for name in [
        "latest.json.tmp",
        "Pulse_0.2.0_x64-setup.exe.sig",
        "pulse.exe",
        "pulse.db",
    ] {
        std::fs::write(dir.path().join(name), b"secret").unwrap();
    }
    // a file beside the updates dir, which a `..` would reach
    std::fs::write(dir.path().parent().unwrap().join("pulse.db"), b"secret").unwrap();
    let (_, token) = register(&app, "kim").await;
    for path in [
        "/updates/..%2Fpulse.db",
        "/updates/%2E%2E%2Fpulse.db",
        "/updates/latest.json.tmp",
        "/updates/Pulse_0.2.0_x64-setup.exe.sig",
        "/updates/pulse.exe",
        "/updates/pulse.db",
        "/updates/Pulse_0.2_x64-setup.exe",
        "/updates/",
    ] {
        let r = get(&app, path, Some(&token)).await;
        assert_eq!(r.status(), 404, "{path}");
    }
}

#[tokio::test]
async fn no_manifest_is_404() {
    let (app, _dir) = app_with_updates().await;
    let (_, token) = register(&app, "jo").await;
    assert_eq!(
        get(&app, "/updates/latest.json", Some(&token))
            .await
            .status(),
        404
    );
    // a listed name that isn't there yet
    let r = get(&app, "/updates/Pulse_0.2.9_x64-setup.exe", Some(&token)).await;
    assert_eq!(r.status(), 404);
}

#[test]
fn installer_names() {
    assert!(installer_name_ok("Pulse_0.2.0_x64-setup.exe"));
    assert!(installer_name_ok("Pulse_10.20.300_x64-setup.exe"));
    for bad in [
        "Pulse_0.2_x64-setup.exe",
        "Pulse_0.2.0_x64-setup.exe.sig",
        "../Pulse_0.2.0_x64-setup.exe",
        "Pulse_0.2.0_x64-setup.exe/..",
        "Pulse_a.b.c_x64-setup.exe",
        "Pulse__x64-setup.exe",
        "latest.json",
        "",
    ] {
        assert!(!installer_name_ok(bad), "{bad}");
    }
}

/// Without colour codes, so `field=value` reads as written.
fn plain(log: &str) -> String {
    let mut out = String::new();
    let mut chars = log.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// #460: each manifest request is logged at info with the app's version (when it sends one), its
/// updater's User-Agent and the status, refused ones included; never the token.
#[tokio::test]
async fn manifest_requests_are_logged_with_version_and_status_never_the_token() {
    let root = tempfile::tempdir().unwrap();
    let updates = root.path().join("updates");
    std::fs::create_dir(&updates).unwrap();
    std::fs::write(
        updates.join("latest.json"),
        br#"{"version":"0.2.15","platforms":{}}"#,
    )
    .unwrap();
    let db_url = format!("sqlite://{}", root.path().join("pulse.db").display());
    let dir = updates.to_str().unwrap().to_string();
    let p = spawn_binary_on(db_url, &[("PULSE_UPDATES_DIR", &dir)], Some(root)).await;
    let http = reqwest::Client::new();
    let s: pulse_protocol::rest::SessionResponse = http
        .post(p.url("/auth/register"))
        .json(&serde_json::json!({"invite_code": binary_invite(&p), "username": "alex", "password": "hunter2hunter2"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ask = |token: &str| {
        http.get(p.url("/updates/latest.json"))
            .bearer_auth(token)
            .header("X-Pulse-Version", "0.2.2")
            .header("User-Agent", "tauri-plugin-updater/2.14.1")
            .send()
    };
    assert_eq!(ask(&s.token).await.unwrap().status(), 200);
    assert_eq!(ask("not-a-real-token").await.unwrap().status(), 401);
    // an app too old to send its version
    let r = http
        .get(p.url("/updates/latest.json"))
        .bearer_auth(&s.token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let lines = loop {
        let log = plain(&p.log());
        let lines: Vec<String> = log
            .lines()
            .filter(|l| l.contains("update check"))
            .map(str::to_string)
            .collect();
        if lines.len() >= 3 || std::time::Instant::now() > deadline {
            break lines;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(lines[0].contains("INFO"), "{}", lines[0]);
    assert!(lines[0].contains("app_version=0.2.2"), "{}", lines[0]);
    assert!(
        lines[0].contains("user_agent=tauri-plugin-updater/2.14.1"),
        "{}",
        lines[0]
    );
    assert!(lines[0].contains("status=200"), "{}", lines[0]);
    assert!(lines[1].contains("status=401"), "{}", lines[1]);
    assert!(lines[2].contains("app_version=-"), "{}", lines[2]);
    let log = p.log();
    assert!(!log.contains(&s.token), "the session token was logged");
    assert!(!log.contains("not-a-real-token"), "a token was logged");
}

/// Each person's last update check is kept (#534): their version and what they were told.
#[tokio::test]
async fn update_checks_are_recorded_per_person() {
    let (app, dir) = app_with_updates().await;
    let (alex, a) = register(&app, "alex").await;
    let (sam, s) = register(&app, "sam").await;
    let check = |token: String, version: Option<&'static str>| {
        let mut req = app
            .http
            .get(app.url("/updates/latest.json"))
            .bearer_auth(token);
        if let Some(v) = version {
            req = req.header("X-Pulse-Version", v);
        }
        req.send()
    };
    let row = |id: pulse_protocol::ids::UserId| {
        sqlx::query_as::<_, (Option<String>, Option<String>, Option<String>)>(
            "SELECT check_version, check_result, checked_at FROM app_versions WHERE user_id = ?",
        )
        .bind(id.to_string())
        .fetch_one(&app.db)
    };

    // nothing published yet
    assert_eq!(
        check(a.clone(), Some("0.2.14")).await.unwrap().status(),
        404
    );
    let (v, r, at) = row(alex).await.unwrap();
    assert_eq!(
        (v.as_deref(), r.as_deref()),
        (Some("0.2.14"), Some("no_manifest"))
    );
    assert!(at.is_some());

    std::fs::write(
        dir.path().join("latest.json"),
        br#"{"version":"0.2.16","platforms":{}}"#,
    )
    .unwrap();
    assert_eq!(
        check(a.clone(), Some("0.2.14")).await.unwrap().status(),
        200
    );
    assert_eq!(row(alex).await.unwrap().1.as_deref(), Some("offered"));
    assert_eq!(
        check(s.clone(), Some("0.2.16")).await.unwrap().status(),
        200
    );
    assert_eq!(row(sam).await.unwrap().1.as_deref(), Some("up_to_date"));
    // an app that sends no version (older than #460)
    assert_eq!(check(s, None).await.unwrap().status(), 200);
    let (v, r, _) = row(sam).await.unwrap();
    assert_eq!((v, r.as_deref()), (None, Some("unknown")));
    // a refused check (signed out) records nothing
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM app_versions")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(n, 2);
}

/// A gateway connect notes the app's version from the upgrade's header, or none from an older app (#534).
#[tokio::test]
async fn connects_are_recorded_with_the_app_version() {
    use futures::SinkExt;
    use pulse_protocol::gateway::{ClientFrame, ServerFrame};
    use tokio_tungstenite::tungstenite::{Message as Ws, client::IntoClientRequest};

    let (app, _dir) = app_with_updates().await;
    let (kim, k) = register(&app, "kim").await;
    let (jo, j) = register(&app, "jo").await;
    let connect = |token: String, version: Option<&'static str>| {
        let mut req = app.ws_url("/gateway").into_client_request().unwrap();
        if let Some(v) = version {
            req.headers_mut()
                .insert("X-Pulse-Version", v.parse().unwrap());
        }
        async move {
            let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
            let f = serde_json::to_string(&ClientFrame::Hello {
                token,
                client_version: pulse_protocol::PROTOCOL_VERSION,
            })
            .unwrap();
            ws.send(Ws::text(f)).await.unwrap();
            match next_frame_beating(&mut ws, std::time::Duration::from_secs(3)).await {
                Some(ServerFrame::Ready(_)) => ws,
                other => panic!("expected Ready, got {other:?}"),
            }
        }
    };
    let _kim_ws = connect(k, Some("0.2.17")).await;
    let _jo_ws = connect(j, None).await;

    // written beside Ready, not before it
    let row = |id: pulse_protocol::ids::UserId| {
        sqlx::query_as::<_, (Option<String>, Option<String>)>(
            "SELECT connect_version, connected_at FROM app_versions WHERE user_id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&app.db)
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while (row(kim).await.unwrap().is_none() || row(jo).await.unwrap().is_none())
        && std::time::Instant::now() < deadline
    {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let (v, at) = row(kim).await.unwrap().expect("kim's connect");
    assert_eq!(v.as_deref(), Some("0.2.17"));
    assert!(at.is_some());
    let (v, at) = row(jo).await.unwrap().expect("jo's connect");
    assert_eq!(v, None);
    assert!(at.is_some());

    let rows = pulse_server::app_versions::list(&app.db).await.unwrap();
    let out = pulse_server::app_versions::format_report(&rows, Some("0.2.17"));
    assert!(out.contains("kim   0.2.17 "), "{out}");
    assert!(out.contains("jo    ?"), "{out}");
}
