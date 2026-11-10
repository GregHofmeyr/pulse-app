//! Models downloaded on first use (#77). Today one: DeepFilterNet3's low-delay model, which "Strong (low delay)" noise
//! suppression runs. Strong's normal model stays bundled; this one (36 MB) only comes down when someone picks it.
//!
//! - It comes from the signed-in server's `/models/<name>` (sign-in only, like `/updates`), asked with the session
//!   token. The token only goes to that server: the URL is built on it and checked ([`crate::updater::same_origin`]),
//!   and redirects aren't followed.
//! - It streams to `<name>.part` beside the final file while its SHA-256 is computed, and is renamed into place only
//!   when the hash is the pinned one ([`LOW_DELAY_SHA256`]). A cut-off or damaged download leaves nothing behind.
//! - Every load checks the hash again; a file that doesn't match is deleted and reported as failed.
//! - [`ModelStore::epoch`] moves on each finished download, so a running mic processor can pick the model up.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use futures::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use url::Url;

/// The low-delay model, by the name libDF ships it under (and the server's allowlist serves).
pub const LOW_DELAY_MODEL: &str = "DeepFilterNet3_ll_onnx.tar.gz";
/// Its SHA-256 (libDF git d375b2d). `deploy/publish-models.sh` reads it from here, so the server only ever gets this file.
pub const LOW_DELAY_SHA256: &str =
    "5998e58e8ba0e09bb76986ef97b84afa065a571ef282d4a1222f341e3251cf3a";
/// Download progress and the end of a download: [`ModelStatus`].
pub const MODEL_EVENT: &str = "pulse://model";

/// How long connecting may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The longest the download may go without receiving anything before it fails.
const STALL: Duration = Duration::from_secs(30);
/// Anything bigger isn't the model (it's 36 MB).
const MAX_BYTES: u64 = 128 * 1024 * 1024;

/// Where the model is, for Settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ModelStatus {
    Missing,
    Downloading { downloaded: u64, total: Option<u64> },
    Ready,
    Failed { error: String },
}

/// Why a load gave no model.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    #[error("not downloaded")]
    Missing,
    #[error("the downloaded model was damaged")]
    Damaged,
    #[error("couldn't read the model: {0}")]
    Io(String),
}

#[derive(Default)]
struct State {
    /// (downloaded, total) while a download runs.
    downloading: Option<(u64, Option<u64>)>,
    /// Why the last download or load failed (cleared by a good download).
    error: Option<String>,
}

/// One downloadable model in a directory.
pub struct ModelStore {
    dir: PathBuf,
    name: &'static str,
    sha256: &'static str,
    state: Mutex<State>,
    epoch: AtomicU64,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl ModelStore {
    pub fn new(dir: PathBuf, name: &'static str, sha256: &'static str) -> Self {
        Self {
            dir,
            name,
            sha256,
            state: Mutex::default(),
            epoch: AtomicU64::new(0),
        }
    }

    /// The low-delay model, kept in `dir`.
    pub fn low_delay(dir: PathBuf) -> Self {
        Self::new(dir, LOW_DELAY_MODEL, LOW_DELAY_SHA256)
    }

    fn path(&self) -> PathBuf {
        self.dir.join(self.name)
    }

    fn part(&self) -> PathBuf {
        self.dir.join(format!("{}.part", self.name))
    }

    /// Bumps on every finished download.
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    /// The file is there (not checked: [`Self::load`] checks it).
    pub fn present(&self) -> bool {
        self.path().is_file()
    }

    pub fn status(&self) -> ModelStatus {
        let st = self.state.lock().unwrap();
        if let Some((downloaded, total)) = st.downloading {
            ModelStatus::Downloading { downloaded, total }
        } else if self.present() {
            ModelStatus::Ready
        } else if let Some(error) = &st.error {
            ModelStatus::Failed {
                error: error.clone(),
            }
        } else {
            ModelStatus::Missing
        }
    }

    /// The model's bytes, checked against the pinned hash every time. A file that doesn't match is deleted (and the
    /// status says so), so the next download starts clean.
    pub fn load(&self) -> Result<Vec<u8>, LoadError> {
        let bytes = match std::fs::read(self.path()) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(LoadError::Missing),
            Err(e) => return Err(LoadError::Io(e.to_string())),
        };
        if hex(&Sha256::digest(&bytes)) != self.sha256 {
            let _ = std::fs::remove_file(self.path());
            self.state.lock().unwrap().error = Some(LoadError::Damaged.to_string());
            return Err(LoadError::Damaged);
        }
        Ok(bytes)
    }

