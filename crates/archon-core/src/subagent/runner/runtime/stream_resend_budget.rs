//! When a stream round resends its request, and when it stops (Issue 364).
//!
//! Two kinds of failed attempt end a stream before its round completes:
//!
//! - The provider ANSWERED, but the stream ended early or empty. Resending is
//!   cheap to bound by count: [`STREAM_RETRIES`] resends of the round, as
//!   before.
//! - The provider gave NO ANSWER: the stream went silent for its idle window,
//!   a transport error ended it, or a resend could not open (a retryable
//!   provider error). After a wake the network can need tens of seconds to
//!   come back, and a fast resend fails at once. So these attempts back off
//!   (doubling, capped) and the round stops only when it has heard nothing
//!   from the provider for a whole no-progress window of awake time AND has
//!   tried more than [`STREAM_RETRIES`] times. Any answer from the provider (a
//!   stream that opens, any event) starts a new window. The stop is the
//!   [`archon_llm::transport_idle::TRANSPORT_STALL_MARKER`] text, which the
//!   workflow host turns into a resumable pause, never a failure.
//!
//! A wait that ended because the machine slept starts a new window too: the
//! awake silence before the sleep does not count, so the round has a whole
//! window after the wake for the network to come back.
//!
//! The round keeps the last provider error it saw (bounded, redacted) and
//! logs it on each retry, so a stop names the real cause: a persistent 500 is
//! "no successful answer ... last provider error: server error (500): ...",
//! not "no answer". A rate limit with a known retry time is not silence: the
//! round waits it out while the wait ends inside the window, and otherwise
//! stops at once as the resumable pause, naming the retry time. A pause and
//! not a long wait in place: a quota reset hours away would outlast the call's
//! own stage limit, which fails the call.

use std::time::Duration;

use super::stream_idle_window::IdleExpired;

pub(super) const STREAM_RETRIES: usize = 3;

/// Linear backoff of a resend after an answered but incomplete stream.
const INCOMPLETE_BACKOFF: Duration = Duration::from_secs(1);

/// First and largest backoff of a resend after no answer.
#[cfg(not(test))]
const UNANSWERED_BACKOFF: (Duration, Duration) = (Duration::from_secs(1), Duration::from_secs(30));
#[cfg(test)]
const UNANSWERED_BACKOFF: (Duration, Duration) =
    (Duration::from_millis(10), Duration::from_millis(80));

/// Why one attempt of a round failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FailedAttempt {
    /// The provider answered; the stream ended early or empty.
    Incomplete,
    /// No answer: idle, a transport error, or a resend that could not open.
    Unanswered(&'static str),
}

/// The resend decision of one round.
pub(super) struct ResendBudget {
    window: Duration,
    last_answer: tokio::time::Instant,
    incomplete: usize,
    unanswered: usize,
    /// The last provider error, bounded and redacted, for the stop text.
    last_error: Option<String>,
}

/// The most characters of a provider error kept for logs and the stop text.
const LAST_ERROR_CHARS: usize = 360;

impl ResendBudget {
    pub(super) fn new(window: Duration) -> Self {
        Self {
            window,
            last_answer: tokio::time::Instant::now(),
            incomplete: 0,
            unanswered: 0,
            last_error: None,
        }
    }

    /// Failed attempts of this round so far.
    pub(super) fn failures(&self) -> usize {
        self.incomplete + self.unanswered
    }

    /// The provider answered: a new no-progress window starts.
    pub(super) fn answered(&mut self) {
        self.last_answer = tokio::time::Instant::now();
    }

    /// An idle window ended. When the machine slept through it, the
    /// no-progress window starts again at the wake.
    pub(super) fn note_expiry(&mut self, expired: IdleExpired) {
        if expired.slept {
            self.last_answer = tokio::time::Instant::now();
        }
    }

    /// Records one failed attempt. `Ok` is the backoff before the resend;
    /// `Err` is the text the round stops with.
    pub(super) fn failed(&mut self, attempt: FailedAttempt) -> Result<Duration, String> {
        match attempt {
            FailedAttempt::Incomplete => {
                // An incomplete stream is still an answer.
                self.answered();
                self.incomplete += 1;
                if self.incomplete > STREAM_RETRIES {
                    return Err(format!(
                        "subagent stream retry exhausted after {STREAM_RETRIES} retries: incomplete response; prior conversation retained"
                    ));
                }
                Ok(INCOMPLETE_BACKOFF * self.incomplete as u32)
            }
            FailedAttempt::Unanswered(reason) => {
                self.unanswered += 1;
                if self.unanswered > STREAM_RETRIES && self.last_answer.elapsed() >= self.window {
                    let (answer, last_error) = match &self.last_error {
                        Some(error) => (
                            "no successful answer",
                            format!("; last provider error: {error}"),
                        ),
                        None => ("no answer", String::new()),
                    };
                    return Err(format!(
                        "{} subagent stream retry exhausted: {answer} from the provider for {:?} of awake time across {} attempts (last: {reason}{last_error}); prior conversation retained",
                        archon_llm::transport_idle::TRANSPORT_STALL_MARKER,
                        self.last_answer.elapsed(),
                        self.unanswered,
                    ));
                }
                Ok(unanswered_backoff(self.unanswered))
            }
        }
    }

