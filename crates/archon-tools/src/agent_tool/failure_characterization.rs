// C0: reference function bodies copied unchanged from 86fbab995.
// Intended classification differences: none. Legacy-only types keep raw test inputs.
// Only reference visibility is widened to pub(super); function bodies are unchanged.
use super::*;
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/characterization/classifier_characterization_rows.rs"
));
mod main_reference {
    #![allow(dead_code)]

    pub(super) fn classify_failure_prefix(err: &str) -> &'static str {
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
}
#[test]
fn c0_agent_tool_main_equivalence() {
    let mut failures = Vec::new();
    for (row, raw) in characterization_rows().iter().enumerate() {
        check_row(
            &mut failures,
            row,
            "failure",
            classify_failure_prefix(raw),
            main_reference::classify_failure_prefix(raw),
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
