//! Reconnect backoff: 1 s, 2 s, 4 s … capped at 30 s. Voice uses ±20% jitter; the gateway, which every
//! client loses at once when the server restarts, uses full jitter so the reconnect wave is spread out.

use std::time::Duration;

const CAP_SECS: f64 = 30.0;

/// Delay before reconnect attempt `attempt` (0-based). `jitter` in 0..=1 maps to ×0.8..×1.2.
pub fn backoff_delay(attempt: u32, jitter: f64) -> Duration {
    Duration::from_secs_f64(ceiling(attempt) * (0.8 + 0.4 * jitter.clamp(0.0, 1.0)))
}

/// Ceiling of reconnect attempt `attempt` (0-based): 1 s, 2 s, 4 s … capped at 30 s.
fn ceiling(attempt: u32) -> f64 {
    2f64.powi(attempt.min(16) as i32).min(CAP_SECS)
}

/// Full jitter (the gateway's schedule): anywhere in 0..=ceiling, so a crowd that lost the server
/// at the same moment comes back spread over the whole window, not in a ±20% clump.
pub fn full_jitter_delay(attempt: u32, jitter: f64) -> Duration {
    Duration::from_secs_f64(ceiling(attempt) * jitter.clamp(0.0, 1.0))
}

/// After a server restart (closed with 1012 / 1001) every client is told at the same instant: the
/// first retry is spread over this window instead of all landing on the new process at once.
pub const RESTART_SPREAD_SECS: f64 = 3.0;

/// How the last session that reached Ready ended (it decides the first retry).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum After {
    /// None reached Ready yet, or it dropped within moments: back off from the start.
    Cold,
    /// Up for a while and then lost (a blip): retry at once.
    Healthy,
    /// The server said it's restarting: retry within [`RESTART_SPREAD_SECS`], at random.
    Restart,
}

/// Gateway reconnect schedule: `n` = attempts since the last session that reached Ready. After a
/// healthy session the first try is immediate (a blip, not an outage), after a restart it's spread
/// over 0–3 s; then full jitter over 1 s, 2 s, 4 s … 30 s.
pub fn reconnect_delay(n: u32, after: After, jitter: f64) -> Duration {
    match (after, n) {
        (After::Healthy, 0) => Duration::ZERO,
        (After::Restart, 0) => {
            Duration::from_secs_f64(RESTART_SPREAD_SECS * jitter.clamp(0.0, 1.0))
        }
        (After::Healthy | After::Restart, n) => full_jitter_delay(n - 1, jitter),
        (After::Cold, n) => full_jitter_delay(n, jitter),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_schedule() {
        let secs: Vec<f64> = (0..7)
            .map(|a| backoff_delay(a, 0.5).as_secs_f64())
            .collect();
        assert_eq!(secs, vec![1.0, 2.0, 4.0, 8.0, 16.0, 30.0, 30.0]);
        assert!((backoff_delay(0, 0.0).as_secs_f64() - 0.8).abs() < 1e-9);
        assert!((backoff_delay(0, 1.0).as_secs_f64() - 1.2).abs() < 1e-9);
        for a in 0..50 {
            assert!(backoff_delay(a, 1.0) <= Duration::from_secs(36));
        }
    }

    #[test]
    fn full_jitter_spans_zero_to_the_ceiling() {
        let top: Vec<f64> = (0..7)
            .map(|a| full_jitter_delay(a, 1.0).as_secs_f64())
            .collect();
        assert_eq!(top, vec![1.0, 2.0, 4.0, 8.0, 16.0, 30.0, 30.0]);
        for a in 0..50 {
            assert_eq!(full_jitter_delay(a, 0.0), Duration::ZERO);
            assert!(full_jitter_delay(a, 1.0) <= Duration::from_secs(30));
            // out-of-range jitter is clamped, never negative or past the cap
            assert_eq!(full_jitter_delay(a, -3.0), Duration::ZERO);
            assert!(full_jitter_delay(a, 7.0) <= Duration::from_secs(30));
        }
        assert!((full_jitter_delay(3, 0.25).as_secs_f64() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn after_a_healthy_session_the_first_retry_is_immediate_then_the_usual_schedule() {
        let healthy: Vec<f64> = (0..4)
            .map(|n| reconnect_delay(n, After::Healthy, 1.0).as_secs_f64())
            .collect();
        assert_eq!(healthy, vec![0.0, 1.0, 2.0, 4.0]);
        let cold: Vec<f64> = (0..3)
            .map(|n| reconnect_delay(n, After::Cold, 1.0).as_secs_f64())
            .collect();
        assert_eq!(cold, vec![1.0, 2.0, 4.0]);
    }

    #[test]
    fn after_a_restart_the_first_retry_is_spread_over_three_seconds() {
        assert_eq!(reconnect_delay(0, After::Restart, 0.0), Duration::ZERO);
        assert!((reconnect_delay(0, After::Restart, 0.5).as_secs_f64() - 1.5).abs() < 1e-9);
        assert!((reconnect_delay(0, After::Restart, 1.0).as_secs_f64() - 3.0).abs() < 1e-9);
        // then the same full-jitter schedule as after a healthy session
        let then: Vec<f64> = (1..4)
            .map(|n| reconnect_delay(n, After::Restart, 1.0).as_secs_f64())
            .collect();
        assert_eq!(then, vec![1.0, 2.0, 4.0]);
    }

    /// The point of full jitter: 1000 clients told to reconnect at once don't land together.
    #[test]
    fn a_crowd_restarting_together_is_spread_out() {
        let mut buckets = [0u32; 6]; // half-second buckets over 0–3 s
        for i in 0..1000 {
            let j = (i as f64 + 0.5) / 1000.0; // evenly spaced stand-in for rand
            let d = reconnect_delay(0, After::Restart, j).as_secs_f64();
            buckets[((d / 0.5) as usize).min(5)] += 1;
        }
        for b in buckets {
            assert!((150..=180).contains(&b), "{buckets:?}");
        }
    }
}
