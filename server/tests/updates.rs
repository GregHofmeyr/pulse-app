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
