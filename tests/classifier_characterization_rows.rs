// 47 representative bodies: plain, Bearer- and cookie-prefixed, key-suffixed, key-in-middle: 235 rows.
fn characterization_rows() -> Vec<String> {
    let bodies = [
        "context window exceeded: request timed out",
        "thread 'worker' panicked at src/foo.rs:1429:1",
        "429",
        "HTTP 429: busy",
        "status 429",
        "429 busy",
        "busy 429",
        "x429x",
        "src/1429/file.rs",
        "rate limit",
        "rate limit exceeded",
        "rate limited",
        "rate_limited",
        "rate_limit_error",
        "rate-limit",
        "ratelimit",
        "request timed out",
        "context-too-large timeout",
        "context too large timeout",
        "error decoding response body",
        "connection reset",
        "protocol: connection reset",
        "parse_error: timeout",
        "HTTP 400: request timed out",
        "HTTP 503: timeout",
        "503 429",
        "user cancelled",
        "cancelled by user",
        "aborted by user",
        "tool_result maximum length",
        "context_length_exceeded",
        "ordinary bad request",
        "no-progress stop: request timed out",
        "429: context window exceeded timeout",
        "400: request timed out",
        // Cross-field halves: main joined a rejection kind and body before matching.
        "window exceeded",
        "maximum length",
        "result maximum length",
        // Reset hints: absolute, relative, split across a prefix, broken by a
        // credential, and provider text that looks like a redaction marker.
        "reset at 2026-11-01T00:00:00Z",
        "retry in 5 minutes",
        "minutes; 2 seconds",
        "retry in 5 sk-ant-abcdefghijklmnopqrstuvwxyz123456 minutes",
        "retry in 5 ***REDACTED*** minutes",
        // Evidence only the raw text keeps: guards and statuses beside a masked line.
        "Cookie: x; host call timeout: request failed",
        "Cookie: session=x; 429",
        "Cookie: session=x; subagent timed out",
        "Cookie: x; request failed",
    ];
    bodies
        .into_iter()
        .flat_map(|body| {
            [
                body.to_string(),
                format!("Authorization: Bearer c0-secret; {body}"),
                format!("Cookie: session=c0-secret; {body}"),
                // Credentials after and inside the body: main matched around them.
                format!("{body}; sk-ant-abcdefghijklmnopqrstuvwxyz123456"),
                format!("{body} sk-ant-abcdefghijklmnopqrstuvwxyz123456 {body}"),
            ]
        })
        .collect()
}
fn check_row<T: std::fmt::Debug + PartialEq>(
    failures: &mut Vec<String>,
    row: usize,
    classifier: &str,
    actual: T,
    expected: T,
) {
    if actual != expected {
        failures.push(format!(
            "row {row} {classifier}: actual={actual:?} main={expected:?}"
        ));
    }
}
