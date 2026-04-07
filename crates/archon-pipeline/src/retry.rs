//! API retry with exponential backoff.
//!
//! Provides retry logic for transient API errors (429, 500, 502, 503)
//! with exponential backoff and optional jitter.

use std::fmt;
use std::future::Future;
use std::time::Duration;

use anyhow::Result;

use crate::runner::{AgentInfo, QualityScore};

// ---------------------------------------------------------------------------
// RetryableError
// ---------------------------------------------------------------------------

/// Classifies an error as retryable or non-retryable.
#[derive(Debug, Clone)]
pub enum RetryableError {
    /// Transient error that should be retried (429, 500, 502, 503, connection issues).
    Retryable {
        status: u16,
        message: String,
        retry_after: Option<Duration>,
    },
    /// Permanent error that should NOT be retried (400, 401, 403, 404, 422).
    NonRetryable { status: u16, message: String },
}

impl fmt::Display for RetryableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Retryable {
                status, message, ..
            } => write!(f, "Retryable error (HTTP {}): {}", status, message),
            Self::NonRetryable { status, message } => {
                write!(f, "Non-retryable error (HTTP {}): {}", status, message)
            }
        }
    }
}

impl std::error::Error for RetryableError {}

// ---------------------------------------------------------------------------
// RetryConfig
// ---------------------------------------------------------------------------

/// Configuration for retry behaviour.
pub struct RetryConfig {
    /// Maximum number of retries (not counting the initial attempt).
    pub max_retries: u32,
    /// Base delay before the first retry.
    pub base_delay: Duration,
    /// Upper bound on any single delay.
    pub max_delay: Duration,
    /// Whether to apply random jitter to delays.
    pub jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 5,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            jitter: true,
        }
    }
}

// ---------------------------------------------------------------------------
// calculate_delay (deterministic — no jitter)
// ---------------------------------------------------------------------------

/// Calculate delay for attempt `attempt` (0-indexed).
///
/// Formula: `min(base_delay * 2^attempt, max_delay)`.
///
/// Jitter is intentionally **not** applied here so callers (and tests) can
/// verify deterministic values.  Jitter is applied inside [`with_retry`].
pub fn calculate_delay(config: &RetryConfig, attempt: u32) -> Duration {
    let base_ms = config.base_delay.as_millis() as u64;
    let multiplier = 2u64.saturating_pow(attempt);
    let delay_ms = base_ms.saturating_mul(multiplier);
    let capped_ms = delay_ms.min(config.max_delay.as_millis() as u64);
    Duration::from_millis(capped_ms)
}

// ---------------------------------------------------------------------------
// classify_error
// ---------------------------------------------------------------------------

/// Classify an [`anyhow::Error`] as retryable or non-retryable.
///
/// 1. If the error chain contains a [`RetryableError`], return it directly.
/// 2. Otherwise fall back to pattern-matching the error message for known
///    HTTP status code patterns.
/// 3. Default: treat as non-retryable.
pub fn classify_error(err: &anyhow::Error) -> RetryableError {
    // Check the chain for a concrete RetryableError.
    if let Some(re) = err.downcast_ref::<RetryableError>() {
        return re.clone();
    }

    // Fallback: scan the display string for HTTP status patterns.
    let msg = err.to_string();

    // Retryable status codes.
    for code in &[429u16, 500, 502, 503] {
        let pattern = format!("{}", code);
        if msg.contains(&pattern) {
            return RetryableError::Retryable {
                status: *code,
                message: msg.clone(),
                retry_after: None,
            };
        }
    }

    // Non-retryable status codes.
    for code in &[400u16, 401, 403, 404, 422] {
        let pattern = format!("{}", code);
        if msg.contains(&pattern) {
            return RetryableError::NonRetryable {
                status: *code,
                message: msg.clone(),
            };
        }
    }

    // Default: non-retryable.
    RetryableError::NonRetryable {
        status: 0,
        message: msg,
    }
}

// ---------------------------------------------------------------------------
// with_retry
// ---------------------------------------------------------------------------

/// Retry an async operation with exponential backoff.
///
/// The operation is called up to `config.max_retries + 1` times (one initial
/// attempt plus up to `max_retries` retries).  Only errors classified as
/// [`RetryableError::Retryable`] trigger a retry; non-retryable errors are
/// returned immediately.
pub async fn with_retry<F, Fut, T>(config: &RetryConfig, operation: F) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut attempt = 0u32;
    loop {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(err) => {
                let classification = classify_error(&err);
                match classification {
                    RetryableError::NonRetryable { .. } => {
                        return Err(err);
                    }
                    RetryableError::Retryable { retry_after, .. } => {
                        if attempt >= config.max_retries {
                            return Err(err);
                        }

                        // Deterministic base delay.
                        let mut delay = calculate_delay(config, attempt);

                        // Respect retry-after hint.
                        if let Some(ra) = retry_after {
                            delay = delay.max(ra);
                        }

                        // Apply jitter.
                        if config.jitter {
                            use rand::Rng;
                            let mut rng = rand::rng();
                            let factor: f64 = rng.random_range(0.5..1.5);
                            delay = Duration::from_millis(
                                (delay.as_millis() as f64 * factor) as u64,
                            );
                        }

                        tracing::warn!(
                            attempt = attempt + 1,
                            max_retries = config.max_retries,
                            delay_ms = delay.as_millis() as u64,
                            error = %err,
                            "Retrying after transient error"
                        );

                        tokio::time::sleep(delay).await;
                        attempt += 1;
                    }
                }
            }
        }
    }
}

