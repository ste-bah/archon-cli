//! When a stream round resends its request, and when it stops (Issue 364).
//!
//! Every limit here is a no-progress limit, and every stop is the
//! [`archon_llm::transport_idle::TRANSPORT_STALL_MARKER`] text, which the
//! workflow host turns into a resumable pause, never a failure. Two kinds of
//! failed attempt end a stream before its round completes:
//!
//! - The provider ANSWERED, but the stream ended early or empty. The round
//!   resends with a capped linear backoff, and stops when it has had no
//!   complete answer for a whole no-progress window since the round began
//!   AND has resent more than [`STREAM_RETRIES`] times.
//! - The provider gave NO ANSWER: the stream went silent for its idle window,
//!   a transport error ended it, or a resend could not open (a retryable
//!   provider error). After a wake the network can need tens of seconds to
//!   come back, and a fast resend fails at once. So these attempts back off
//!   (doubling, capped) and the round stops only when it has heard nothing
//!   from the provider for a whole no-progress window of awake time AND has
//!   tried more than [`STREAM_RETRIES`] times. Model output starts a new
//!   window; a stream that opens, or framing alone, does not, so a stream
//!   that opens and resets again and again still pauses.
//!
//! A wait that ended because the machine slept starts a new window too: the
//! awake silence before the sleep does not count, so the round has a whole
//! window after the wake for the network to come back.
//!
//! The round keeps the last provider error it saw since the last answer
//! (bounded, redacted) and logs it on each retry, so a stop names the real
//! cause: a persistent 500 is "no successful answer ... last provider error:
//! server error (500): ...", not "no answer". A rate limit with a retry time
//! is not silence: the round waits it out while the wait ends inside the
//! window, and otherwise stops at once as the resumable pause, naming the
//! retry time and whether the provider named it or it is a default wait. A
//! pause and not a long wait in place: a quota reset hours away would outlast
//! the call's own stage limit, which fails the call. A request the provider
//! rejected ends the round at once, its text redacted the same way.

use std::time::Duration;

use super::stream_idle_window::IdleExpired;
use archon_llm::provider::LlmError;

pub(super) const STREAM_RETRIES: usize = 3;

/// First step and largest backoff of a resend after an incomplete stream
/// (linear), and after no answer (doubling).
#[cfg(not(test))]
const INCOMPLETE_BACKOFF: (Duration, Duration) = (Duration::from_secs(1), Duration::from_secs(30));
#[cfg(test)]
const INCOMPLETE_BACKOFF: (Duration, Duration) =
    (Duration::from_millis(10), Duration::from_millis(80));
#[cfg(not(test))]
const UNANSWERED_BACKOFF: (Duration, Duration) = (Duration::from_secs(1), Duration::from_secs(30));
#[cfg(test)]
const UNANSWERED_BACKOFF: (Duration, Duration) =
    (Duration::from_millis(10), Duration::from_millis(80));

/// The most characters of a provider error kept for logs and texts.
const LAST_ERROR_CHARS: usize = 360;

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
    /// When the round began: no complete answer since then.
    started: tokio::time::Instant,
    last_answer: tokio::time::Instant,
    incomplete: usize,
    unanswered: usize,
    /// The last provider error since the last answer, bounded and redacted.
    last_error: Option<String>,
}

impl ResendBudget {
    pub(super) fn new(window: Duration) -> Self {
        let now = tokio::time::Instant::now();
        Self {
            window,
            started: now,
            last_answer: now,
            incomplete: 0,
            unanswered: 0,
            last_error: None,
        }
    }

    /// Failed attempts of this round so far.
    pub(super) fn failures(&self) -> usize {
        self.incomplete + self.unanswered
    }

    /// The provider sent model output: a new no-progress window starts, and
    /// an older error is no longer the cause of what comes next.
    pub(super) fn answered(&mut self) {
        self.last_answer = tokio::time::Instant::now();
        self.last_error = None;
    }

    /// A stream opened. Not yet progress (a stream that opens and resets
    /// again and again must reach the window), but an open error before it
    /// is no longer the cause of what comes next.
    pub(super) fn opened(&mut self) {
        self.last_error = None;
    }

    /// An idle window ended. When the machine slept through it, the
    /// no-progress window starts again at the wake.
    pub(super) fn note_expiry(&mut self, expired: IdleExpired) {
        if expired.slept {
            self.last_answer = tokio::time::Instant::now();
        }
    }

    /// An error event ended the stream (`connection reset`, ...): it is the
    /// cause a later stop names.
    pub(super) fn note_stream_error(&mut self, error_type: &str, message: &str) {
        let text = bounded_text(&format!("{error_type}: {message}"));
        tracing::warn!(
            attempt = self.failures() + 1,
            error = %text,
            scope = "subagent",
            "subagent stream ended with an error; resending inside the no-progress window"
        );
        self.last_error = Some(text);
    }

