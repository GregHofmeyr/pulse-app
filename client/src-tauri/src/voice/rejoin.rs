//! Getting voice back after LiveKit gives up reconnecting (it tries for ~45 s on its own).

use std::time::Duration;

use livekit::DisconnectReason;

/// Rejoin attempts before giving up: with the delays below, about a minute more.
pub const MAX_ATTEMPTS: u32 = 6;

/// Whether losing the room this way is worth an automatic rejoin: not when we left, were removed,
/// were replaced by another login of ours, or the room is gone.
pub fn worth_rejoining(reason: DisconnectReason) -> bool {
    !matches!(
        reason,
        DisconnectReason::ClientInitiated
            | DisconnectReason::DuplicateIdentity
            | DisconnectReason::ParticipantRemoved
            | DisconnectReason::RoomDeleted
    )
}

/// Delay before rejoin attempt `attempt` (0-based): the gateway's schedule (1 s, 2 s, 4 s … ±20 %).
pub fn delay(attempt: u32, jitter: f64) -> Duration {
    crate::backoff::backoff_delay(attempt, jitter)
}

/// How long LiveKit may stay "reconnecting" before we give up on it ourselves and rejoin. Its own
/// retries end in ~45 s against a server that refuses, but against one that doesn't answer at all
/// (a black hole) a single attempt can hang for minutes.
pub const RECONNECT_GIVE_UP: Duration = Duration::from_secs(60);

/// Cuts a LiveKit reconnect short: armed on its first "reconnecting", disarmed on "reconnected".
pub struct GiveUpTimer {
    after: Duration,
    at: Option<tokio::time::Instant>,
}

impl GiveUpTimer {
    pub fn new(after: Duration) -> Self {
        Self { after, at: None }
    }

    pub fn after(&self) -> Duration {
        self.after
    }

    /// LiveKit is reconnecting: start the clock (a repeat doesn't restart it).
    pub fn arm(&mut self) {
        self.at
            .get_or_insert_with(|| tokio::time::Instant::now() + self.after);
    }

    /// Reconnected.
    pub fn disarm(&mut self) {
        self.at = None;
    }

    /// Resolves when the reconnect has gone on too long; never while not reconnecting.
    pub async fn fired(&self) {
        match self.at {
            Some(at) => tokio::time::sleep_until(at).await,
            None => std::future::pending().await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejoins_unless_we_left_were_removed_or_the_room_is_gone() {
        use DisconnectReason::*;
        for r in [
            UnknownReason,
            ServerShutdown,
            SignalClose,
            JoinFailure,
            ConnectionTimeout,
            MediaFailure,
            StateMismatch,
            RoomClosed,
        ] {
            assert!(worth_rejoining(r), "{r:?}");
        }
        for r in [
            ClientInitiated,
            DuplicateIdentity,
            ParticipantRemoved,
            RoomDeleted,
        ] {
            assert!(!worth_rejoining(r), "{r:?}");
        }
    }

    /// A LiveKit reconnect that never ends (a server that doesn't answer at all) is cut short.
    #[tokio::test]
    async fn a_long_reconnect_is_given_up_on() {
        let after = Duration::from_millis(50);
        let mut t = GiveUpTimer::new(after);
        // not reconnecting: never fires
        assert!(
            tokio::time::timeout(Duration::from_millis(150), t.fired())
                .await
                .is_err()
        );
        let start = tokio::time::Instant::now();
        t.arm();
        tokio::time::sleep(Duration::from_millis(30)).await;
        t.arm(); // another Reconnecting doesn't push the deadline out
        tokio::time::timeout(Duration::from_secs(1), t.fired())
            .await
            .expect("never fired");
        let took = start.elapsed();
        assert!(
            took >= after && took < Duration::from_millis(75),
            "{took:?}"
        );
    }

    /// Reconnected in time: the timer stands down.
    #[tokio::test]
    async fn reconnected_disarms_the_give_up_timer() {
        let mut t = GiveUpTimer::new(Duration::from_millis(50));
        t.arm();
        t.disarm();
        assert!(
            tokio::time::timeout(Duration::from_millis(150), t.fired())
                .await
                .is_err()
        );
    }

    #[test]
    fn the_give_up_outlasts_the_sdks_own_reconnecting() {
        assert!(RECONNECT_GIVE_UP > Duration::from_secs(45));
    }

    #[test]
    fn about_a_minute_of_attempts_on_the_gateway_schedule() {
        let total: f64 = (0..MAX_ATTEMPTS).map(|a| delay(a, 0.5).as_secs_f64()).sum();
        assert_eq!(total, 1.0 + 2.0 + 4.0 + 8.0 + 16.0 + 30.0);
    }
}
