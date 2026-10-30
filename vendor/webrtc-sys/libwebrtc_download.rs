// Pulse addition (not upstream): download the prebuilt libwebrtc archive and verify its SHA-256
// against a pinned value before extracting it. Upstream's `webrtc_sys_build::download_webrtc()`
// extracts whatever GitHub serves, unverified. This keeps its URL and cache layout
// (`webrtc_sys_build::webrtc_dir()`), so only the verification step is new.
//
// `LK_CUSTOM_WEBRTC` (a locally built libwebrtc) bypasses the download and the check, as before.

use std::{
    env,
    fs::{self, File},
    io::{self, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

/// SHA-256 of `webrtc-<triple>.zip` from the `webrtc_sys_build::WEBRTC_TAG` GitHub release, one
/// line per target Pulse builds (linux x64: CI + dev; windows x64: release installer).
///
/// To bump (new webrtc-sys-build / WEBRTC_TAG, or a new target): for each triple, download
///   https://github.com/livekit/rust-sdks/releases/download/<WEBRTC_TAG>/webrtc-<triple>.zip
/// run `sha256sum` on it, cross-check against the asset digest GitHub shows on the release page
/// (or `gh release view <WEBRTC_TAG> -R livekit/rust-sdks --json assets`), and update the tag and
/// hashes here together. A triple without an entry fails the build.
const WEBRTC_TAG: &str = "webrtc-a545de4";
const PINNED_SHA256: &[(&str, &str)] = &[
    ("linux-x64-release", "2ece73b9a72a2cacd73d0c62ac3a414e202f37885c3c8f49dddb43c1443ac0bd"),
    ("win-x64-release", "862403fcb248c7d8803487e4c22b8821c6130e89100cb9c69be95ef45ff46146"),
];

/// Written next to the extracted directory once the archive it came from has been verified. A
/// cache without it (populated before verification existed, or by another hash) is re-downloaded.
const VERIFIED_MARKER: &str = ".pulse-verified-sha256";

/// Make sure `webrtc_sys_build::webrtc_dir()` holds a libwebrtc whose archive matched the pin.
pub fn ensure_verified_webrtc() {
    if let Some(custom) = webrtc_sys_build::custom_dir() {
        if !custom.exists() {
            panic!("LK_CUSTOM_WEBRTC points to {}, which does not exist", custom.display());
        }
        return;
    }

    assert_eq!(
        webrtc_sys_build::WEBRTC_TAG,
        WEBRTC_TAG,
        "webrtc-sys-build now downloads libwebrtc {}, but the pinned SHA-256s in \
         vendor/webrtc-sys/libwebrtc_download.rs are for {WEBRTC_TAG}; re-pin them (see the comment there)",
        webrtc_sys_build::WEBRTC_TAG,
    );

    let triple = webrtc_sys_build::webrtc_triple();
    let expected = match PINNED_SHA256.iter().find(|(t, _)| *t == triple) {
        Some((_, hash)) => *hash,
        None => panic!(
            "no pinned SHA-256 for the prebuilt libwebrtc `{triple}` ({}). Add one to \
             vendor/webrtc-sys/libwebrtc_download.rs (see the comment there), or build libwebrtc \
             yourself and set LK_CUSTOM_WEBRTC",
            webrtc_sys_build::download_url(),
        ),
    };

    if let Err(e) = ensure(&triple, expected) {
        panic!("libwebrtc: {e}");
    }
}

fn ensure(triple: &str, expected: &str) -> Result<(), String> {
    let scratch = scratch::path(webrtc_sys_build::SCRATH_PATH);
    fs::create_dir_all(&scratch).map_err(|e| format!("create {}: {e}", scratch.display()))?;
    // Same lock file as webrtc_sys_build::download_webrtc(), so the two never race.
    let lock = File::create(scratch.join(".lock")).map_err(|e| format!("create lock file: {e}"))?;
    fs2::FileExt::lock_exclusive(&lock).map_err(|e| format!("lock {}: {e}", scratch.display()))?;

    let webrtc_dir = webrtc_sys_build::webrtc_dir();
    let parent = webrtc_dir.parent().expect("webrtc_dir has a parent").to_path_buf();
    let marker = parent.join(VERIFIED_MARKER);

    if webrtc_dir.exists() {
        if fs::read_to_string(&marker).is_ok_and(|m| m.trim() == expected) {
            return Ok(());
        }
        println!(
            "cargo:warning=libwebrtc cache at {} is not verified against the current pinned SHA-256; \
             re-downloading it",
            webrtc_dir.display()
        );
        fs::remove_dir_all(&webrtc_dir)
            .map_err(|e| format!("remove unverified {}: {e}", webrtc_dir.display()))?;
    }
    let _ = fs::remove_file(&marker);

    let url = webrtc_sys_build::download_url();
    let zip_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("webrtc.zip");
    // Removes the zip on every exit path, success, error or panic (declared before the file
    // handle, so the handle is closed first, which Windows needs).
    let _zip_guard = RemoveOnDrop(zip_path.clone());
    let (zip_file, actual) = download(&url, &zip_path)?;
    if actual != expected {
        return Err(format!(
            "SHA-256 mismatch for {url}\n  expected {expected}\n  got      {actual}\n\
             The archive was not extracted. If LiveKit re-published it on purpose, verify the new \
             archive and update PINNED_SHA256 in vendor/webrtc-sys/libwebrtc_download.rs"
        ));
    }

    // Extracted from the same open handle that was hashed: nothing can swap the file in between.
    extract(zip_file, &parent, triple, &webrtc_dir)?;
    // Written last: a crash before this point leaves a cache that gets re-verified next time.
    fs::write(&marker, format!("{expected}\n"))
        .map_err(|e| format!("write {}: {e}", marker.display()))?;
    Ok(())
}

/// Stream `url` into `dest`, returning the still-open file (rewound to the start) and the hex
/// SHA-256 of what was written to it.
fn download(url: &str, dest: &Path) -> Result<(File, String), String> {
    let mut last_err = String::new();
    for attempt in 1..=3 {
        if attempt > 1 {
            println!(
                "cargo:warning=libwebrtc download attempt {} failed ({last_err}), retrying in 5s",
                attempt - 1
            );
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
        let mut resp = match reqwest::blocking::get(url) {
            Ok(r) => r,
            Err(e) => {
                last_err = e.to_string();
                continue;
            }
        };
        let status = resp.status();
        if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            last_err = status.to_string();
            continue;
        }
        if status != reqwest::StatusCode::OK {
            return Err(format!("download {url}: HTTP {status}"));
        }
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(dest)
            .map_err(|e| format!("create {}: {e}", dest.display()))?;
        let mut out = HashingWriter { inner: file, hasher: Sha256::new() };
        match io::copy(&mut resp, &mut out) {
            Ok(_) => {
                let HashingWriter { inner: mut file, hasher } = out;
                file.flush().map_err(|e| format!("write {}: {e}", dest.display()))?;
                file.seek(SeekFrom::Start(0))
                    .map_err(|e| format!("rewind {}: {e}", dest.display()))?;
                return Ok((file, hex(&hasher.finalize())));
            }
            Err(e) => last_err = e.to_string(),
        }
    }
    Err(format!("download {url}: {last_err}"))
}

/// Extract into a sibling temp dir, then rename into place (as upstream does), so observers see
/// either no `webrtc_dir` or a complete one.
fn extract(zip_file: File, parent: &Path, triple: &str, webrtc_dir: &Path) -> Result<(), String> {
    let tmp = parent.join(format!(".{triple}.tmp"));
    let _ = fs::remove_dir_all(&tmp);
    // Removes the temp dir on every exit path, including a failed extract or rename.
    let _tmp_guard = RemoveOnDrop(tmp.clone());
    fs::create_dir_all(&tmp).map_err(|e| format!("create {}: {e}", tmp.display()))?;

    let mut archive = zip::ZipArchive::new(zip_file).map_err(|e| format!("open archive: {e}"))?;
    archive.extract(&tmp).map_err(|e| format!("extract archive: {e}"))?;
    drop(archive);

    // The zip root is `{triple}/`.
    fs::rename(tmp.join(triple), webrtc_dir)
        .map_err(|e| format!("move extracted libwebrtc into {}: {e}", webrtc_dir.display()))?;

    #[cfg(unix)]
    ensure_owner_readable(webrtc_dir).map_err(|e| format!("fix permissions: {e}"))?;
    Ok(())
}

/// Deletes a file or directory tree when dropped (best effort).
struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        if self.0.is_dir() {
            let _ = fs::remove_dir_all(&self.0);
        } else {
            let _ = fs::remove_file(&self.0);
        }
    }
}

struct HashingWriter {
    inner: File,
    hasher: Sha256,
}

impl Write for HashingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Upstream normalizes permissions after extraction (some archive entries lack owner read).
#[cfg(unix)]
fn ensure_owner_readable(p: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let meta = fs::symlink_metadata(p)?;
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    let mut perms = meta.permissions();
    let mut mode = perms.mode() | 0o600;
    if meta.is_dir() {
        mode |= 0o100;
    }
    perms.set_mode(mode);
    let _ = fs::set_permissions(p, perms);
    if meta.is_dir() {
        for entry in fs::read_dir(p)? {
            ensure_owner_readable(&entry?.path())?;
        }
    }
    Ok(())
}
