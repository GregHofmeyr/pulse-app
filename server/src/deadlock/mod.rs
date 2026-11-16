//! Deadlock (#390): post-match cards, rank badges and the weekly rank movers, squad stats and patch notes, from
//! deadlock-api.com (free, unofficial, open source). Valve offers no stats API for Deadlock; everything is keyed by the
//! Steam account people linked (#379): Deadlock's account id is the SteamID3, `SteamID64 − 76561197960265728`.
//!
//! - [`api`]: the HTTP client: an optional key, a descriptive User-Agent, a per-hour allowance for the rate-limited
//!   match history, and backoff on 429/5xx. When deadlock-api goes dark it's logged once and every Deadlock bit hides
//!   ([`api::Api::available`]) until it answers again.
//! - [`assets`]: hero and rank names and art (fetched by the server from an allowlist of hosts; clients never call
//!   third parties).
//! - [`matches`], [`ranks`], [`patches`]: what the [`poller`] does each round; [`squad`]: a profile's Deadlock section.
//! - [`routes`]: each server's Deadlock channel, the member list's ranks, a member's Deadlock section.

pub mod api;
pub mod assets;
pub mod matches;
pub mod patches;
pub mod poller;
pub mod ranks;
pub mod routes;
pub mod squad;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{DEADLOCK_APP_ID, Playing};

use crate::config::DeadlockConfig;

/// `SteamID64 − this` is the SteamID3 (Deadlock's account id).
pub const STEAM_ID64_BASE: u64 = 76_561_197_960_265_728;
/// How long after someone was last seen playing Deadlock their match history is still read often.
pub const ACTIVE_TAIL: Duration = Duration::from_secs(2 * 3600);

/// The SteamID3 of a 64-bit SteamID (digits), counted from `base` ([`STEAM_ID64_BASE`]; tests use their own so fake
/// ids convert); `None` when it isn't one.
pub fn account_id(steam_id: &str, base: u64) -> Option<u32> {
    let id: u64 = steam_id.trim().parse().ok()?;
    let a = id.checked_sub(base)?;
    // deadlock-api takes an int32
    u32::try_from(a)
        .ok()
        .filter(|a| *a > 0 && *a <= i32::MAX as u32)
}

/// Whether `playing` is Deadlock, by Steam app id (shown, or noted for the server) or by name.
pub fn is_deadlock(app_id: Option<u32>, playing: Option<&Playing>) -> bool {
    app_id == Some(DEADLOCK_APP_ID)
        || playing.is_some_and(|p| p.name.trim().eq_ignore_ascii_case("deadlock"))
}

#[derive(Clone)]
pub struct Deadlock {
    inner: Arc<Inner>,
}

struct Inner {
    api: api::Api,
    base: u64,
    assets: assets::Assets,
    /// When each person's match history was last read, and when they were last seen playing.
    checked: Mutex<HashMap<UserId, Instant>>,
    played: Mutex<HashMap<UserId, Instant>>,
    /// Match history reads pause until then (deadlock-api said 429 for it).
    history_pause: Mutex<Option<Instant>>,
    squad: squad::Cache,
}

impl Deadlock {
    pub fn new(cfg: &DeadlockConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                api: api::Api::new(cfg),
                base: cfg.steam_id_base,
                assets: assets::Assets::new(cfg),
                checked: Mutex::default(),
                played: Mutex::default(),
                history_pause: Mutex::default(),
                squad: squad::Cache::default(),
            }),
        }
    }

    pub fn api(&self) -> &api::Api {
        &self.inner.api
    }

    pub fn assets(&self) -> &assets::Assets {
        &self.inner.assets
    }

    pub(crate) fn squad_cache(&self) -> &squad::Cache {
        &self.inner.squad
    }

    /// `steam_id`'s Deadlock account id.
    pub fn account_of(&self, steam_id: &str) -> Option<u32> {
        account_id(steam_id, self.inner.base)
    }

    /// `user` is playing Deadlock now.
    pub fn saw_playing(&self, user: UserId) {
        let mut p = self.inner.played.lock().unwrap_or_else(|e| e.into_inner());
        p.insert(user, Instant::now());
    }

    /// Whether `user` played Deadlock within [`ACTIVE_TAIL`].
    pub fn recently_played(&self, user: UserId) -> bool {
        let p = self.inner.played.lock().unwrap_or_else(|e| e.into_inner());
        p.get(&user).is_some_and(|t| t.elapsed() < ACTIVE_TAIL)
    }

    pub fn last_checked(&self, user: UserId) -> Option<Instant> {
        let c = self.inner.checked.lock().unwrap_or_else(|e| e.into_inner());
        c.get(&user).copied()
    }

    pub fn mark_checked(&self, user: UserId) {
        let mut c = self.inner.checked.lock().unwrap_or_else(|e| e.into_inner());
        c.insert(user, Instant::now());
    }

    pub fn pause_history(&self, for_: Duration) {
        let mut p = self
            .inner
            .history_pause
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *p = Some(Instant::now() + for_);
    }

    pub fn history_paused(&self) -> bool {
        let p = self
            .inner
            .history_pause
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        p.is_some_and(|until| Instant::now() < until)
    }

    /// Forget what's in memory about `user` (they unlinked or turned a switch off).
    pub fn forget(&self, user: UserId) {
        self.inner
            .checked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&user);
    }
}

