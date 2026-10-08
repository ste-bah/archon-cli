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
}

impl ResendBudget {
    pub(super) fn new(window: Duration) -> Self {
        Self {
            window,
            last_answer: tokio::time::Instant::now(),
            incomplete: 0,
            unanswered: 0,
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
                    return Err(format!(
                        "{} subagent stream retry exhausted: no answer from the provider for {:?} of awake time across {} attempts (last: {reason}); prior conversation retained",
                        archon_llm::transport_idle::TRANSPORT_STALL_MARKER,
                        self.last_answer.elapsed(),
                        self.unanswered,
                    ));
                }
                Ok(unanswered_backoff(self.unanswered))
            }
        }
    }
}

fn unanswered_backoff(failures: usize) -> Duration {
    let (first, cap) = UNANSWERED_BACKOFF;
    let doublings = failures.saturating_sub(1).min(16) as u32;
    first.saturating_mul(1 << doublings).min(cap)
}

/// Is this failure to open a stream one a resend may cure?
pub(super) fn retryable_open_error(error: &anyhow::Error) -> bool {
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