    /// Records one failed attempt. `Ok` is the backoff before the resend;
    /// `Err` is the text the round stops with.
    pub(super) fn failed(&mut self, attempt: FailedAttempt) -> Result<Duration, String> {
        match attempt {
            FailedAttempt::Incomplete => {
                self.incomplete += 1;
                let quiet = self.started.elapsed();
                if self.incomplete > STREAM_RETRIES && quiet >= self.window {
                    return Err(self.stop_text(
                        "no complete answer",
                        quiet,
                        self.incomplete,
                        "incomplete response",
                    ));
                }
                let (step, cap) = INCOMPLETE_BACKOFF;
                Ok(step.saturating_mul(self.incomplete as u32).min(cap))
            }
            FailedAttempt::Unanswered(reason) => {
                self.unanswered += 1;
                let quiet = self.last_answer.elapsed();
                if self.unanswered > STREAM_RETRIES && quiet >= self.window {
                    let answer = if self.last_error.is_some() {
                        "no successful answer"
                    } else {
                        "no answer"
                    };
                    return Err(self.stop_text(answer, quiet, self.unanswered, reason));
                }
                Ok(unanswered_backoff(self.unanswered))
            }
        }
    }

    fn stop_text(&self, answer: &str, quiet: Duration, attempts: usize, reason: &str) -> String {
        let last_error = self
            .last_error
            .as_ref()
            .map(|error| format!("; last provider error: {error}"))
            .unwrap_or_default();
        format!(
            "{} subagent stream retry exhausted: {answer} from the provider for {quiet:?} of awake time across {attempts} attempts (last: {reason}{last_error}); prior conversation retained",
            archon_llm::transport_idle::TRANSPORT_STALL_MARKER,
        )
    }

    /// One open that returned `error`. `Ok` is the backoff before the next
    /// open; `Err` ends the round: a request the provider rejected, at once
    /// and redacted, or the stop text.
    pub(super) fn open_failed(
        &mut self,
        error: anyhow::Error,
        reason: &'static str,
    ) -> anyhow::Result<Duration> {
        let retry_after = rate_limit_retry_after(&error);
        if retry_after.is_none() && !retryable_open_error(&error) {
            return Err(redacted(error));
        }
        let text = bounded_text(&format!("{error:#}"));
        tracing::warn!(
            reason,
            attempt = self.failures() + 1,
            retry_after_secs = retry_after.map(|(wait, _)| wait.as_secs()),
            error = %text,
            scope = "subagent",
            "subagent stream could not open; trying again inside the no-progress window"
        );
        self.last_error = Some(text);
        match retry_after {
            Some((wait, from_provider)) if wait > UNANSWERED_BACKOFF.1 => {
                self.rate_limited(wait, from_provider, reason)
            }
            // A short wait is served by the usual backoff, never shorter.
            short => self
                .failed(FailedAttempt::Unanswered(reason))
                .map(|backoff| backoff.max(short.map(|(wait, _)| wait).unwrap_or_default())),
        }
        .map_err(anyhow::Error::msg)
    }

    /// A rate limit with a wait of `retry_after`. A known wait is not
    /// silence: it is served when it ends inside the no-progress window, and
    /// otherwise the round stops now with the retry time.
    fn rate_limited(
        &mut self,
        retry_after: Duration,
        from_provider: bool,
        reason: &str,
    ) -> Result<Duration, String> {
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
        let wait = if from_provider {
            "the provider asked to retry after"
        } else {
            "the provider named no wait; the default wait is"
        };
        Err(format!(
            "{} subagent stream rate limited: {wait} {}s (at {at}), past the {:?} no-progress window (last: {reason}; last provider error: {}); resume after that time; prior conversation retained",
            archon_llm::transport_idle::TRANSPORT_STALL_MARKER,
            retry_after.as_secs(),
            self.window,
            self.last_error.as_deref().unwrap_or("rate limited"),
        ))
    }
}

/// The wait of a rate limit, and whether the provider named it.
fn rate_limit_retry_after(error: &anyhow::Error) -> Option<(Duration, bool)> {
    match error.downcast_ref::<LlmError>()? {
        LlmError::RateLimited {
            retry_after_secs,
            from_provider,
        } => Some((Duration::from_secs(*retry_after_secs), *from_provider)),
        _ => None,
    }
}

/// `error` with its provider text redacted and bounded as the stop text is;
/// the variant is kept, so callers still classify it.
fn redacted(error: anyhow::Error) -> anyhow::Error {
    let error = match error.downcast::<LlmError>() {
        Ok(error) => error,
        Err(other) => return other,
    };
    let clean = |text: String| bounded_text(&text);
    anyhow::Error::new(match error {
        LlmError::Http(text) => LlmError::Http(clean(text)),
        LlmError::Auth(text) => LlmError::Auth(clean(text)),
        LlmError::QuotaExceeded(text) => LlmError::QuotaExceeded(clean(text)),
        LlmError::Unsupported(text) => LlmError::Unsupported(clean(text)),
        LlmError::Serialize(text) => LlmError::Serialize(clean(text)),
        LlmError::Server { status, message } => LlmError::Server {
            status,
            message: clean(message),
        },
        LlmError::Rejected {
            error_type,
            message,
        } => LlmError::Rejected {
            error_type,
            message: clean(message),
        },
        LlmError::ContextWindowExceeded {
            provider_message,
            provider,
            model,
        } => LlmError::ContextWindowExceeded {
            provider_message: clean(provider_message),
            provider,
            model,
        },
        other => other,
    })
}

/// `text` for a log line or a stop text: redacted as the log layer redacts,
/// one line, at most [`LAST_ERROR_CHARS`] characters.
fn bounded_text(text: &str) -> String {
    let text = archon_observability::redaction::redact_text(text);
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
        .downcast_ref::<LlmError>()
        .is_some_and(|error| archon_llm::classify_retry(error) == archon_llm::RetryDecision::Retry)
}

#[cfg(test)]
#[path = "stream_resend_budget_tests.rs"]
mod tests;
