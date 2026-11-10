//! Downloadable models (#77): the low-delay DeepFilterNet3 model, only for signed-in people, only by its exact name.
mod common;

use common::*;
use pulse_server::models::{LOW_DELAY_MODEL, model_name_ok};

/// The updates dir the test fills: `updates/` inside a temp dir, with the models in `updates/models/`.
struct Dirs {
    _root: tempfile::TempDir,
    updates: std::path::PathBuf,
}

impl Dirs {
    fn models(&self) -> std::path::PathBuf {
        self.updates.join("models")
    }
}

async fn app_with_models() -> (TestApp, Dirs) {
    let root = tempfile::tempdir().unwrap();
    let updates = root.path().join("updates");
    std::fs::create_dir_all(updates.join("models")).unwrap();
    let mut cfg = test_config();
    cfg.updates_dir = updates.clone();
    (
        spawn_with(cfg).await,
        Dirs {
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
async fn the_model_is_streamed_to_signed_in_people_only() {
    let (app, dirs) = app_with_models().await;
    let body: Vec<u8> = (0..2 * 1024 * 1024).map(|i| (i % 253) as u8).collect();
    std::fs::write(dirs.models().join(LOW_DELAY_MODEL), &body).unwrap();
    let (_, token) = register(&app, "alex").await;
    let path = format!("/models/{LOW_DELAY_MODEL}");

    assert_eq!(get(&app, &path, None).await.status(), 401);
    assert_eq!(get(&app, &path, Some("not-a-token")).await.status(), 401);

    let r = get(&app, &path, Some(&token)).await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["content-type"], "application/octet-stream");
    assert_eq!(r.headers()["content-length"], body.len().to_string());
    let got = r.bytes().await.unwrap();
    assert!(got[..] == body[..], "the model's bytes, unchanged");
}

#[tokio::test]
async fn not_published_yet_is_404() {
    let (app, _dirs) = app_with_models().await;
    let (_, token) = register(&app, "sam").await;
    let r = get(&app, &format!("/models/{LOW_DELAY_MODEL}"), Some(&token)).await;
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn only_the_allowlisted_name_is_served() {
    let (app, dirs) = app_with_models().await;
    for name in [
        "DeepFilterNet3_onnx.tar.gz",
        "DeepFilterNet3_ll_onnx.tar.gz.tmp",
        "pulse.db",
    ] {
        std::fs::write(dirs.models().join(name), b"secret").unwrap();
    }
    // the updates dir's own files and its parent's, which a `..` would reach
    std::fs::write(dirs.updates.join("latest.json"), b"secret").unwrap();
    std::fs::write(dirs.updates.parent().unwrap().join("pulse.db"), b"secret").unwrap();
    let (_, token) = register(&app, "kim").await;
    for path in [
        "/models/DeepFilterNet3_onnx.tar.gz",
        "/models/DeepFilterNet3_ll_onnx.tar.gz.tmp",
        "/models/pulse.db",
        "/models/..%2Flatest.json",
        "/models/%2E%2E%2F..%2Fpulse.db",
        "/models/",
    ] {
        let r = get(&app, path, Some(&token)).await;
        assert_eq!(r.status(), 404, "{path}");
    }
}

#[test]
fn model_names() {
    assert!(model_name_ok("DeepFilterNet3_ll_onnx.tar.gz"));
    for bad in [
        "DeepFilterNet3_onnx.tar.gz",
        "deepfilternet3_ll_onnx.tar.gz",
        "../DeepFilterNet3_ll_onnx.tar.gz",
        "DeepFilterNet3_ll_onnx.tar.gz/..",
        "",
    ] {
        assert!(!model_name_ok(bad), "{bad}");
    }
}
