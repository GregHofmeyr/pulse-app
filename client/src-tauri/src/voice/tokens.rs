//! Voice join tokens fetched ahead of the click (hovering a voice channel), each used once.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use pulse_protocol::ids::ChannelId;
use pulse_protocol::rest::VoiceTokenResponse;

use crate::api::Api;

/// The server's tokens last 10 min, but one only needs to be valid when the join starts. A
/// short shelf life keeps a token minted before a permission change from being used much later.
pub const FRESH_FOR: Duration = Duration::from_secs(120);

type Fetch = Shared<BoxFuture<'static, Option<VoiceTokenResponse>>>;

struct Entry {
    channel: ChannelId,
    /// Whose token it is: a different login must never reuse it.
    session: String,
    started: Instant,
    fetch: Fetch,
}

impl Entry {
    fn fits(&self, session: &str, channel: ChannelId, now: Instant) -> bool {
        self.channel == channel
            && self.session == session
            && now.saturating_duration_since(self.started) < FRESH_FOR
    }
}

/// One prefetched token (the channel last hovered): hover → click is the pattern it serves.
#[derive(Default)]
pub struct TokenCache {
    entry: Mutex<Option<Entry>>,
}

impl TokenCache {
    /// Start fetching `channel`'s token unless a fresh one is already here or on its way.
    pub fn prefetch(&self, api: &Api, session: &str, channel: ChannelId, now: Instant) {
        let mut slot = self.entry.lock().unwrap();
        if slot.as_ref().is_some_and(|e| e.fits(session, channel, now)) {
            return;
        }
        let (api, token) = (api.clone(), session.to_string());
        let fetch = async move { api.voice_token(&token, channel).await.ok() }
            .boxed()
            .shared();
        tokio::spawn(fetch.clone()); // shared futures are lazy: run it now, not at the click
        *slot = Some(Entry {
            channel,
            session: session.to_string(),
            started: now,
            fetch,
        });
    }

    /// The prefetched token for `channel`, waiting for it if still in flight. Used once; `None`
    /// if there's none (or it failed): fetch normally.
    pub async fn take(
        &self,
        session: &str,
        channel: ChannelId,
        now: Instant,
    ) -> Option<VoiceTokenResponse> {
        let fetch = {
            let mut slot = self.entry.lock().unwrap();
            if !slot.as_ref().is_some_and(|e| e.fits(session, channel, now)) {
                return None;
            }
            slot.take()?.fetch
        };
        fetch.await
    }

    pub fn clear(&self) {
        self.entry.lock().unwrap().take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulse_server::testing;

    async fn setup() -> (testing::TestApp, Api, String, ChannelId, ChannelId) {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let text = testing::general(&app, &token, s.id).await.id;
        let api = Api::new(&format!("http://{}", app.addr));
        let lounge = api
            .channels(&token, s.id)
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.kind == pulse_protocol::rest::ChannelKind::Voice)
            .unwrap()
            .id;
        (app, api, token, lounge, text)
    }

    #[tokio::test]
    async fn a_prefetched_token_is_used_once() {
        let (_app, api, token, lounge, _) = setup().await;
        let c = TokenCache::default();
        let now = Instant::now();
        c.prefetch(&api, &token, lounge, now);
        let t = c.take(&token, lounge, now).await.expect("prefetched");
        assert!(!t.token.is_empty());
        assert!(
            c.take(&token, lounge, now).await.is_none(),
            "a token is used once"
        );
    }

    #[tokio::test]
    async fn a_click_during_the_prefetch_waits_for_it() {
        let (_app, api, token, lounge, _) = setup().await;
        let c = TokenCache::default();
        let now = Instant::now();
        c.prefetch(&api, &token, lounge, now);
        // no yield in between: the request is still in flight
        assert!(c.take(&token, lounge, now).await.is_some());
    }

    #[tokio::test]
    async fn stale_foreign_or_other_channel_prefetches_are_not_used() {
        let (_app, api, token, lounge, text) = setup().await;
        let c = TokenCache::default();
        let now = Instant::now();
        c.prefetch(&api, &token, lounge, now);
        assert!(c.take(&token, text, now).await.is_none(), "other channel");
        assert!(
            c.take("someone-else", lounge, now).await.is_none(),
            "other session"
        );
        let late = now + FRESH_FOR + Duration::from_secs(1);
        assert!(c.take(&token, lounge, late).await.is_none(), "stale");
    }

    #[tokio::test]
    async fn a_cleared_cache_gives_nothing() {
        let (_app, api, token, lounge, _) = setup().await;
        let c = TokenCache::default();
        let now = Instant::now();
        c.prefetch(&api, &token, lounge, now);
        c.clear();
        assert!(
            c.take(&token, lounge, now).await.is_none(),
            "cleared on sign-out"
        );
    }

    /// A prefetch that failed (here: a text channel has no voice) just means a normal fetch.
    #[tokio::test]
    async fn a_failed_prefetch_gives_nothing() {
        let (_app, api, token, _, text) = setup().await;
        let c = TokenCache::default();
        let now = Instant::now();
        c.prefetch(&api, &token, text, now);
        assert!(c.take(&token, text, now).await.is_none());
    }
}