/// HTML (a patch's notes) as plain text: tags out, the common entities decoded, whitespace runs as one space.
pub fn plain_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let decoded = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#039;", "'")
        .replace("&amp;", "&");
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// At most `max` characters of `text`, cut at a word and marked with "…" when cut; `None` when empty.
pub fn excerpt(text: &str, max: usize) -> Option<String> {
    let text = crate::steam::one_line(text, usize::MAX)?;
    if text.chars().count() <= max {
        return Some(text);
    }
    let cut: String = text.chars().take(max).collect();
    let cut = match cut.rfind(' ') {
        Some(i) if i > max / 2 => cut[..i].to_string(),
        _ => cut,
    };
    Some(format!(
        "{}…",
        cut.trim_end_matches([',', '.', ';', ':', ' '])
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steam_ids_convert_to_account_ids() {
        assert_eq!(account_id("76561197960265729", STEAM_ID64_BASE), Some(1));
        assert_eq!(
            account_id("76561197960265728", STEAM_ID64_BASE),
            None,
            "zero"
        );
        assert_eq!(
            account_id("76500000000000001", STEAM_ID64_BASE),
            None,
            "below the base"
        );
        assert_eq!(
            account_id("76500000000000007", 76_500_000_000_000_000),
            Some(7)
        );
        assert_eq!(account_id("nope", STEAM_ID64_BASE), None);
        // above int32: deadlock-api can't take it
        assert_eq!(
            account_id(
                &(STEAM_ID64_BASE + (1u64 << 31)).to_string(),
                STEAM_ID64_BASE
            ),
            None
        );
    }

    #[test]
    fn deadlock_by_app_id_or_name() {
        let p = |name: &str| Playing {
            name: name.into(),
            steam_app_id: None,
            started_at: None,
            art: None,
            join: None,
        };
        assert!(is_deadlock(Some(DEADLOCK_APP_ID), None));
        assert!(is_deadlock(None, Some(&p("Deadlock"))));
        assert!(!is_deadlock(Some(570), Some(&p("Space Game"))));
        assert!(!is_deadlock(None, None));
    }

    #[test]
    fn html_becomes_a_short_plain_excerpt() {
        let t = plain_text(
            "<p>Haze:&nbsp;more <b>haze</b> &amp; smoke.</p><ul><li>Infernus: hotter</li></ul>",
        );
        assert_eq!(t, "Haze: more haze & smoke. Infernus: hotter");
        assert_eq!(excerpt(&t, 100).as_deref(), Some(t.as_str()));
        assert_eq!(excerpt(&t, 20).as_deref(), Some("Haze: more haze &…"));
        assert_eq!(excerpt("   ", 10), None);
    }

    #[test]
    fn history_pauses_and_playing_is_remembered() {
        let d = Deadlock::new(&DeadlockConfig::default());
        let u = UserId::new();
        assert!(!d.recently_played(u) && !d.history_paused());
        d.saw_playing(u);
        assert!(d.recently_played(u));
        d.pause_history(Duration::from_secs(60));
        assert!(d.history_paused());
        assert_eq!(d.last_checked(u), None);
        d.mark_checked(u);
        assert!(d.last_checked(u).is_some());
        d.forget(u);
        assert_eq!(d.last_checked(u), None);
    }
}
