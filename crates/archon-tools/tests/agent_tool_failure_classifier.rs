//! Tests for `classify_failure_prefix` — the conservative heuristic that
//! prefixes subagent failure strings so the LLM stops guessing "rate limited"
//! when the real error is something else.

use archon_tools::agent_tool::classify_failure_prefix;

#[test]
fn rate_limit_429_exact() {
    assert_eq!(
        classify_failure_prefix("HTTP 429: rate limit exceeded"),
        "[subagent_rate_limited]"
    );
}

#[test]
fn context_limit_does_not_match_rate_limit() {
    // "limit" alone is too weak — must be "rate limit" with word boundaries
    assert_eq!(
        classify_failure_prefix("context limit reached"),
        "[subagent_failure]"
    );
}

#[test]
fn rust_panic_format() {
    assert_eq!(
        classify_failure_prefix("thread 'tokio-rt-worker' panicked at src/foo.rs:42"),
        "[subagent_panic]"
    );
}

#[test]
fn auth_401_unauthorized() {
    assert_eq!(
        classify_failure_prefix("ApiError(401): unauthorized"),
        "[subagent_auth_failed]"
    );
}

#[test]
fn author_substring_does_not_match_auth() {
    // "author" contains "auth" as substring — must NOT match
    assert_eq!(
        classify_failure_prefix("author of the file"),
        "[subagent_failure]"
    );
}

#[test]
fn timeout_explicit() {
    assert_eq!(
        classify_failure_prefix("task timed out after 30s"),
        "[subagent_timeout]"
    );
}

#[test]
fn connection_reset_is_default() {
    assert_eq!(
        classify_failure_prefix("connection reset by peer"),
        "[subagent_failure]"
    );
}

#[test]
fn rate_limit_with_hyphen() {
    assert_eq!(
        classify_failure_prefix("rate-limit exceeded for API key"),
        "[subagent_rate_limited]"
    );
}

#[test]
fn panic_with_panicked_at() {
    assert_eq!(
        classify_failure_prefix("panicked at 'assertion failed: x > 0', src/main.rs:10"),
        "[subagent_panic]"
    );
}

#[test]
fn auth_invalid_api_key() {
    assert_eq!(
        classify_failure_prefix("invalid api key: key not found"),
        "[subagent_auth_failed]"
    );
}

#[test]
fn timeout_deadline_exceeded() {
    assert_eq!(
        classify_failure_prefix("deadline exceeded: context deadline exceeded"),
        "[subagent_timeout]"
    );
}

#[test]
fn generic_failure_default_label() {
    assert_eq!(
        classify_failure_prefix("something went wrong"),
        "[subagent_failure]"
    );
}
