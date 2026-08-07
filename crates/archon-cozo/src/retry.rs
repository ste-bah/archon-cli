//! How long a guarded operation waits, and what it is willing to wait for.
//!
//! Two separate judgements live here and they are deliberately not the same
//! predicate. `is_retryable_cozo_error` answers "should the guard sleep and try
//! this again?". `is_store_contention` answers "may the caller give up on this
//! one item and keep going?" -- a broader question, because a bounded wait that
//! expired is not worth retrying but is still only contention.

use std::time::Duration;

use crate::CozoGuardConfig;
use crate::locking::WRITE_LOCK_WAIT_EXPIRED;

pub(crate) fn normalized_attempts(config: &CozoGuardConfig) -> usize {
    config.max_attempts.max(1)
}

#[cfg(test)]
pub(crate) fn cumulative_backoff_budget(config: &CozoGuardConfig) -> Duration {
    (0..normalized_attempts(config).saturating_sub(1))
        .map(|attempt| backoff_duration(config, attempt))
        .sum()
}

pub(crate) fn retry_backoff(
    context: &str,
    config: &CozoGuardConfig,
    attempt: usize,
    attempts: usize,
    error: &str,
) -> Option<Duration> {
    if !is_retryable_cozo_error(error) || attempt + 1 >= attempts {
        return None;
    }

    tracing::warn!(
        context,
        attempt = attempt + 1,
        max_attempts = attempts,
        error,
        "Cozo store busy; retrying guarded operation"
    );
    Some(backoff_duration(config, attempt))
}

pub fn is_retryable_cozo_error(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    if explicit_error_codes(&message).any(|code| code != 5) {
        return false;
    }
    [
        "database is locked",
        "database table is locked",
        "locked (code 5)",
        "code: some(5)",
        "sqlite_busy",
        "write-lock unavailable",
        "write lock unavailable",
    ]
    .iter()
    .any(|signal| message.contains(signal))
}

/// Did `message` describe losing a race for the store, rather than a fault?
///
/// Broader than [`is_retryable_cozo_error`] on purpose, and asking a different
/// question. That predicate decides whether the guard should *sleep and try
/// again*; a wedged holder fails it deliberately, because retrying a lock that
/// has already outlasted a bounded wait only delays the diagnosis. This one
/// decides whether the caller may *degrade* -- skip one file, keep the pass --
/// and for that purpose an expired wait is contention just as much as a
/// SQLITE_BUSY is. Callers with nothing to degrade to should keep using the
/// retry predicate and propagate.
pub fn is_store_contention(message: &str) -> bool {
    is_retryable_cozo_error(message) || message.contains(WRITE_LOCK_WAIT_EXPIRED)
}

/// Render a Cozo error together with every error in its source chain.
///
/// `cozo::Error` is a `miette::Report`, and `Display` on a report shows only
/// the outermost context. Cozo attaches the interesting part underneath: a
/// `:create` that loses a race is reported as `when executing against relation
/// 'code_chunks'`, with `Cannot create relation code_chunks as one with the
/// same name already exists` sitting one link down. Formatting the report --
/// including with `{:#}` after it has been folded into an `anyhow::Error` --
/// never reaches that link, so a caller classifying the failure by its message
/// sees only the context and cannot tell a lost race from a malformed schema.
/// That is issue #144. Walking the chain is what lets the benign match stay
/// narrow instead of being widened to cover the context string, which is the
/// same text a genuinely broken schema change produces.
pub fn render_cozo_error(error: &cozo::Error) -> String {
    error
        .chain()
        .map(|link| link.to_string())
        .collect::<Vec<_>>()
        .join(": ")
}

fn explicit_error_codes(message: &str) -> impl Iterator<Item = u64> + '_ {
    message.match_indices("code").filter_map(|(index, _)| {
        let suffix = &message[index + "code".len()..];
        let suffix = suffix.trim_start_matches(|character: char| {
            character.is_ascii_whitespace() || matches!(character, ':' | '(')
        });
        let suffix = suffix.strip_prefix("some(").unwrap_or(suffix);
        let digits = suffix
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>();
        digits.parse().ok()
    })
}

pub(crate) fn backoff_duration(config: &CozoGuardConfig, attempt: usize) -> Duration {
    let initial = config.initial_backoff.as_millis() as u64;
    let max = config.max_backoff.as_millis() as u64;
    Duration::from_millis(initial.saturating_mul(attempt as u64 + 1).min(max))
}
