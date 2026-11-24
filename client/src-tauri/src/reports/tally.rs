//! Session tallies for bug reports (#605): WARN/ERROR log lines by target (a tracing layer beside the file writer),
//! and the recent network changes and gateway reconnects. Kept in memory for the process's life, small and capped:
//! a long log's problems show at the top of the report instead of somewhere in 256 KB.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use serde::Serialize;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

/// Targets tracked by name; lines from any more are counted under [`OTHER`].
const MAX_TARGETS: usize = 64;
const OTHER: &str = "(other)";
/// How many targets a report lists (worst first).
const LISTED: usize = 12;
/// How much of a target's latest message a report quotes.
const MESSAGE_CHARS: usize = 120;
/// Gateway reconnects remembered (newest last).
const RECONNECTS_KEPT: usize = 8;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Entry {
    warn: u32,
    error: u32,
    last: String,
}

/// WARN and ERROR lines so far, by target.
#[derive(Debug, Default)]
pub struct Warnings {
    by_target: BTreeMap<String, Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TargetCount {
    pub target: String,
    pub warn: u32,
    pub error: u32,
    /// The target's most recent WARN/ERROR message, cut short.
    pub last: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WarningSummary {
    pub warn: u32,
    pub error: u32,
    /// Worst first (errors, then warnings); at most [`LISTED`].
    pub top: Vec<TargetCount>,
    /// Targets with warnings or errors that didn't make the list.
    pub more_targets: usize,
}

impl Warnings {
    pub fn record(&mut self, target: &str, level: Level, message: &str) {
        let key = if self.by_target.contains_key(target) || self.by_target.len() < MAX_TARGETS {
            target
        } else {
            OTHER
        };
        let e = self.by_target.entry(key.to_string()).or_default();
        if level == Level::ERROR {
            e.error = e.error.saturating_add(1);
        } else {
            e.warn = e.warn.saturating_add(1);
        }
        e.last = cut(message, MESSAGE_CHARS);
    }

    pub fn summary(&self) -> WarningSummary {
        let mut all: Vec<TargetCount> = self
            .by_target
            .iter()
            .map(|(t, e)| TargetCount {
                target: t.clone(),
                warn: e.warn,
                error: e.error,
                last: e.last.clone(),
            })
            .collect();
        all.sort_by_key(|t| std::cmp::Reverse((t.error, t.warn)));
        let (warn, error) = all.iter().fold((0u32, 0u32), |(w, e), t| {
            (w.saturating_add(t.warn), e.saturating_add(t.error))
        });
        let more_targets = all.len().saturating_sub(LISTED);
        all.truncate(LISTED);
        WarningSummary {
            warn,
            error,
            top: all,
            more_targets,
        }
    }
}

/// `s` on one line, at most `max` characters (an ellipsis says it was cut).
fn cut(s: &str, max: usize) -> String {
    let one: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let one = one.trim();
    match one.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &one[..i]),
        None => one.to_string(),
    }
}

static WARNINGS: Mutex<Warnings> = Mutex::new(Warnings {
    by_target: BTreeMap::new(),
});

pub fn warnings() -> WarningSummary {
    WARNINGS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .summary()
}

/// The tracing layer that feeds [`warnings`]. It sees what the log's filter lets through, so its counts match the
/// file.
pub struct WarnTally;

impl<S: Subscriber> Layer<S> for WarnTally {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        // ERROR < WARN < INFO in tracing's order
        if *meta.level() > Level::WARN {
            return;
        }
        let mut m = Message(String::new());
        event.record(&mut m);
        WARNINGS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .record(meta.target(), *meta.level(), &m.0);
    }
}

struct Message(String);

