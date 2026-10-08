//! Backoff and classification for contention recovery.
//!
//! Two separate judgements live here and they are deliberately not the same
//! predicate. `is_retryable_cozo_error` answers "should the guard sleep and try
//! this again?". `is_store_contention` also recognizes legacy acquisition
//! wait-expiry strings. Retries have no attempt limit; they end only after a
//! full no-progress window (`crate::contention`), as typed StoreBusy.

use std::time::Duration;

use crate::CozoGuardConfig;
use crate::locking::WRITE_LOCK_WAIT_EXPIRED;

pub(crate) fn backoff_steps(config: &CozoGuardConfig) -> usize {
    config.max_attempts.max(1)
}

#[cfg(test)]
pub(crate) fn initial_backoff_ramp(config: &CozoGuardConfig) -> Duration {
    (0..backoff_steps(config).saturating_sub(1))
        .map(|attempt| backoff_duration(config, attempt))
        .sum()
}

pub fn is_retryable_cozo_error(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    // StoreBusy is an authoritative response category. Its lock path/context
    // may contain text like "code500", which is not a SQLite error code.
    if message.contains("retryable store busy after ")
        && message.contains("operation not completed:")
    {
        return true;
    }
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
/// again*. A no-progress window that expires is explicit retryable
/// `StoreBusy`; the guard returns it without restarting the window. This
/// predicate additionally recognizes legacy expiry messages as contention.
/// Callers with nothing to degrade to must propagate, never substitute success.
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
        // A filename/identifier containing code500 is not an error-code field.
        if message[..index]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
        {
            return None;
        }
        let suffix = &message[index + "code".len()..];
        if !suffix.starts_with(|c: char| c.is_ascii_whitespace() || matches!(c, ':' | '(' | '=')) {
            return None;
        }
        let suffix = suffix.trim_start_matches(|character: char| {
            character.is_ascii_whitespace() || matches!(character, ':' | '(' | '=')
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
