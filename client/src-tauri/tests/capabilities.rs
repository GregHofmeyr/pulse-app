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

/// Also what keeps the two pop-outs without any capability: the link-preview pop-out (window `embed`, a remote page)
/// and the build pop-out (window `build`, #620). No file may name either, by label or wildcard, nor add `webviews` or
/// `remote` scopes. The build window gets no capability at all, not even event listening: Tauri hands every event
/// the core emits (`emit` and `emit_to` alike) to an any-target listener in any window, so listening would let it
/// read the whole gateway feed. Its app commands are limited by the IPC gate (src/ipc_gate.rs).
#[test]
fn only_the_default_capability_exists_and_names_neither_pop_out() {
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
fn no_capability_reaches_the_build_window() {
    for path in capability_files() {
        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        for key in ["windows", "webviews"] {
            for w in v[key].as_array().into_iter().flatten() {
                let w = w.as_str().expect("plain string labels only");
                assert!(
                    w != "build" && w != "embed" && !w.contains('*'),
                    "{} names {w:?}",
                    path.display()
                );
            }
        }
    }
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
