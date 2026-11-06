//! App updates (#82): `latest.json` and the signed installer, only for signed-in people.
mod common;

use common::*;
use pulse_server::updates::installer_name_ok;

/// A server whose updates dir is a temp dir the test fills.
async fn app_with_updates() -> (TestApp, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = test_config();
    cfg.updates_dir = dir.path().to_path_buf();
    (spawn_with(cfg).await, dir)
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
    assert_eq!(get(&app, "/updates/latest.json", Some("not-a-token")).await.status(), 401);

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
    for name in ["latest.json.tmp", "Pulse_0.2.0_x64-setup.exe.sig", "pulse.exe", "pulse.db"] {
        std::fs::write(dir.path().join(name), b"secret").unwrap();
    }
    // a file beside the updates dir, which a `..` would reach
    std::fs::write(dir.path().parent().unwrap().join("pulse.db"), b"secret").ok();
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
    assert_eq!(get(&app, "/updates/latest.json", Some(&token)).await.status(), 404);
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
