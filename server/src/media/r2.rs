//! The media store's remote location: clip videos in an S3-compatible bucket (Cloudflare R2).
//! The server never touches the video bytes. It signs PUT and GET URLs for clients with
//! `rusty-s3` (sans-IO SigV4) and runs HEAD and DELETE itself.

use std::time::Duration;

use anyhow::Context;
use reqwest::Url;
use rusty_s3::actions::S3Action;
use rusty_s3::{Bucket, Credentials, UrlStyle};

use super::MediaKind;
use crate::config::ClipStoreConfig;
use crate::error::AppResult;

const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

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
        (put.sign(ttl), expires_at(ttl))
    }

    pub fn download_url(&self, key: &str, ttl: Duration) -> (Url, String) {
        let get = self.bucket.get_object(Some(&self.creds), key);
        (get.sign(ttl), expires_at(ttl))
    }

    /// The object's size, or `None` if it isn't there.
    pub async fn head(&self, key: &str) -> AppResult<Option<u64>> {
        let url = self
            .bucket
            .head_object(Some(&self.creds), key)
            .sign(HTTP_TIMEOUT);
        let r = self.http.head(url).send().await.context("R2 HEAD")?;
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
            .sign(HTTP_TIMEOUT);
        let r = self.http.delete(url).send().await.context("R2 DELETE")?;
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
        let step = |name: &str, e: reqwest::Error| format!("{name} failed: {e}");
        let status = |name: &str, s: reqwest::StatusCode| format!("{name} failed: HTTP {s}");

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

/// When a URL signed now for `ttl` stops working, in the API's timestamp shape.
fn expires_at(ttl: Duration) -> String {
    (chrono::Utc::now() + ttl).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
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

    #[test]
    fn object_keys_are_namespaced() {
        let hash = "ab".repeat(32);
        assert_eq!(
            R2::object_key(MediaKind::Clip, &hash),
            format!("clips/{hash}.mp4")
        );
    }
}
