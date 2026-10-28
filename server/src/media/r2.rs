//! The media store's remote location: clip videos in an S3-compatible bucket (Cloudflare R2).
//! The server never touches the video bytes. It signs PUT and GET URLs for clients with
//! `rusty-s3` (sans-IO SigV4) and runs HEAD and DELETE itself.
//!
//! No URL this module signs is ever formatted into an error or log line: a signed URL carries
//! the key id and a live signature. Every reqwest error goes through [`quiet`] first.

use std::time::Duration;

use anyhow::Context;
use reqwest::Url;
use rusty_s3::actions::S3Action;
use rusty_s3::{Bucket, Credentials, UrlStyle};

use super::MediaKind;
use crate::config::ClipStoreConfig;
use crate::error::AppResult;

const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
/// How long the URLs the server signs for itself (HEAD, DELETE) stay valid. Unrelated to the
/// HTTP timeout: it only has to outlast the clock skew between us and the store.
const SERVER_SIGN_TTL: Duration = Duration::from_secs(300);

/// Strip the request URL (key id and signature) from a transport error.
fn quiet(e: reqwest::Error) -> reqwest::Error {
    e.without_url()
}

#[derive(Clone)]
pub struct R2 {
    bucket: Bucket,
    creds: Credentials,
    http: reqwest::Client,
}

impl R2 {
    pub fn new(cfg: &ClipStoreConfig) -> anyhow::Result<Self> {
        let endpoint: Url = cfg
            .endpoint
            .parse()
            .context("PULSE_CLIPS_S3_ENDPOINT isn't a URL")?;
        let bucket = Bucket::new(
            endpoint,
            UrlStyle::Path,
            cfg.bucket.clone(),
            cfg.region.clone(),
        )
        .context("PULSE_CLIPS_S3_ENDPOINT can't be used as an S3 endpoint")?;
        Ok(Self {
            bucket,
            creds: Credentials::new(cfg.key_id.clone(), cfg.secret.clone()),
            http: reqwest::Client::builder().timeout(HTTP_TIMEOUT).build()?,
        })
    }

    /// `clips/<hash>.mp4` for a clip. Other kinds live on disk and have no object key.
    pub fn object_key(kind: MediaKind, hash: &str) -> String {
        debug_assert_eq!(kind, MediaKind::Clip);
        format!("clips/{hash}.mp4")
    }

    /// A PUT URL that only accepts exactly `size` bytes of `content_type`: both are signed
    /// headers, so the client must send them unchanged.
    pub fn upload_url(
        &self,
        key: &str,
        size: u64,
        content_type: &str,
        ttl: Duration,
    ) -> (Url, String) {
        let mut put = self.bucket.put_object(Some(&self.creds), key);
        put.headers_mut().insert("content-length", size.to_string());
        put.headers_mut()
            .insert("content-type", content_type.to_string());
        let url = put.sign(ttl);
        let expires = expires_at(&url, ttl);
        (url, expires)
    }

    pub fn download_url(&self, key: &str, ttl: Duration) -> (Url, String) {
        let get = self.bucket.get_object(Some(&self.creds), key);
        let url = get.sign(ttl);
        let expires = expires_at(&url, ttl);
        (url, expires)
    }

