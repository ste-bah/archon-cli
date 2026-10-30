// C0: unchanged classifier bodies from 86fbab995. Intended differences: none.
use super::*;
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/classifier_characterization_rows.rs"
));
mod main_reference {
    pub(super) fn is_retryable(status: u16, body: &str) -> bool {
        matches!(status, 429 | 500 | 502 | 503 | 504)
            || contains_any(
                body,
                &[
                    "rate limit",
                    "rate-limit",
                    "ratelimit",
                    "overloaded",
                    "service unavailable",
                    "upstream connect",
                    "connection refused",
                ],
            )
    }

    pub(super) fn contains_any(body: &str, needles: &[&str]) -> bool {
        let body = body.to_lowercase();
        needles.iter().any(|needle| body.contains(needle))
    }
}
#[test]
fn c0_codex_rate_limit_main_equivalence() {
    let mut failures = Vec::new();
    for (row, raw) in characterization_rows().iter().enumerate() {
        for status in [0, 400, 408, 429, 500, 502, 503, 504] {
            check_row(
                &mut failures,
                row,
                &format!("codex/{status}"),
                is_retryable(status, raw),
                main_reference::is_retryable(status, raw),
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