    /// One open that returned `error`. `Ok` is the backoff before the next
    /// open; `Err` ends the round: a request the provider rejected, unchanged
    /// and at once, or the stop text.
    pub(super) fn open_failed(
        &mut self,
        error: anyhow::Error,
        reason: &'static str,
    ) -> anyhow::Result<Duration> {
        let retry_after = rate_limit_retry_after(&error);
        if retry_after.is_none() && !retryable_open_error(&error) {
            return Err(error);
        }
        let text = bounded_error_text(&error);
        tracing::warn!(
            reason,
            attempt = self.failures() + 1,
            retry_after_secs = retry_after.map(|wait| wait.as_secs()),
            error = %text,
            scope = "subagent",
            "subagent stream could not open; trying again inside the no-progress window"
        );
        self.last_error = Some(text);
        match retry_after {
            Some(wait) if wait > UNANSWERED_BACKOFF.1 => self.rate_limited(wait, reason),
            // A short wait is served by the usual backoff, never shorter.
            short => self
                .failed(FailedAttempt::Unanswered(reason))
                .map(|backoff| backoff.max(short.unwrap_or_default())),
        }
        .map_err(anyhow::Error::msg)
    }

    /// The provider asked for a wait of `retry_after`. A known wait is not
    /// silence: it is served when it ends inside the no-progress window, and
    /// otherwise the round stops now with the retry time.
    fn rate_limited(&mut self, retry_after: Duration, reason: &str) -> Result<Duration, String> {
        self.unanswered += 1;
        if self.last_answer.elapsed().saturating_add(retry_after) <= self.window {
            return Ok(retry_after);
        }
        let at = chrono::Duration::from_std(retry_after)
            .ok()
            .and_then(|wait| chrono::Utc::now().checked_add_signed(wait))
            .map_or_else(
                || "unknown".to_string(),
                |at| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            );
        Err(format!(
            "{} subagent stream rate limited: the provider asked to retry after {}s (at {}), past the {:?} no-progress window (last: {reason}; last provider error: {}); resume after that time; prior conversation retained",
            archon_llm::transport_idle::TRANSPORT_STALL_MARKER,
            retry_after.as_secs(),
            at,
            self.window,
            self.last_error.as_deref().unwrap_or("rate limited"),
        ))
    }
}

/// The provider's retry time when `error` is a rate limit.
fn rate_limit_retry_after(error: &anyhow::Error) -> Option<Duration> {
    match error.downcast_ref::<archon_llm::provider::LlmError>()? {
        archon_llm::provider::LlmError::RateLimited { retry_after_secs } => {
            Some(Duration::from_secs(*retry_after_secs))
        }
        _ => None,
    }
}

/// `error` for a log line or the stop text: redacted as the log layer
/// redacts, one line, at most [`LAST_ERROR_CHARS`] characters.
fn bounded_error_text(error: &anyhow::Error) -> String {
    let text = archon_observability::redaction::redact_text(&format!("{error:#}"));
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let total = text.chars().count();
    if total <= LAST_ERROR_CHARS {
        return text;
    }
    let mut bounded: String = text.chars().take(LAST_ERROR_CHARS).collect();
    bounded.push_str(&format!(
        " [truncated: {} more chars]",
        total - LAST_ERROR_CHARS
    ));
    bounded
}

fn unanswered_backoff(failures: usize) -> Duration {
    let (first, cap) = UNANSWERED_BACKOFF;
    let doublings = failures.saturating_sub(1).min(16) as u32;
    first.saturating_mul(1 << doublings).min(cap)
}

