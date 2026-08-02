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
