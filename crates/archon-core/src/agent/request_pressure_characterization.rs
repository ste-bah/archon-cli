// C0: reference function bodies copied unchanged from 86fbab995.
// Intended classification differences: none. Legacy-only types keep raw test inputs.
// Only reference visibility is widened to pub(super); function bodies are unchanged.
use super::*;
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/classifier_characterization_rows.rs"
));
mod main_reference {
    #![allow(dead_code)]

    pub(super) fn rate_limit_message(message: &str) -> bool {
        let lower = message.to_ascii_lowercase();
        lower.contains("429")
            || lower.contains("rate limit")
            || lower.contains("rate_limited")
            || lower.contains("rate_limit_error")
    }
}
#[test]
fn c0_request_pressure_main_equivalence() {
    let mut failures = Vec::new();
    for (row, raw) in characterization_rows().iter().enumerate() {
        check_row(
            &mut failures,
            row,
            "rate_limit",
            is_rate_limited_error(&archon_llm::provider::LlmError::Http(raw.into())),
            main_reference::rate_limit_message(raw),
        );
        // 86fbab995 stored a server message redacted (DiagnosticMessage).
        check_row(
            &mut failures,
            row,
            "rate_limit/server",
            is_rate_limited_error(&archon_llm::provider::LlmError::Server {
                status: 400,
                message: raw.into(),
                pipeline_retry_after_secs: None,
            }),
            main_reference::rate_limit_message(&archon_observability::redaction::redact_text(raw)),
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
