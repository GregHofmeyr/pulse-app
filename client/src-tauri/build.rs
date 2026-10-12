fn main() {
    // Release workflows set PULSE_COMMIT (from GITHUB_SHA); bug reports name the build.
    println!("cargo:rerun-if-env-changed=PULSE_COMMIT");
    tauri_build::build()
}