    /// Download from `server` (the signed-in one) with `token`; `progress` hears each step (a few a second) and the
    /// end. A download already running: returns at once (that one carries on and reports).
    pub async fn download(
        &self,
        server: &str,
        token: &str,
        mut progress: impl FnMut(&ModelStatus),
    ) -> Result<(), String> {
        {
            let mut st = self.state.lock().unwrap();
            if st.downloading.is_some() {
                return Ok(());
            }
            st.downloading = Some((0, None));
        }
        progress(&self.status());
        let result = self.fetch(server, token, &mut progress).await;
        let _ = std::fs::remove_file(self.part());
        {
            let mut st = self.state.lock().unwrap();
            st.downloading = None;
            st.error = result.as_ref().err().cloned();
        }
        if result.is_ok() {
            self.epoch.fetch_add(1, Ordering::SeqCst);
        }
        progress(&self.status());
        result
    }

    async fn fetch(
        &self,
        server: &str,
        token: &str,
        progress: &mut impl FnMut(&ModelStatus),
    ) -> Result<(), String> {
        let url = model_url(server, self.name)?;
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            // the token goes to this server only: a redirect anywhere is a failure
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| format!("couldn't start the download: {e}"))?;
        let resp = http
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|_| "couldn't reach the server".to_string())?;
        match resp.status().as_u16() {
            200 => {}
            401 => return Err("sign in again to download it".into()),
            404 => return Err("the server doesn't have it yet".into()),
            code => return Err(format!("the server answered {code}")),
        }
        let total = resp.content_length();
        if total.is_some_and(|t| t > MAX_BYTES) {
            return Err("the server offered something else".into());
        }
        tokio::fs::create_dir_all(&self.dir)
            .await
            .map_err(|e| format!("couldn't save it: {e}"))?;
        let mut file = tokio::fs::File::create(self.part())
            .await
            .map_err(|e| format!("couldn't save it: {e}"))?;
        let mut hasher = Sha256::new();
        let mut downloaded = 0u64;
        let mut last_emit = Instant::now();
        let mut body = resp.bytes_stream();
        loop {
            let chunk = match tokio::time::timeout(STALL, body.next()).await {
                Err(_) => return Err("the download stalled".into()),
                Ok(None) => break,
                Ok(Some(Err(_))) => return Err("the download was cut off".into()),
                Ok(Some(Ok(c))) => c,
            };
            downloaded += chunk.len() as u64;
            if downloaded > MAX_BYTES {
                return Err("the server offered something else".into());
            }
            hasher.update(&chunk);
            file.write_all(&chunk)
                .await
                .map_err(|e| format!("couldn't save it: {e}"))?;
            if last_emit.elapsed() >= Duration::from_millis(100) {
                last_emit = Instant::now();
                self.state.lock().unwrap().downloading = Some((downloaded, total));
                progress(&ModelStatus::Downloading { downloaded, total });
            }
        }
        if total.is_some_and(|t| t != downloaded) {
            return Err("the download was cut off".into());
        }
        if hex(&hasher.finalize()) != self.sha256 {
            return Err("the download was damaged".into());
        }
        file.sync_all()
            .await
            .map_err(|e| format!("couldn't save it: {e}"))?;
        drop(file);
        tokio::fs::rename(self.part(), self.path())
            .await
            .map_err(|e| format!("couldn't save it: {e}"))
    }
}

