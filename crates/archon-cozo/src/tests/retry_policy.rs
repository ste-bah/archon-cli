//! What the retry policies promise, and which errors they apply to.
//!
//! Two questions, both answered without running a retry loop: how long a policy
//! is willing to sleep in total, and whether a given error message is a busy
//! signal worth retrying or a terminal fault worth surfacing.

use std::time::Duration;

use super::*;

#[test]
fn default_retry_policy_has_twenty_attempts() {
    assert_eq!(CozoGuardConfig::default().max_attempts, 20);
}

#[test]
fn default_retry_policy_caps_cumulative_sleep_at_nineteen_seconds() {
    assert_eq!(
        cumulative_backoff_budget(&CozoGuardConfig::default()),
        Duration::from_secs(19)
    );
}

#[test]
fn interactive_retry_policy_has_four_and_a_half_second_sleep_budget() {
    let config = CozoGuardConfig::for_interactive_db_path("/tmp/interactive.db");

    assert_eq!(config.max_attempts, 10);
    assert_eq!(
        cumulative_backoff_budget(&config),
        Duration::from_millis(4_500)
    );
}

#[test]
fn retryable_errors_include_sqlite_and_file_lock_variants() {
    assert!(is_retryable_cozo_error("database is locked (code 5)"));
    assert!(is_retryable_cozo_error("sqlite_busy"));
    assert!(is_retryable_cozo_error("Cozo write lock unavailable"));
    assert!(!is_retryable_cozo_error("relation not found"));
}

/// Contention and retryability are separate questions; #140 turned on the gap.
///
/// The write path's own failure -- `Cozo write lock unavailable ... operation
/// would block` -- and the bounded wait's expiry both mean "another process has
/// the store". Only the first is worth another round of backoff, but a caller
/// that can drop one file and continue must recognise both, or it goes back to
/// unwinding a whole repository walk over a single contended file.
#[test]
fn contention_covers_both_a_busy_store_and_an_expired_wait() {
    assert!(is_store_contention("database is locked (code 5)"));
    assert!(is_store_contention(
        "leann index: replace indexed file: Cozo write lock unavailable at \
         /repo/.archon/leann.db.archon-cozo-write.lock: operation would block"
    ));

    let expired = "index: Cozo write lock at /repo/leann.db.archon-cozo-write.lock \
                   was still held after waiting 60000ms: operation would block";
    assert!(is_store_contention(expired));
    // Still not worth retrying: we already waited the whole budget.
    assert!(!is_retryable_cozo_error(expired));

    // A fault is neither. Skipping past these would hide a real defect.
    assert!(!is_store_contention("relation not found"));
    assert!(!is_store_contention(
        "when executing against relation 'code_chunks'"
    ));
}

#[test]
fn retryable_errors_match_only_precise_busy_signals() {
    for message in [
        "database is locked",
        "database table is locked",
        "locked (code 5)",
        "code: Some(5)",
        "SQLITE_BUSY",
        "write-lock unavailable",
        "write lock unavailable",
    ] {
        assert!(is_retryable_cozo_error(message), "{message}");
    }

    for message in [
        "code 50",
        "code 500",
        "code: Some(50)",
        "code: Some(500)",
        "database is locked (code 500)",
        "database table is locked (code 500)",
        "poison error",
        "would-block",
        "unrelated code 5 prefix",
    ] {
        assert!(!is_retryable_cozo_error(message), "{message}");
    }
}
