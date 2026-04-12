//! Pipeline step backoff schedule.

use std::time::Duration;

use crate::spec::BackoffKind;

/// Maximum backoff cap: 5 minutes.
const MAX_BACKOFF: Duration = Duration::from_secs(300);

/// Compute the delay for a given attempt and backoff strategy.
///
/// `attempt` is 1-based (first retry = attempt 1).
pub fn delay(kind: BackoffKind, attempt: u32, base_ms: u64) -> Duration {
    let ms = match kind {
        BackoffKind::Fixed => base_ms,
        BackoffKind::Linear => base_ms.saturating_mul(attempt as u64),
        BackoffKind::Exponential => {
            base_ms.saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1)))
        }
    };
    let dur = Duration::from_millis(ms);
    dur.min(MAX_BACKOFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exponential_doubles() {
        assert_eq!(delay(BackoffKind::Exponential, 1, 100), Duration::from_millis(100));
        assert_eq!(delay(BackoffKind::Exponential, 2, 100), Duration::from_millis(200));
        assert_eq!(delay(BackoffKind::Exponential, 3, 100), Duration::from_millis(400));
        assert_eq!(delay(BackoffKind::Exponential, 4, 100), Duration::from_millis(800));
    }

    #[test]
    fn linear_scales() {
        assert_eq!(delay(BackoffKind::Linear, 1, 100), Duration::from_millis(100));
        assert_eq!(delay(BackoffKind::Linear, 2, 100), Duration::from_millis(200));
        assert_eq!(delay(BackoffKind::Linear, 3, 100), Duration::from_millis(300));
    }

    #[test]
    fn fixed_constant() {
        assert_eq!(delay(BackoffKind::Fixed, 1, 100), Duration::from_millis(100));
        assert_eq!(delay(BackoffKind::Fixed, 2, 100), Duration::from_millis(100));
        assert_eq!(delay(BackoffKind::Fixed, 5, 100), Duration::from_millis(100));
    }

    #[test]
    fn capped_at_five_minutes() {
        let d = delay(BackoffKind::Exponential, 30, 1000);
        assert_eq!(d, Duration::from_secs(300));
    }
}