/// `/models/<name>` on `server`, refused unless it's on the same origin (it carries the session token).
pub fn model_url(server: &str, name: &str) -> Result<Url, String> {
    let base = Url::parse(server).map_err(|e| format!("bad server address: {e}"))?;
    // the base's path is kept: `https://host/pulse` → `https://host/pulse/models/…`
    let mut dir = base.clone();
    if !dir.path().ends_with('/') {
        dir.set_path(&format!("{}/", dir.path()));
    }
    let url = dir
        .join(&format!("models/{name}"))
        .map_err(|e| format!("bad server address: {e}"))?;
    if !crate::updater::same_origin(&base, &url) {
        return Err("refusing to send the session to another host".into());
    }
    Ok(url)
}

static LOW_DELAY: OnceLock<Arc<ModelStore>> = OnceLock::new();

/// At startup: the low-delay model lives in `dir` (the app's local data, shared by profiles).
pub fn install_low_delay(dir: PathBuf) -> Arc<ModelStore> {
    LOW_DELAY
        .get_or_init(|| Arc::new(ModelStore::low_delay(dir)))
        .clone()
}

/// The low-delay model's store (`None` before [`install_low_delay`], e.g. in tests).
pub fn low_delay_store() -> Option<Arc<ModelStore>> {
    LOW_DELAY.get().cloned()
}

/// The low-delay model in libDF's cargo checkout (the file `just publish-models` uploads), found by its hash under
/// `$CARGO_HOME/git/checkouts`. For the tests and `tests/denoise_bench.rs`; never used by the app.
#[doc(hidden)]
pub fn checkout_low_delay_model_path() -> Option<PathBuf> {
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|h| Path::new(&h).join(".cargo"))
        })?;
    let checkouts = cargo_home.join("git").join("checkouts");
    for repo in std::fs::read_dir(checkouts).ok()?.flatten() {
        if !repo
            .file_name()
            .to_string_lossy()
            .starts_with("deepfilternet-")
        {
            continue;
        }
        for rev in std::fs::read_dir(repo.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            let p = rev.path().join("models").join(LOW_DELAY_MODEL);
            if std::fs::read(&p).is_ok_and(|b| hex(&Sha256::digest(&b)) == LOW_DELAY_SHA256) {
                return Some(p);
            }
        }
    }
    None
}

