//! Reconnect backoff: 1 s, 2 s, 4 s … capped at 30 s, ±20% jitter so clients don't stampede.

use std::time::Duration;

const CAP_SECS: f64 = 30.0;

/// Delay before reconnect attempt `attempt` (0-based). `jitter` in 0..=1 maps to ×0.8..×1.2.
pub fn backoff_delay(attempt: u32, jitter: f64) -> Duration {
    let base = 2f64.powi(attempt.min(16) as i32).min(CAP_SECS);
    Duration::from_secs_f64(base * (0.8 + 0.4 * jitter.clamp(0.0, 1.0)))
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
}
