use std::time::Duration;

use crate::errors::DocsError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RateLimitRetry {
    pub max_attempts: usize,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl RateLimitRetry {
    pub fn vlm_default(base_delay: Duration) -> Self {
        Self {
            max_attempts: 5,
            base_delay,
            max_delay: Duration::from_secs(30),
        }
    }

    pub fn delay_for_attempt(&self, retry_index: usize) -> Duration {
        let multiplier = 1_u32.checked_shl(retry_index as u32).unwrap_or(u32::MAX);
        self.base_delay
            .saturating_mul(multiplier)
            .min(self.max_delay)
    }
}

pub fn retry_rate_limited<T>(
    config: RateLimitRetry,
    mut operation: impl FnMut() -> Result<T, DocsError>,
) -> Result<T, DocsError> {
    for attempt in 0..config.max_attempts {
        match operation() {
            Ok(value) => return Ok(value),
            Err(DocsError::VlmRateLimit { .. }) if attempt + 1 < config.max_attempts => {
                std::thread::sleep(config.delay_for_attempt(attempt));
            }
            Err(error) => return Err(error),
        }
    }

    unreachable!("retry loop returns on every operation result")
}

pub fn retry_vlm_transient<T>(
    config: RateLimitRetry,
    mut operation: impl FnMut() -> Result<T, DocsError>,
) -> Result<T, DocsError> {
    for attempt in 0..config.max_attempts {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) if is_retryable_vlm_error(&error) && attempt + 1 < config.max_attempts => {
                std::thread::sleep(config.delay_for_attempt(attempt));
            }
            Err(error) => return Err(error),
        }
    }

    unreachable!("retry loop returns on every operation result")
}

fn is_retryable_vlm_error(error: &DocsError) -> bool {
    match error {
        DocsError::VlmTimeout { .. } | DocsError::VlmRateLimit { .. } => true,
        DocsError::VlmProvider {
            status_code: Some(status),
            ..
        } => (500..=599).contains(status),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_schedule_doubles_and_caps() {
        let retry = RateLimitRetry {
            max_attempts: 5,
            base_delay: Duration::from_secs(10),
            max_delay: Duration::from_secs(30),
        };
        assert_eq!(retry.delay_for_attempt(0), Duration::from_secs(10));
        assert_eq!(retry.delay_for_attempt(1), Duration::from_secs(20));
        assert_eq!(retry.delay_for_attempt(2), Duration::from_secs(30));
        assert_eq!(retry.delay_for_attempt(3), Duration::from_secs(30));
    }

    #[test]
    fn transient_vlm_errors_are_retried() {
        let mut attempts = 0;
        let retry = RateLimitRetry {
            max_attempts: 3,
            base_delay: Duration::from_millis(0),
            max_delay: Duration::from_millis(0),
        };

        let result = retry_vlm_transient(retry, || {
            attempts += 1;
            if attempts < 3 {
                return Err(DocsError::VlmTimeout {
                    provider: "ollama".into(),
                    message: "slow local model".into(),
                });
            }
            Ok("described")
        });

        assert_eq!(result.unwrap(), "described");
        assert_eq!(attempts, 3);
    }
}