    /// The object's size, or `None` if it isn't there.
    pub async fn head(&self, key: &str) -> AppResult<Option<u64>> {
        let url = self
            .bucket
            .head_object(Some(&self.creds), key)
            .sign(SERVER_SIGN_TTL);
        let r = self
            .http
            .head(url)
            .send()
            .await
            .map_err(quiet)
            .with_context(|| format!("R2 HEAD {key}"))?;
        match r.status() {
            s if s.is_success() => Ok(Some(
                r.headers()
                    .get(reqwest::header::CONTENT_LENGTH)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse().ok())
                    .context("R2 HEAD answered without a content-length")?,
            )),
            reqwest::StatusCode::NOT_FOUND => Ok(None),
            s => Err(anyhow::anyhow!("R2 HEAD answered {s}").into()),
        }
    }

    /// Delete the object; one that is already gone counts as done.
    pub async fn delete(&self, key: &str) -> AppResult<()> {
        let url = self
            .bucket
            .delete_object(Some(&self.creds), key)
            .sign(SERVER_SIGN_TTL);
        let r = self
            .http
            .delete(url)
            .send()
            .await
            .map_err(quiet)
            .with_context(|| format!("R2 DELETE {key}"))?;
        match r.status() {
            s if s.is_success() || s == reqwest::StatusCode::NOT_FOUND => Ok(()),
            s => Err(anyhow::anyhow!("R2 DELETE answered {s}").into()),
        }
    }

    /// The `clips-check` probe: PUT, HEAD, GET and DELETE one small object through signed URLs.
    /// `Err` names the failing step and its HTTP status.
    pub async fn probe(&self) -> Result<(), String> {
        const BODY: &[u8] = b"pulse clips-chk\n";
        let key = format!("clips/check-{:016x}.bin", rand::random::<u64>());
        let ttl = Duration::from_secs(60);
        let step = |name: &str, e: reqwest::Error| format!("{name} failed: {}", quiet(e));
        let status = |name: &str, s: reqwest::StatusCode| format!("{name} failed: HTTP {s}");

        // The store must refuse a body that isn't the signed length: that is the cap's first line
        // of defence, and only the real store can prove it.
        let (url, _) = self.upload_url(&key, BODY.len() as u64, "application/octet-stream", ttl);
        let over = self
            .http
            .put(url)
            .header("content-type", "application/octet-stream")
            .body([BODY, b"!"].concat())
            .send()
            .await
            .map_err(|e| step("length check", e))?;
        if let Err(e) = length_check(over.status()) {
            let _ = self.delete(&key).await;
            return Err(e);
        }

        let (url, _) = self.upload_url(&key, BODY.len() as u64, "application/octet-stream", ttl);
        let r = self
            .http
            .put(url)
            .header("content-type", "application/octet-stream")
            .body(BODY)
            .send()
            .await
            .map_err(|e| step("PUT", e))?;
        if !r.status().is_success() {
            return Err(status("PUT", r.status()));
        }

        let result = async {
            let r = self
                .http
                .head(self.bucket.head_object(Some(&self.creds), &key).sign(ttl))
                .send()
                .await
                .map_err(|e| step("HEAD", e))?;
            if !r.status().is_success() {
                return Err(status("HEAD", r.status()));
            }
            let len = r
                .headers()
                .get(reqwest::header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok());
            if len != Some(&BODY.len().to_string()) {
                return Err(format!("HEAD failed: content-length was {len:?}"));
            }
            let (url, _) = self.download_url(&key, ttl);
            let r = self
                .http
                .get(url)
                .send()
                .await
                .map_err(|e| step("GET", e))?;
            if !r.status().is_success() {
                return Err(status("GET", r.status()));
            }
            if r.bytes().await.map_err(|e| step("GET", e))?.as_ref() != BODY {
                return Err("GET failed: the body came back different".into());
            }
            Ok(())
        }
        .await;

        // Always try to remove the probe, even after a failed step.
        let del = self
            .http
            .delete(self.bucket.delete_object(Some(&self.creds), &key).sign(ttl))
            .send()
            .await
            .map_err(|e| step("DELETE", e));
        result?;
        let del = del?;
        if !del.status().is_success() {
            return Err(status("DELETE", del.status()));
        }
        Ok(())
    }
}

/// The `clips-check` length step: a store that answers 2xx to a body longer than the signed
/// length doesn't enforce the cap. Only a 4xx is a refusal; a 5xx (an outage) or anything else
/// proves nothing.
fn length_check(status: reqwest::StatusCode) -> Result<(), String> {
    if status.is_success() {
        Err("length check failed: the store accepted a body longer than the signed length".into())
    } else if status.is_client_error() {
        Ok(())
    } else {
        Err(format!("length check failed: HTTP {status}"))
    }
}