/// Is this failure to open a stream one a resend may cure?
fn retryable_open_error(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<archon_llm::provider::LlmError>()
        .is_some_and(|error| archon_llm::classify_retry(error) == archon_llm::RetryDecision::Retry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn quick_unanswered_resends_inside_the_window_never_stop_the_round() {
        let mut budget = ResendBudget::new(Duration::from_secs(3600));
        for _ in 0..50 {
            assert!(budget.failed(FailedAttempt::Unanswered("open")).is_ok());
        }
    }

    #[tokio::test]
    async fn unanswered_resends_stop_after_a_whole_window_with_the_stall_marker() {
        let mut budget = ResendBudget::new(Duration::from_millis(30));
        tokio::time::sleep(Duration::from_millis(40)).await;
        for _ in 0..STREAM_RETRIES {
            assert!(budget.failed(FailedAttempt::Unanswered("idle")).is_ok());
        }
        let stop = budget
            .failed(FailedAttempt::Unanswered("idle"))
            .expect_err("a whole window without an answer stops the round");
        assert!(
            stop.starts_with(archon_llm::transport_idle::TRANSPORT_STALL_MARKER),
            "{stop}"
        );
        assert!(stop.contains("stream retry exhausted"), "{stop}");
    }

    #[tokio::test]
    async fn an_answer_starts_a_new_window() {
        let mut budget = ResendBudget::new(Duration::from_millis(30));
        tokio::time::sleep(Duration::from_millis(40)).await;
        budget.answered();
        for _ in 0..10 {
            assert!(budget.failed(FailedAttempt::Unanswered("open")).is_ok());
        }
    }

    /// Fails before the fix: awake silence from before a sleep counted, so
    /// a few quick failures after the wake stopped the round.
    #[tokio::test]
    async fn a_sleep_starts_a_new_window_at_the_wake() {
        let mut budget = ResendBudget::new(Duration::from_millis(30));
        tokio::time::sleep(Duration::from_millis(40)).await;
        budget.note_expiry(IdleExpired { slept: true });
        for _ in 0..10 {
            assert!(budget.failed(FailedAttempt::Unanswered("open")).is_ok());
        }
        let mut awake = ResendBudget::new(Duration::from_millis(30));
        tokio::time::sleep(Duration::from_millis(40)).await;
        awake.note_expiry(IdleExpired { slept: false });
        assert!((0..10).any(|_| awake.failed(FailedAttempt::Unanswered("open")).is_err()));
    }

    #[test]
    fn unanswered_backoff_doubles_up_to_its_cap() {
        let (first, cap) = UNANSWERED_BACKOFF;
        assert_eq!(unanswered_backoff(1), first);
        assert_eq!(unanswered_backoff(2), first * 2);
        assert_eq!(unanswered_backoff(60), cap);
    }

    /// Fails before the fix: the stop said "no answer" and dropped the
    /// provider's status and body.
    #[tokio::test]
    async fn the_stop_names_the_last_provider_error_bounded_and_redacted() {
        let mut budget = ResendBudget::new(Duration::from_millis(20));
        let body = format!(
            "model not loaded api_key=sk-ant-secret123 {}",
            "x".repeat(900)
        );
        let server = || {
            anyhow::Error::new(archon_llm::provider::LlmError::Server {
                status: 500,
                message: body.clone(),
            })
        };
        tokio::time::sleep(Duration::from_millis(30)).await;
        let stop = (0..10)
            .find_map(|_| {
                budget
                    .open_failed(server(), "first open could not open")
                    .err()
            })
            .expect("a whole window of 500s stops the round")
            .to_string();
        assert!(
            stop.starts_with(archon_llm::transport_idle::TRANSPORT_STALL_MARKER),
            "{stop}"
        );
        assert!(stop.contains("no successful answer"), "{stop}");
        assert!(
            stop.contains("server error (500): model not loaded"),
            "{stop}"
        );
        assert!(!stop.contains("sk-ant-secret123"), "{stop}");
        assert!(stop.contains("[truncated:") && stop.len() < 900, "{stop}");
    }

    /// Fails before the fix: a long Retry-After failed the call.
    #[tokio::test]
    async fn a_rate_limit_is_waited_inside_the_window_and_paused_past_it() {
        let limited = |secs| {
            anyhow::Error::new(archon_llm::provider::LlmError::RateLimited {
                retry_after_secs: secs,
            })
        };
        let mut budget = ResendBudget::new(Duration::from_secs(600));
        let wait = budget.open_failed(limited(90), "first open could not open");
        assert_eq!(
            wait.expect("served inside the window"),
            Duration::from_secs(90)
        );
        let stop = budget
            .open_failed(limited(8_004), "first open could not open")
            .expect_err("past the window it stops now")
            .to_string();
        assert!(
            stop.starts_with(archon_llm::transport_idle::TRANSPORT_STALL_MARKER),
            "{stop}"
        );
        assert!(stop.contains("retry after 8004s (at 20"), "{stop}");
        assert!(stop.contains("resume after that time"), "{stop}");
    }

    #[tokio::test]
    async fn a_rejected_open_ends_the_round_unchanged() {
        let mut budget = ResendBudget::new(Duration::from_secs(600));
        let error = budget
            .open_failed(
                anyhow::Error::new(archon_llm::provider::LlmError::Server {
                    status: 400,
                    message: "invalid".into(),
                }),
                "first open could not open",
            )
            .expect_err("a 4xx is not retried");
        assert_eq!(error.to_string(), "server error (400): invalid");
        assert_eq!(budget.failures(), 0);
    }

    #[tokio::test]
    async fn incomplete_streams_keep_their_count_bound() {
        let mut budget = ResendBudget::new(Duration::from_secs(3600));
        for _ in 0..STREAM_RETRIES {
            assert!(budget.failed(FailedAttempt::Incomplete).is_ok());
        }
        let stop = budget.failed(FailedAttempt::Incomplete).unwrap_err();
        assert!(stop.contains("incomplete response"), "{stop}");
    }
}
