// ---------------------------------------------------------------------------
// Failure classification — additive prefix so the LLM stops guessing
// "rate limited" when the real error is something else.
// ---------------------------------------------------------------------------

/// Conservative heuristic: classify a subagent failure string into a
/// category prefix. Only emits a specific category when the signal is
/// unambiguous (HTTP status codes, exact Rust panic format, etc.).
/// Defaults to neutral `[subagent_failure]` — the original error text
/// carries the truth regardless.
pub fn classify_failure_prefix(err: &str) -> &'static str {
    let low = err.to_lowercase();

    // Rate limit: requires HTTP 429 OR the explicit phrase "rate limit"
    // surrounded by word boundaries.
    if low.contains("429 ")
        || low.contains(" 429")
        || low == "429"
        || low.contains(" rate limit ")
        || low.starts_with("rate limit ")
        || low.ends_with(" rate limit")
        || low.contains("rate-limit")
    {
        return "[subagent_rate_limited]";
    }

    // Auth: HTTP 401 OR explicit "authentication failed" / "invalid api key" /
    // "unauthorized". Do NOT match generic "auth" substring.
    if low.contains(" 401")
        || low.contains("401 ")
        || low.contains("authentication failed")
        || low.contains("invalid api key")
        || low.contains("unauthorized")
    {
        return "[subagent_auth_failed]";
    }

    // Panic: only the explicit "panicked at" phrase (standard Rust format).
    if low.contains("panicked at") || low.contains("thread '") {
        return "[subagent_panic]";
    }

    // Timeout: "timed out", "timeout exceeded", "deadline exceeded".
    if low.contains("timed out")
        || low.contains("timeout exceeded")
        || low.contains("deadline exceeded")
    {
        return "[subagent_timeout]";
    }

    // Default — the error text carries the truth; we just label it
    // generically so the LLM knows it was a subagent failure.
    "[subagent_failure]"
}