// ===========================================================================
// Quality Gate Retry Logic (TASK-PIPE-A06)
// ===========================================================================

// ---------------------------------------------------------------------------
// QualityRetryConfig
// ---------------------------------------------------------------------------

/// Configuration for quality gate retries.
pub struct QualityRetryConfig {
    /// Maximum number of retries (not counting the initial attempt).
    pub max_retries: u32,
    /// Minimum quality score to accept an agent's output.
    pub quality_threshold: f64,
}

impl Default for QualityRetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            quality_threshold: 0.6,
        }
    }
}

// ---------------------------------------------------------------------------
// QualityRetryResult
// ---------------------------------------------------------------------------

/// Result of a quality retry sequence.
#[derive(Debug)]
pub enum QualityRetryResult {
    /// The agent produced output meeting the quality threshold.
    Accepted {
        output: String,
        score: QualityScore,
        attempt: u32,
    },
    /// A critical agent exhausted all retries without meeting the threshold.
    Failed {
        agent_key: String,
        final_score: QualityScore,
        attempts: u32,
        reason: String,
    },
    /// A non-critical agent exhausted all retries; its output is skipped.
    Skipped {
        agent_key: String,
        final_score: QualityScore,
        attempts: u32,
        warning: String,
    },
}

// ---------------------------------------------------------------------------
// build_quality_feedback
// ---------------------------------------------------------------------------

/// Build feedback message from a [`QualityScore`] explaining what needs
/// improvement.
pub fn build_quality_feedback(score: &QualityScore, output: &str) -> String {
    if output.is_empty() {
        return "Previous output was empty. Please provide a complete response.".to_string();
    }

    let mut feedback = format!("Quality score {:.2} is below threshold. ", score.overall);

    if score.dimensions.is_empty() {
        feedback.push_str("Please improve the overall quality of your response.");
    } else {
        feedback.push_str("Areas needing improvement: ");
        let low_dims: Vec<_> = score
            .dimensions
            .iter()
            .filter(|(_, v)| **v < 0.6)
            .collect();
        if low_dims.is_empty() {
            // All dimensions are acceptable but overall is low.
            feedback.push_str("Overall coherence and completeness need improvement.");
        } else {
            for (name, value) in &low_dims {
                feedback.push_str(&format!("{} ({:.2}), ", name, value));
            }
        }
    }

    feedback
}

// ---------------------------------------------------------------------------
// retry_on_quality
// ---------------------------------------------------------------------------

/// Retry an agent when quality is below threshold.
///
/// The `run_agent` closure receives a feedback string describing what needs
/// improvement and returns the new `(output, QualityScore)` pair.
pub async fn retry_on_quality<F, Fut>(
    config: &QualityRetryConfig,
    agent: &AgentInfo,
    initial_output: &str,
    initial_score: &QualityScore,
    run_agent: F,
) -> Result<QualityRetryResult>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<(String, QualityScore)>>,
{
    // Use agent-specific threshold if set, otherwise config default.
    let threshold = if agent.quality_threshold > 0.0 {
        agent.quality_threshold
    } else {
        config.quality_threshold
    };

    // Check initial score.
    if initial_score.overall >= threshold {
        return Ok(QualityRetryResult::Accepted {
            output: initial_output.to_string(),
            score: initial_score.clone(),
            attempt: 1,
        });
    }

    let mut current_output = initial_output.to_string();
    let mut current_score = initial_score.clone();

    for retry in 0..config.max_retries {
        let feedback = build_quality_feedback(&current_score, &current_output);

        tracing::warn!(
            agent_key = %agent.key,
            attempt = retry + 2, // +2 because attempt 1 was the initial
            previous_score = current_score.overall,
            feedback_preview = %feedback.chars().take(100).collect::<String>(),
            "Quality retry"
        );

        let (new_output, new_score) = run_agent(feedback).await?;

        if new_score.overall >= threshold {
            return Ok(QualityRetryResult::Accepted {
                output: new_output,
                score: new_score,
                attempt: retry + 2,
            });
        }

        current_output = new_output;
        current_score = new_score;
    }

    // Exhausted retries.
    let total_attempts = config.max_retries + 1;

    if agent.critical {
        let reason = format!(
            "Critical agent '{}' failed quality threshold ({:.2}) after {} attempts",
            agent.key, threshold, total_attempts
        );
        Ok(QualityRetryResult::Failed {
            agent_key: agent.key.clone(),
            final_score: current_score,
            attempts: total_attempts,
            reason,
        })
    } else {
        let warning = format!(
            "Non-critical agent '{}' skipped after {} attempts (best score: {:.2})",
            agent.key, total_attempts, current_score.overall
        );
        Ok(QualityRetryResult::Skipped {
            agent_key: agent.key.clone(),
            final_score: current_score,
            attempts: total_attempts,
            warning,
        })
    }
}
