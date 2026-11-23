//! What a call does when the network changes (#355) or a subscribed track stalls (#217). Pure decisions; the room
//! event loop (`mod.rs`) carries them out.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use livekit::ConnectionState;
use tokio::time::Instant;

/// At most one network-change rejoin or reconnect this often, across calls (a rejoin is a new call).
pub const MIN_GAP: Duration = Duration::from_secs(10);
/// A reconnect we asked for that hasn't come back by then is given up on: rejoin.
pub const RECONNECT_GRACE: Duration = Duration::from_secs(8);

/// What a call does about a network change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetAction {
    /// Nothing: the call is already over, or we acted moments ago.
    Ignore,
    /// LiveKit was already reconnecting over the old path: rejoin now instead of waiting out its timers.
    Rejoin,
    /// The room still looks connected: ask LiveKit to reconnect (a resume: new signal connection and an ICE
    /// restart), and rejoin if it isn't back within [`RECONNECT_GRACE`].
    Reconnect,
}

/// Decides, and takes the rate limit's slot if it acts.
pub fn decide(state: ConnectionState, limit: &Limiter, now: Instant) -> NetAction {
    let action = match state {
        ConnectionState::Disconnected => return NetAction::Ignore,
        ConnectionState::Reconnecting => NetAction::Rejoin,
        ConnectionState::Connected => NetAction::Reconnect,
    };
    if limit.take(now) {
        action
    } else {
        NetAction::Ignore
    }
}

/// The once-per-[`MIN_GAP`] limit, shared by every call of the manager.
#[derive(Default)]
pub struct Limiter(Mutex<Option<Instant>>);

impl Limiter {
    /// True (and the slot is taken) if nothing acted in the last [`MIN_GAP`].
    pub fn take(&self, now: Instant) -> bool {
        let mut last = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|t| now.saturating_duration_since(t) < MIN_GAP) {
            return false;
        }
        *last = Some(now);
        true
    }

    /// Something acted at `now` anyway (a reconnect given up on): the next change waits a full gap.
    pub fn mark(&self, now: Instant) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(now);
    }
}

/// Resubscribes per stalled track before leaving it alone for the rest of the call.
pub const RESUBSCRIBE_MAX: u32 = 3;
/// The wait before each further resubscribe of the same track.
const RESUBSCRIBE_BACKOFF: [Duration; 2] = [Duration::from_secs(30), Duration::from_secs(120)];

/// #217's simple self-heal: a subscribed track that stops handing us frames while the room is connected is
/// resubscribed, once per stall, with a growing wait between tries and [`RESUBSCRIBE_MAX`] in all. Per call.
#[derive(Default)]
pub struct Resubscribes(HashMap<String, (u32, Instant)>);

impl Resubscribes {
    /// Whether to resubscribe `track` (stalled at `now`); counts the try if so.
    pub fn allow(&mut self, track: &str, connected: bool, now: Instant) -> bool {
        if !connected {
            // the room itself is down: its own recovery (or ours) handles every track at once
            return false;
        }
        match self.0.get_mut(track) {
            None => {
                self.0.insert(track.to_owned(), (1, now));
                true
            }
            Some((n, last)) => {
                if *n >= RESUBSCRIBE_MAX {
                    return false;
                }
                let wait =
                    RESUBSCRIBE_BACKOFF[(*n as usize - 1).min(RESUBSCRIBE_BACKOFF.len() - 1)];
                if now.saturating_duration_since(*last) < wait {
                    return false;
                }
                *n += 1;
                *last = now;
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnecting_rejoins_connected_reconnects_gone_does_nothing() {
        let now = Instant::now();
        assert_eq!(
            decide(ConnectionState::Reconnecting, &Limiter::default(), now),
            NetAction::Rejoin
        );
        assert_eq!(
            decide(ConnectionState::Connected, &Limiter::default(), now),
            NetAction::Reconnect
        );
        assert_eq!(
            decide(ConnectionState::Disconnected, &Limiter::default(), now),
            NetAction::Ignore
        );
    }

    #[test]
    fn at_most_one_action_per_gap() {
        let t0 = Instant::now();
        let limit = Limiter::default();
        assert_eq!(
            decide(ConnectionState::Connected, &limit, t0),
            NetAction::Reconnect
        );
        assert_eq!(
            decide(
                ConnectionState::Reconnecting,
                &limit,
                t0 + Duration::from_secs(5)
            ),
            NetAction::Ignore
        );
        assert_eq!(
            decide(ConnectionState::Reconnecting, &limit, t0 + MIN_GAP),
            NetAction::Rejoin
        );
        // a call that's over takes no slot
        let limit = Limiter::default();
        decide(ConnectionState::Disconnected, &limit, t0);
        assert!(limit.take(t0));
    }

    #[test]
    fn a_reconnect_given_up_on_pushes_the_next_action_out() {
        let t0 = Instant::now();
        let limit = Limiter::default();
        assert!(limit.take(t0));
        limit.mark(t0 + RECONNECT_GRACE);
        assert!(!limit.take(t0 + MIN_GAP));
        assert!(limit.take(t0 + RECONNECT_GRACE + MIN_GAP));
    }

    #[test]
    fn the_grace_ends_before_the_rate_limit_would_allow_another_change() {
        assert!(RECONNECT_GRACE < MIN_GAP);
    }

    #[test]
    fn a_stalled_track_is_resubscribed_with_back_off_then_left_alone() {
        let t0 = Instant::now();
        let mut r = Resubscribes::default();
        assert!(r.allow("TR_a", true, t0));
        // stalled again soon after: wait
        assert!(!r.allow("TR_a", true, t0 + Duration::from_secs(10)));
        // another track has its own count
        assert!(r.allow("TR_b", true, t0 + Duration::from_secs(10)));
        assert!(r.allow("TR_a", true, t0 + Duration::from_secs(30)));
        assert!(!r.allow("TR_a", true, t0 + Duration::from_secs(100)));
        assert!(r.allow("TR_a", true, t0 + Duration::from_secs(150)));
        // out of tries for this call
        assert!(!r.allow("TR_a", true, t0 + Duration::from_secs(3600)));
    }

    #[test]
    fn no_resubscribe_while_the_room_is_down() {
        let mut r = Resubscribes::default();
        assert!(!r.allow("TR_a", false, Instant::now()));
        // and it didn't use up a try
        assert!(r.allow("TR_a", true, Instant::now()));
    }
}
