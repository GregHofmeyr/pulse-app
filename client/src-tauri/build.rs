fn main() {
    // Release workflows set PULSE_COMMIT (from GITHUB_SHA); bug reports name the build.
    println!("cargo:rerun-if-env-changed=PULSE_COMMIT");

    // The app manifest (Common Controls v6, the same one tauri-build embeds by default) goes in through the linker,
    // so the lib's test exe gets it too: tauri-build's resource reaches only `[[bin]]`s. Without it, a test exe that
    // builds a tauri app (updater.rs's mock-app test) imports TaskDialogIndirect from the v5 comctl32.dll and Windows
    // won't start it (0xc0000139, STATUS_ENTRYPOINT_NOT_FOUND).
    let mut windows = tauri_build::WindowsAttributes::new();
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        // No UAC section added by the linker: the app's manifest stays exactly tauri-build's.
        println!("cargo:rustc-link-arg=/MANIFESTUAC:NO");
        windows = tauri_build::WindowsAttributes::new_without_app_manifest();
    }
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("failed to run tauri-build");
}