#[cfg(test)]
impl ModelStore {
    /// As if a download had just finished with `bytes` (no server).
    pub(crate) fn put_for_test(&self, bytes: &[u8]) {
        std::fs::create_dir_all(&self.dir).unwrap();
        std::fs::write(self.path(), bytes).unwrap();
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
pub(crate) fn checkout_low_delay_model() -> Vec<u8> {
    let p = checkout_low_delay_model_path()
        .expect("libDF's checkout (cargo git checkouts) has the low-delay model");
    std::fs::read(p).unwrap()
}

/// The signed-in session: (server, token).
fn current_session(app: &tauri::AppHandle) -> Option<(String, String)> {
    use tauri::Manager;
    app.try_state::<crate::commands::Core>()
        .and_then(|c| c.current().ok())
        .map(|(api, token)| (api.base().to_string(), token))
}

/// Where the low-delay model is.
#[tauri::command]
pub fn model_status() -> ModelStatus {
    low_delay_store().map_or(ModelStatus::Missing, |s| s.status())
}

/// Download the low-delay model from the signed-in server ([`MODEL_EVENT`]s on the way); the status it ends in.
#[tauri::command]
pub async fn model_download(app: tauri::AppHandle) -> Result<ModelStatus, String> {
    use tauri::Emitter;
    let store = low_delay_store().ok_or("models aren't available")?;
    let Some((server, token)) = current_session(&app) else {
        return Err("sign in to download it".into());
    };
    tracing::info!(model = LOW_DELAY_MODEL, "models: downloading");
    let result = store
        .download(&server, &token, |st| {
            let _ = app.emit(MODEL_EVENT, st);
        })
        .await;
    match &result {
        Ok(()) => tracing::info!(model = LOW_DELAY_MODEL, "models: downloaded"),
        Err(e) => tracing::warn!(model = LOW_DELAY_MODEL, error = %e, "models: download failed"),
    }
    Ok(store.status())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;

    type Requests = Arc<Mutex<Vec<String>>>;

    /// A one-shot HTTP server: each connection gets `reply` (raw bytes, then the connection closes). Returns its base
    /// URL and the request heads it saw.
    async fn serve(reply: Vec<u8>) -> (String, Requests) {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        let seen: Requests = Arc::default();
        let log = seen.clone();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = l.accept().await {
                let mut head = Vec::new();
                let mut buf = [0u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }
                log.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&head).into_owned());
                let _ = sock.write_all(&reply).await;
                let _ = sock.shutdown().await;
            }
        });
        (format!("http://{addr}/"), seen)
    }

    fn ok_reply(body: &[u8]) -> Vec<u8> {
        let mut r = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/octet-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        r.extend_from_slice(body);
        r
    }

    fn body() -> Vec<u8> {
        (0..300_000u32).map(|i| (i % 251) as u8).collect()
    }

    /// A store in a temp dir that expects [`body`].
    fn store() -> (tempfile::TempDir, ModelStore) {
        let dir = tempfile::tempdir().unwrap();
        let sha: &'static str = Box::leak(hex(&Sha256::digest(body())).into_boxed_str());
        let s = ModelStore::new(dir.path().join("models"), "m.tar.gz", sha);
        (dir, s)
    }

    fn leftovers(s: &ModelStore) -> Vec<String> {
        std::fs::read_dir(&s.dir)
            .map(|d| {
                d.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn a_good_download_lands_with_the_token_sent_and_loads() {
        let (_d, s) = store();
        assert_eq!(s.status(), ModelStatus::Missing);
        let (server, seen) = serve(ok_reply(&body())).await;
        let mut heard = Vec::new();
        s.download(&server, "tok123", |st| heard.push(st.clone()))
            .await
            .unwrap();
        let req = seen.lock().unwrap()[0].to_lowercase();
        assert!(req.starts_with("get /models/m.tar.gz "), "{req}");
        assert!(req.contains("authorization: bearer tok123"), "{req}");
        assert_eq!(s.status(), ModelStatus::Ready);
        assert_eq!(
            heard.first(),
            Some(&ModelStatus::Downloading {
                downloaded: 0,
                total: None
            })
        );
        assert_eq!(heard.last(), Some(&ModelStatus::Ready));
        assert_eq!(s.epoch(), 1, "a finished download moves the epoch");
        assert_eq!(s.load().unwrap(), body());
        assert_eq!(leftovers(&s), ["m.tar.gz"], "no .part left");
    }

    #[tokio::test]
    async fn a_download_with_the_wrong_hash_leaves_nothing() {
        let (_d, s) = store();
        let mut other = body();
        other[1000] ^= 1;
        let (server, _) = serve(ok_reply(&other)).await;
        let err = s.download(&server, "t", |_| {}).await.unwrap_err();
        assert!(err.contains("damaged"), "{err}");
        assert!(matches!(s.status(), ModelStatus::Failed { .. }));
        assert!(leftovers(&s).is_empty(), "{:?}", leftovers(&s));
        assert_eq!(s.epoch(), 0);
        assert_eq!(s.load(), Err(LoadError::Missing));
    }

    #[tokio::test]
    async fn a_cut_off_download_leaves_nothing() {
        let (_d, s) = store();
        let mut reply = ok_reply(&body());
        reply.truncate(reply.len() - 100_000); // the connection closes early
        let (server, _) = serve(reply).await;
        let err = s.download(&server, "t", |_| {}).await.unwrap_err();
        assert!(err.contains("cut off"), "{err}");
        assert!(matches!(s.status(), ModelStatus::Failed { .. }));
        assert!(leftovers(&s).is_empty(), "{:?}", leftovers(&s));
    }

    #[tokio::test]
    async fn offline_fails_and_a_retry_succeeds() {
        let (_d, s) = store();
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let dead = format!("http://{}/", l.local_addr().unwrap());
        drop(l); // nothing listens there now
        let err = s.download(&dead, "t", |_| {}).await.unwrap_err();
        assert!(err.contains("couldn't reach"), "{err}");
        assert_eq!(
            s.status(),
            ModelStatus::Failed {
                error: "couldn't reach the server".into()
            }
        );
        let (server, _) = serve(ok_reply(&body())).await;
        s.download(&server, "t", |_| {}).await.unwrap();
        assert_eq!(s.status(), ModelStatus::Ready, "a retry clears the failure");
    }

    #[tokio::test]
    async fn signed_out_and_not_published_say_why() {
        let (_d, s) = store();
        let (server, _) =
            serve(b"HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\n\r\n".to_vec()).await;
        let err = s.download(&server, "t", |_| {}).await.unwrap_err();
        assert!(err.contains("sign in"), "{err}");
        let (server, _) =
            serve(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n".to_vec()).await;
        let err = s.download(&server, "t", |_| {}).await.unwrap_err();
        assert!(err.contains("doesn't have it"), "{err}");
        assert!(leftovers(&s).is_empty());
    }

    /// The token goes to the signed-in server only: a redirect (anywhere) isn't followed.
    #[tokio::test]
    async fn redirects_are_not_followed() {
        let (_d, s) = store();
        let (elsewhere, elsewhere_seen) = serve(ok_reply(&body())).await;
        let redirect = format!(
            "HTTP/1.1 302 Found\r\nlocation: {elsewhere}models/m.tar.gz\r\ncontent-length: 0\r\n\r\n"
        );
        let (server, _) = serve(redirect.into_bytes()).await;
        assert!(s.download(&server, "secret", |_| {}).await.is_err());
        assert!(
            elsewhere_seen.lock().unwrap().is_empty(),
            "followed the redirect"
        );
        assert_ne!(s.status(), ModelStatus::Ready);
    }

    #[test]
    fn the_url_is_on_the_signed_in_server() {
        let u = model_url("https://pulse.example", LOW_DELAY_MODEL).unwrap();
        assert_eq!(
            u.as_str(),
            "https://pulse.example/models/DeepFilterNet3_ll_onnx.tar.gz"
        );
        let u = model_url("http://127.0.0.1:7890/", "m").unwrap();
        assert_eq!(u.as_str(), "http://127.0.0.1:7890/models/m");
        assert!(model_url("not a url", "m").is_err());
        let u = model_url("https://pulse.example/sub", "m").unwrap();
        assert_eq!(u.as_str(), "https://pulse.example/sub/models/m");
    }

    /// Every load checks the hash: a file changed on disk is deleted and reported, not used.
    #[test]
    fn a_damaged_file_on_disk_is_deleted_and_reported() {
        let (_d, s) = store();
        std::fs::create_dir_all(&s.dir).unwrap();
        let mut bad = body();
        bad[0] ^= 1;
        std::fs::write(s.path(), &bad).unwrap();
        assert_eq!(s.status(), ModelStatus::Ready, "present (unchecked)");
        assert_eq!(s.load(), Err(LoadError::Damaged));
        assert!(!s.present(), "deleted");
        assert_eq!(
            s.status(),
            ModelStatus::Failed {
                error: "the downloaded model was damaged".into()
            }
        );
        std::fs::write(s.path(), body()).unwrap();
        assert_eq!(s.load().unwrap(), body(), "a good file loads");
    }

    #[test]
    fn the_pinned_hash_is_the_checkouts_file() {
        let bytes = checkout_low_delay_model();
        assert_eq!(hex(&Sha256::digest(&bytes)), LOW_DELAY_SHA256);
    }

    #[test]
    fn status_serialises_for_the_ui() {
        let j = |s: ModelStatus| serde_json::to_value(s).unwrap();
        assert_eq!(
            j(ModelStatus::Missing),
            serde_json::json!({"state": "missing"})
        );
        assert_eq!(j(ModelStatus::Ready), serde_json::json!({"state": "ready"}));
        assert_eq!(
            j(ModelStatus::Downloading {
                downloaded: 5,
                total: Some(10)
            }),
            serde_json::json!({"state": "downloading", "downloaded": 5, "total": 10})
        );
        assert_eq!(
            j(ModelStatus::Failed { error: "x".into() }),
            serde_json::json!({"state": "failed", "error": "x"})
        );
    }
}
