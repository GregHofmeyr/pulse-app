//! On Linux the clip editor plays local videos from a loopback HTTP server (src/video.rs), so the
//! Linux configs allow `http://127.0.0.1:*` as a media source. Tauri merges `tauri.linux.conf.json`
//! over `tauri.conf.json` (a JSON merge patch: the CSP string is replaced whole), and `just
//! dev-client` adds `tauri.dev.linux.conf.json` over `tauri.dev.conf.json` the same way. So each
//! Linux CSP is a full copy: it must stay its base plus that one source, and nothing else.

use std::{fs, path::Path};

const LOOPBACK: &str = "http://127.0.0.1:*";

fn config(name: &str) -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(name);
    serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap()
}

fn csp(name: &str) -> String {
    config(name)["app"]["security"]["csp"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// `csp` with `source` added at the end of `media-src`.
fn with_media_source(csp: &str, source: &str) -> String {
    let directives: Vec<String> = csp
        .split(';')
        .map(|d| {
            if d.trim_start().starts_with("media-src ") {
                format!("{d} {source}")
            } else {
                d.to_owned()
            }
        })
        .collect();
    assert_ne!(directives.join(";"), csp, "no media-src in {csp}");
    directives.join(";")
}

fn media_src(csp: &str) -> Vec<&str> {
    let d = csp
        .split(';')
        .map(str::trim)
        .find(|d| d.starts_with("media-src "))
        .unwrap();
    d.split_whitespace().skip(1).collect()
}

#[test]
fn the_linux_csp_is_the_base_csp_plus_loopback_media() {
    assert_eq!(
        csp("tauri.linux.conf.json"),
        with_media_source(&csp("tauri.conf.json"), LOOPBACK)
    );
}

#[test]
fn the_linux_dev_csp_is_the_dev_csp_plus_loopback_media() {
    assert_eq!(
        csp("tauri.dev.linux.conf.json"),
        with_media_source(&csp("tauri.dev.conf.json"), LOOPBACK)
    );
}

#[test]
fn the_linux_configs_change_nothing_but_the_csp() {
    for name in ["tauri.linux.conf.json", "tauri.dev.linux.conf.json"] {
        let mut v = config(name);
        v["app"]["security"]
            .as_object_mut()
            .unwrap()
            .remove("csp")
            .unwrap();
        assert_eq!(
            v,
            serde_json::json!({ "app": { "security": {} } }),
            "{name}"
        );
    }
}

/// Windows (and macOS) play from the asset protocol: no loopback media there.
#[test]
fn the_base_csps_allow_no_loopback_media() {
    for name in ["tauri.conf.json", "tauri.dev.conf.json"] {
        assert!(!media_src(&csp(name)).contains(&LOOPBACK), "{name}");
    }
}
