//! Steam account linking and live achievements (#379; design: `docs/superpowers/specs/2026-11-16-steam-achievements-design.md`).
//!
//! - [`openid`] + [`routes`]: Settings links a Steam account with Steam's OpenID 2.0 sign-in in the browser.
//! - [`poller`]: while a linked person plays a Steam game (and for a while after), their achievements are read every
//!   couple of minutes; new unlocks become cards ([`cards`]) in each server's Achievements channel.
//! - [`backfill`]: once per link, their played games are read slowly, so old unlocks are never announced and the
//!   completion rate has something to average.
//!
//! In memory here: the sign-ins in progress, which game each person plays for the poller, and who's backed off.

pub mod achievements;
pub mod api;
pub mod backfill;
pub mod cards;
pub mod hours;
pub mod openid;
pub mod poller;
pub mod routes;
pub mod store;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pulse_protocol::ids::UserId;
use sha2::{Digest, Sha256};

use crate::config::SteamConfig;

/// How long a person is left alone after Steam says their game details are private.
pub const PRIVATE_BACKOFF: Duration = Duration::from_secs(3600);
/// How long after any other failure.
pub const ERROR_BACKOFF: Duration = Duration::from_secs(600);

#[derive(Clone)]
pub struct Steam {
    inner: Arc<Inner>,
}

struct Inner {
    api: api::Api,
    ttl: Duration,
    /// Sign-ins in progress: the state token's hash → whose, and when it was made.
    states: Mutex<HashMap<String, (UserId, Instant)>>,
    /// The app id each person's client last named for a game, by name (kept even with "Show game art" off).
    notes: Mutex<HashMap<UserId, (String, u32)>>,
    tracks: Mutex<achievements::Tracks>,
    /// Who isn't polled until when, and the games that have no stats for them.
    backoff: Mutex<HashMap<UserId, Instant>>,
    no_stats: Mutex<HashMap<(UserId, u32), Instant>>,
    /// One backfill at a time, server-wide.
    backfill: tokio::sync::Mutex<()>,
}

impl Steam {
    pub fn new(cfg: &SteamConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                api: api::Api::new(cfg),
                ttl: cfg.link_ttl,
                states: Mutex::default(),
                notes: Mutex::default(),
                tracks: Mutex::default(),
                backoff: Mutex::default(),
                no_stats: Mutex::default(),
                backfill: tokio::sync::Mutex::new(()),
            }),
        }
    }

    pub fn api(&self) -> &api::Api {
        &self.inner.api
    }

    /// A new one-time sign-in state for `user` (kept hashed; expired ones are dropped).
    pub fn new_state(&self, user: UserId) -> String {
        let token = hex::encode(rand::random::<[u8; 32]>());
        let mut states = self.inner.states.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        states.retain(|_, (_, at)| now.duration_since(*at) < self.inner.ttl);
        states.insert(hash(&token), (user, now));
        token
    }

    /// Whose sign-in `state` is, using it up; `None` when unknown or expired.
    pub fn take_state(&self, state: &str) -> Option<UserId> {
        let mut states = self.inner.states.lock().unwrap_or_else(|e| e.into_inner());
        let (user, at) = states.remove(&hash(state))?;
        (at.elapsed() < self.inner.ttl).then_some(user)
    }

    /// `user`'s client named `app_id` for the game called `name` (the poller's source when the shown "Playing"
    /// carries no id because art is hidden).
    pub fn note_playing(&self, user: UserId, name: &str, app_id: u32) {
        let mut notes = self.inner.notes.lock().unwrap_or_else(|e| e.into_inner());
        notes.insert(user, (name.to_string(), app_id));
    }

    /// The Steam app id of what `user` plays now, from their shown "Playing" or, when that hides it, the app id their
    /// client named for the same game.
    pub fn app_of(
        &self,
        user: UserId,
        playing: Option<&pulse_protocol::rest::Playing>,
    ) -> Option<u32> {
        let p = playing?;
        if let Some(id) = p.steam_app_id {
            return Some(id);
        }
        let notes = self.inner.notes.lock().unwrap_or_else(|e| e.into_inner());
        notes
            .get(&user)
            .filter(|(name, _)| *name == p.name)
            .map(|(_, id)| *id)
    }

    fn observe(
        &self,
        user: UserId,
        playing: Option<(u32, i64)>,
        tail: Duration,
    ) -> Option<(u32, i64)> {
        let mut t = self.inner.tracks.lock().unwrap_or_else(|e| e.into_inner());
        t.observe(user, playing, Instant::now(), tail)
    }

    /// The game the poller follows for `user` now (playing, or within the tail).
    pub fn tracked(&self, user: UserId) -> Option<u32> {
        let t = self.inner.tracks.lock().unwrap_or_else(|e| e.into_inner());
        t.current(user)
    }

    pub fn back_off(&self, user: UserId, for_: Duration) {
        let mut b = self.inner.backoff.lock().unwrap_or_else(|e| e.into_inner());
        b.insert(user, Instant::now() + for_);
    }

    pub fn backed_off(&self, user: UserId) -> bool {
        let mut b = self.inner.backoff.lock().unwrap_or_else(|e| e.into_inner());
        match b.get(&user) {
            Some(until) if Instant::now() < *until => true,
            Some(_) => {
                b.remove(&user);
                false
            }
            None => false,
        }
    }

    fn mark_no_stats(&self, user: UserId, app: u32) {
        let mut n = self
            .inner
            .no_stats
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        n.insert((user, app), Instant::now() + Duration::from_secs(24 * 3600));
    }

    fn has_no_stats(&self, user: UserId, app: u32) -> bool {
        let n = self
            .inner
            .no_stats
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        n.get(&(user, app))
            .is_some_and(|until| Instant::now() < *until)
    }

    /// Forget everything in memory about `user` (they unlinked).
    pub fn forget(&self, user: UserId) {
        self.inner
            .tracks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .forget(user);
        self.inner
            .backoff
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&user);
        self.inner
            .no_stats
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(u, _), _| *u != user);
    }
}

