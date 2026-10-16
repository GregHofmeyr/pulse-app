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

    #[test]
    fn about_a_minute_of_attempts_on_the_gateway_schedule() {
        let total: f64 = (0..MAX_ATTEMPTS).map(|a| delay(a, 0.5).as_secs_f64()).sum();
        assert_eq!(total, 1.0 + 2.0 + 4.0 + 8.0 + 16.0 + 30.0);
    }
}
