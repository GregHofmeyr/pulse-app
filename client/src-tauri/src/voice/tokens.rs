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
/// How long a click waits for a prefetch still in flight. A token takes one round trip; one this
/// late is stuck (a dead connection, a server not answering), and a fresh fetch is the better bet
/// than waiting out the 15 s request timeout.
pub const PREFETCH_WAIT: Duration = Duration::from_secs(2);

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
            // finished and failed: nothing to wait for, so the next hover fetches again
            && !matches!(self.fetch.peek(), Some(None))
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
        self.take_within(session, channel, now, PREFETCH_WAIT).await
    }

    /// [`take`](Self::take), waiting at most `wait` for a prefetch still in flight.
    async fn take_within(
        &self,
        session: &str,
        channel: ChannelId,
        now: Instant,
        wait: Duration,
    ) -> Option<VoiceTokenResponse> {
        let fetch = {
            let mut slot = self.entry.lock().unwrap();
            if !slot.as_ref().is_some_and(|e| e.fits(session, channel, now)) {
                return None;
            }
            slot.take()?.fetch
        };
        tokio::time::timeout(wait, fetch).await.ok().flatten()
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

    /// A failed prefetch doesn't hold the slot for FRESH_FOR: hovering again fetches again.
    #[tokio::test]
    async fn a_failed_prefetch_is_fetched_again_on_the_next_hover() {
        let (_app, api, token, lounge, _) = setup().await;
        // nothing listens here: the first prefetch fails at once
        let dead = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            Api::new(&format!("http://{}", l.local_addr().unwrap()))
        };
        let c = TokenCache::default();
        let now = Instant::now();
        c.prefetch(&dead, &token, lounge, now);
        let failed = c.entry.lock().unwrap().as_ref().unwrap().fetch.clone();
        assert!(failed.await.is_none());
        c.prefetch(&api, &token, lounge, now);
        assert!(
            c.take(&token, lounge, now).await.is_some(),
            "not fetched again"
        );
    }

    /// A prefetch stuck on a server that never answers doesn't hold the click for the whole
    /// request timeout: past the wait, the join fetches normally.
    #[tokio::test]
    async fn a_click_waits_only_so_long_for_a_stuck_prefetch() {
        let (_app, _, token, lounge, _) = setup().await;
        // accepts (the OS backlog) but never answers
        let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let stuck = Api::new(&format!("http://{}", silent.local_addr().unwrap()));
        let c = TokenCache::default();
        let now = Instant::now();
        c.prefetch(&stuck, &token, lounge, now);
        let t0 = Instant::now();
        let got = c
            .take_within(&token, lounge, now, Duration::from_millis(200))
            .await;
        assert!(got.is_none());
        assert!(
            t0.elapsed() < Duration::from_secs(2),
            "waited {:?}",
            t0.elapsed()
        );
    }
}
