//! The webview's Tauri capability set is an explicit allow-list. Widening it must be a
//! deliberate change to this test, not a side effect (SECURITY.md section 4).

use std::{fs, path::Path};

fn capability_files() -> Vec<std::path::PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
    let mut files: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    files
}

#[test]
fn only_the_default_capability_exists_and_targets_main() {
    let files = capability_files();
    let names: Vec<_> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap())
        .collect();
    assert_eq!(names, ["default.json"]);
    let v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&files[0]).unwrap()).unwrap();
    assert_eq!(v["windows"], serde_json::json!(["main"]));
    assert!(v.get("webviews").is_none() && v.get("remote").is_none());
}

#[test]
fn main_window_permissions_are_exactly_the_ui_minimum() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities/default.json");
    let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let mut got: Vec<String> = v["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            p.as_str()
                .expect("plain string permissions only")
                .to_owned()
        })
        .collect();
    got.sort();
    let mut want = vec![
        // store.svelte.ts / voice.svelte.ts: listen() and its unlisten
        "core:event:allow-listen",
        "core:event:allow-unlisten",
        // windowFocus.svelte.ts: onFocusChanged (listen above) + isFocused()
        "core:window:allow-is-focused",
        // Shell.svelte window buttons
        "core:window:allow-minimize",
        "core:window:allow-toggle-maximize",
        "core:window:allow-close",
        // data-tauri-drag-region: drag, and double-click maximise
        "core:window:allow-start-dragging",
        "core:window:allow-internal-toggle-maximize",
    ];
    want.sort();
    assert_eq!(got, want);
}
