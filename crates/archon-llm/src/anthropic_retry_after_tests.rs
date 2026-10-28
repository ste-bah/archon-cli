use super::*;

fn rate_limited(header: Option<&str>, body: &str) -> (u64, bool) {
    match classify_error(429, body, header) {
        ApiError::RateLimited {
            retry_after_secs,
            from_provider,
            ..
        } => (retry_after_secs, from_provider),
        other => panic!("expected a rate limit, got {other:?}"),
    }
}

/// Fails before the fix: an HTTP-date Retry-After was dropped silently
/// for a 30 s wait that then read as the provider's.
#[test]
fn a_retry_after_http_date_is_the_wait_until_that_time() {
    let now = chrono::DateTime::parse_from_rfc2822("Wed, 21 Oct 2026 07:28:00 GMT")
        .unwrap()
        .with_timezone(&chrono::Utc);
    assert_eq!(
        retry_after_from_header("Wed, 21 Oct 2026 07:30:00 GMT", now),
        Some(120)
    );
    assert_eq!(
        retry_after_from_header("Wed, 21 Oct 2026 07:00:00 GMT", now),
        Some(0)
    );
    assert_eq!(retry_after_from_header(" 45 ", now), Some(45));
    assert_eq!(retry_after_from_header("soon", now), None);
}

#[test]
fn a_rate_limit_says_whether_the_provider_named_its_wait() {
    assert_eq!(rate_limited(Some("12"), ""), (12, true));
    assert_eq!(rate_limited(None, r#"{"retry_after": 9}"#), (9, true));
    assert_eq!(rate_limited(Some("soon"), "{}"), (30, false));
    assert_eq!(rate_limited(None, "not json"), (30, false));
}