impl Visit for Message {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_string();
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

// ---- network and reconnects ----

#[derive(Debug, Default)]
pub struct NetEvents {
    changes: u32,
    resumes: u32,
    gateway_reconnects: u32,
    /// (when, why, on which network), newest last.
    recent: VecDeque<(Instant, &'static str, Option<String>)>,
    /// (when, resumed, the network after it), newest last.
    recent_changes: VecDeque<(Instant, bool, Option<String>)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reconnect {
    pub ago_s: u64,
    pub reason: &'static str,
    /// The masked network label it happened on (`network xxxx`), if known.
    pub network: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NetChange {
    pub ago_s: u64,
    pub resumed: bool,
    /// The masked network label after the change: a different one from the last means it moved networks.
    pub network: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NetSummary {
    /// Real network changes netwatch saw this session (a route that moved, or a resume from sleep).
    pub changes: u32,
    pub resumes: u32,
    /// The last few, newest last.
    pub recent_changes: Vec<NetChange>,
    pub gateway_reconnects: u32,
    /// The last few, newest last.
    pub recent_gateway_reconnects: Vec<Reconnect>,
}

impl NetEvents {
    pub fn network_change(&mut self, resumed: bool, network: Option<String>, now: Instant) {
        self.changes = self.changes.saturating_add(1);
        if resumed {
            self.resumes = self.resumes.saturating_add(1);
        }
        if self.recent_changes.len() == RECONNECTS_KEPT {
            self.recent_changes.pop_front();
        }
        self.recent_changes.push_back((now, resumed, network));
    }

    pub fn gateway_reconnect(
        &mut self,
        reason: &'static str,
        network: Option<String>,
        now: Instant,
    ) {
        self.gateway_reconnects = self.gateway_reconnects.saturating_add(1);
        if self.recent.len() == RECONNECTS_KEPT {
            self.recent.pop_front();
        }
        self.recent.push_back((now, reason, network));
    }

    pub fn summary(&self, now: Instant) -> NetSummary {
        let ago = |t: Instant| now.saturating_duration_since(t).as_secs();
        NetSummary {
            changes: self.changes,
            resumes: self.resumes,
            recent_changes: self
                .recent_changes
                .iter()
                .map(|(t, resumed, network)| NetChange {
                    ago_s: ago(*t),
                    resumed: *resumed,
                    network: network.clone(),
                })
                .collect(),
            gateway_reconnects: self.gateway_reconnects,
            recent_gateway_reconnects: self
                .recent
                .iter()
                .map(|(t, reason, network)| Reconnect {
                    ago_s: ago(*t),
                    reason,
                    network: network.clone(),
                })
                .collect(),
        }
    }
}

static NET: Mutex<Option<NetEvents>> = Mutex::new(None);

fn with_net<T>(f: impl FnOnce(&mut NetEvents) -> T) -> T {
    let mut g = NET.lock().unwrap_or_else(PoisonError::into_inner);
    f(g.get_or_insert_with(NetEvents::default))
}

/// netwatch saw a real change (`resumed`: the machine woke from sleep in it); `network`: the masked label after it.
pub fn network_change(resumed: bool, network: Option<String>) {
    with_net(|n| n.network_change(resumed, network, Instant::now()));
}

/// The gateway is about to reconnect, and why (a fixed word: no addresses or server text). Tagged with the masked
/// label of the network netwatch last saw.
pub fn gateway_reconnect(reason: &'static str) {
    let network = crate::netwatch::path::current_network();
    with_net(|n| n.gateway_reconnect(reason, network, Instant::now()));
}

pub fn net() -> NetSummary {
    with_net(|n| n.summary(Instant::now()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tracing_subscriber::layer::SubscriberExt;

    #[test]
    fn counts_by_target_worst_first() {
        let mut w = Warnings::default();
        w.record(
            "pulse_client::gateway",
            Level::WARN,
            "gateway: no answer in time",
        );
        w.record(
            "pulse_client::gateway",
            Level::WARN,
            "gateway: no answer in time; again",
        );
        w.record(
            "pulse_client::voice",
            Level::ERROR,
            "voice: reopening a device failed",
        );
        w.record("share", Level::WARN, "share: 3 lines dropped");
        let s = w.summary();
        assert_eq!((s.warn, s.error), (3, 1));
        assert_eq!(s.top[0].target, "pulse_client::voice");
        assert_eq!(s.top[1].target, "pulse_client::gateway");
        assert_eq!(s.top[1].warn, 2);
        assert_eq!(s.top[1].last, "gateway: no answer in time; again");
        assert_eq!(s.more_targets, 0);
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["top"][0]["error"], 1);
        assert_eq!(v["warn"], 3);
    }

    #[test]
    fn targets_and_messages_are_capped() {
        let mut w = Warnings::default();
        for i in 0..MAX_TARGETS + 10 {
            w.record(&format!("t{i}"), Level::WARN, "x");
        }
        assert_eq!(w.by_target.len(), MAX_TARGETS + 1);
        assert_eq!(w.by_target[OTHER].warn, 10);
        let s = w.summary();
        assert_eq!(s.top.len(), LISTED);
        assert_eq!(s.more_targets, MAX_TARGETS + 1 - LISTED);
        assert_eq!(s.warn, (MAX_TARGETS + 10) as u32);

        let long = "é".repeat(500) + "\nsecond line";
        w.record("t0", Level::ERROR, &long);
        let last = &w.by_target["t0"].last;
        assert_eq!(last.chars().count(), MESSAGE_CHARS + 1);
        assert!(last.ends_with('…') && !last.contains('\n'));
    }

    #[test]
    fn the_layer_counts_warn_and_error_only() {
        let mut before = warnings();
        let sub = tracing_subscriber::registry().with(WarnTally);
        tracing::subscriber::with_default(sub, || {
            tracing::info!(target: "tally_test", "fine");
            tracing::warn!(target: "tally_test", n = 3, "something odd");
            tracing::error!(target: "tally_test", "broke: {}", "badly");
        });
        let after = warnings();
        let find = |s: &WarningSummary| s.top.iter().find(|t| t.target == "tally_test").cloned();
        let t = find(&after).expect("tallied");
        assert_eq!((t.warn, t.error), (1, 1));
        assert_eq!(t.last, "broke: badly");
        before.top.retain(|t| t.target == "tally_test");
        assert!(before.top.is_empty());
    }

    #[test]
    fn network_events_keep_the_last_few_reconnects() {
        let t0 = Instant::now();
        let mut n = NetEvents::default();
        n.network_change(false, Some("network a3f9".into()), t0);
        n.network_change(
            true,
            Some("network 07c2".into()),
            t0 + Duration::from_secs(5),
        );
        for i in 0..RECONNECTS_KEPT + 2 {
            let reason = if i == RECONNECTS_KEPT + 1 {
                "server_restart"
            } else {
                "dropped"
            };
            n.gateway_reconnect(
                reason,
                Some("network 07c2".into()),
                t0 + Duration::from_secs(i as u64),
            );
        }
        let s = n.summary(t0 + Duration::from_secs(60));
        assert_eq!((s.changes, s.resumes), (2, 1));
        assert_eq!(s.recent_changes.len(), 2);
        assert_eq!(s.recent_changes[1].ago_s, 55);
        assert!(s.recent_changes[1].resumed);
        assert_eq!(s.recent_changes[0].network.as_deref(), Some("network a3f9"));
        assert_eq!(s.recent_changes[1].network.as_deref(), Some("network 07c2"));
        assert_eq!(s.gateway_reconnects, (RECONNECTS_KEPT + 2) as u32);
        assert_eq!(s.recent_gateway_reconnects.len(), RECONNECTS_KEPT);
        let last = s.recent_gateway_reconnects.last().unwrap();
        assert_eq!(last.reason, "server_restart");
        assert_eq!(last.network.as_deref(), Some("network 07c2"));
        assert_eq!(last.ago_s, 60 - (RECONNECTS_KEPT as u64 + 1));
    }
}