/// When a URL signed for `ttl` stops working, in the API's timestamp shape: computed from the
/// URL's own `X-Amz-Date` (the signing instant, whole seconds), never from a second clock read.
fn expires_at(signed: &Url, ttl: Duration) -> String {
    let date = signed
        .query_pairs()
        .find(|(k, _)| k == "X-Amz-Date")
        .and_then(|(_, v)| chrono::NaiveDateTime::parse_from_str(&v, "%Y%m%dT%H%M%SZ").ok())
        .map(|d| d.and_utc())
        .unwrap_or_else(chrono::Utc::now);
    (date + ttl).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::fake_s3::{FAKE_BUCKET, FAKE_KEY_ID, FAKE_SECRET, FakeS3, FakeStore};
    use reqwest::StatusCode;

    const TTL: Duration = Duration::from_secs(3600);

    async fn setup() -> (FakeS3, R2) {
        let fake = FakeS3::start(FakeStore::Memory).await;
        let r2 = R2::new(&ClipStoreConfig {
            endpoint: fake.endpoint(),
            bucket: FAKE_BUCKET.into(),
            region: "auto".into(),
            key_id: FAKE_KEY_ID.into(),
            secret: FAKE_SECRET.into(),
        })
        .unwrap();
        (fake, r2)
    }

    #[tokio::test]
    async fn signed_put_then_head_get_delete_round_trip() {
        let (fake, r2) = setup().await;
        let http = reqwest::Client::new();
        let key = "clips/aa.mp4";
        let (url, expires) = r2.upload_url(key, 5, "video/mp4", TTL);
        assert!(expires.ends_with('Z') && expires.len() == 24, "{expires}");
        let r = http
            .put(url)
            .header("content-type", "video/mp4")
            .body(b"hello".to_vec())
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(fake.object(key).unwrap().0, b"hello");
        assert_eq!(fake.object(key).unwrap().1, "video/mp4");

        assert_eq!(r2.head(key).await.unwrap(), Some(5));
        let (url, _) = r2.download_url(key, TTL);
        assert_eq!(
            http.get(url).send().await.unwrap().bytes().await.unwrap(),
            "hello"
        );
        r2.delete(key).await.unwrap();
        assert!(fake.object(key).is_none());
        // Deleting what is already gone is fine.
        r2.delete(key).await.unwrap();
        assert_eq!(r2.head(key).await.unwrap(), None);
    }

    #[tokio::test]
    async fn upload_url_signs_the_exact_length() {
        let (fake, r2) = setup().await;
        let (url, _) = r2.upload_url("clips/bb.mp4", 5, "video/mp4", TTL);
        let signed = url
            .query_pairs()
            .find(|(k, _)| k == "X-Amz-SignedHeaders")
            .unwrap()
            .1
            .to_string();
        assert!(signed.contains("content-length"), "{signed}");
        assert!(signed.contains("content-type"), "{signed}");
        let r = reqwest::Client::new()
            .put(url)
            .header("content-type", "video/mp4")
            .body(b"too long".to_vec())
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        assert!(fake.keys().is_empty());
    }

    #[tokio::test]
    async fn head_of_a_missing_object_is_none() {
        let (_fake, r2) = setup().await;
        assert_eq!(r2.head("clips/missing.mp4").await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_broken_store_is_an_error_not_a_missing_object() {
        let (fake, r2) = setup().await;
        fake.set_down(true);
        assert!(r2.head("clips/x.mp4").await.is_err());
        assert!(r2.delete("clips/x.mp4").await.is_err());
    }

    #[tokio::test]
    async fn transport_errors_never_show_the_signed_url() {
        let (fake, r2) = setup().await;
        drop(fake);
        // Nothing listens any more: a transport error, not an HTTP status.
        for err in [
            r2.head("clips/x.mp4").await.unwrap_err(),
            r2.delete("clips/x.mp4").await.unwrap_err(),
        ] {
            for text in [format!("{err:?}"), format!("{err}")] {
                for secret in [FAKE_KEY_ID, "X-Amz-Signature", "X-Amz-Credential"] {
                    assert!(!text.contains(secret), "{secret} leaked: {text}");
                }
            }
        }
        let e = r2.head("clips/x.mp4").await.unwrap_err();
        assert!(format!("{e:?}").contains("R2 HEAD clips/x.mp4"), "{e:?}");
        let e = r2.delete("clips/x.mp4").await.unwrap_err();
        assert!(format!("{e:?}").contains("R2 DELETE clips/x.mp4"), "{e:?}");
        let probe = r2.probe().await.unwrap_err();
        assert!(probe.contains("failed"), "{probe}");
        for secret in [FAKE_KEY_ID, "X-Amz-Signature", "X-Amz-Credential"] {
            assert!(!probe.contains(secret), "{secret} leaked: {probe}");
        }
    }

    #[tokio::test]
    async fn server_requests_tolerate_clock_skew() {
        let (fake, r2) = setup().await;
        let http = reqwest::Client::new();
        let (url, _) = r2.upload_url("clips/cc.mp4", 2, "video/mp4", TTL);
        http.put(url)
            .header("content-type", "video/mp4")
            .body(b"hi".to_vec())
            .send()
            .await
            .unwrap();
        // The fake's clock runs a minute ahead of the signer.
        fake.set_now(Some(chrono::Utc::now().timestamp() + 60));
        assert_eq!(r2.head("clips/cc.mp4").await.unwrap(), Some(2));
        r2.delete("clips/cc.mp4").await.unwrap();
        assert!(fake.object("clips/cc.mp4").is_none());
    }

    #[tokio::test]
    async fn expires_at_matches_the_signature() {
        let (_fake, r2) = setup().await;
        let ttl = Duration::from_secs(900);
        let (put, put_exp) = r2.upload_url("clips/dd.mp4", 5, "video/mp4", ttl);
        let (get, get_exp) = r2.download_url("clips/dd.mp4", ttl);
        for (url, exp) in [(put, put_exp), (get, get_exp)] {
            let q = |name: &str| {
                url.query_pairs()
                    .find(|(k, _)| k == name)
                    .unwrap()
                    .1
                    .to_string()
            };
            let signed = chrono::NaiveDateTime::parse_from_str(&q("X-Amz-Date"), "%Y%m%dT%H%M%SZ")
                .unwrap()
                .and_utc();
            let secs: i64 = q("X-Amz-Expires").parse().unwrap();
            let want = (signed + chrono::Duration::seconds(secs))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            assert_eq!(exp, want);
            assert!(exp.ends_with(".000Z"), "{exp}");
        }
    }

    #[test]
    fn a_store_that_accepts_a_long_body_fails_the_length_check() {
        let e = length_check(StatusCode::OK).unwrap_err();
        assert!(e.starts_with("length check failed"), "{e}");
        assert!(length_check(StatusCode::FORBIDDEN).is_ok());
        assert!(length_check(StatusCode::BAD_REQUEST).is_ok());
    }

    #[test]
    fn a_store_outage_is_not_a_refusal() {
        for code in [
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::SERVICE_UNAVAILABLE,
        ] {
            let e = length_check(code).unwrap_err();
            assert_eq!(e, format!("length check failed: HTTP {code}"));
        }
        // a redirect proves nothing either
        assert!(length_check(StatusCode::TEMPORARY_REDIRECT).is_err());
    }

    #[test]
    fn object_keys_are_namespaced() {
        let hash = "ab".repeat(32);
        assert_eq!(
            R2::object_key(MediaKind::Clip, &hash),
            format!("clips/{hash}.mp4")
        );
    }
}