fn hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Text from Steam as one plain line: control characters out, whitespace runs as one space, at most `max`
/// characters; `None` when nothing's left.
pub fn one_line(raw: &str, max: usize) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let joined = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = joined.chars().take(max).collect();
    let cut = cut.trim_end().to_string();
    (!cut.is_empty()).then_some(cut)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_works_once_and_only_for_its_owner() {
        let s = Steam::new(&SteamConfig::default());
        let u = UserId::new();
        let st = s.new_state(u);
        assert_eq!(st.len(), 64);
        assert_eq!(s.take_state("nope"), None);
        assert_eq!(s.take_state(&st), Some(u));
        assert_eq!(s.take_state(&st), None, "one use");
    }

    #[test]
    fn an_expired_state_is_refused() {
        let s = Steam::new(&SteamConfig {
            link_ttl: Duration::ZERO,
            ..SteamConfig::default()
        });
        let st = s.new_state(UserId::new());
        assert_eq!(s.take_state(&st), None);
    }

    #[test]
    fn the_app_id_comes_from_the_shown_game_or_the_note_for_it() {
        use pulse_protocol::rest::Playing;
        let s = Steam::new(&SteamConfig::default());
        let u = UserId::new();
        let game = |name: &str, id: Option<u32>| Playing {
            name: name.into(),
            steam_app_id: id,
            started_at: None,
            art: None,
        };
        assert_eq!(s.app_of(u, None), None);
        assert_eq!(s.app_of(u, Some(&game("Space Game", Some(10)))), Some(10));
        assert_eq!(s.app_of(u, Some(&game("Space Game", None))), None);
        s.note_playing(u, "Space Game", 10);
        assert_eq!(s.app_of(u, Some(&game("Space Game", None))), Some(10));
        assert_eq!(
            s.app_of(u, Some(&game("Other", None))),
            None,
            "a different game"
        );
    }

    #[test]
    fn one_line_cleans_steam_text() {
        assert_eq!(one_line(" a\u{7}\n b ", 10).as_deref(), Some("a b"));
        assert_eq!(one_line("   ", 10), None);
        assert_eq!(one_line("abcdef", 3).as_deref(), Some("abc"));
    }
}
