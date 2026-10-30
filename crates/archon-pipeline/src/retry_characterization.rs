// C0: reference function bodies copied unchanged from 86fbab995.
// Intended classification differences: none. Legacy-only types keep raw test inputs.
// Only reference visibility is widened to pub(super); function bodies are unchanged.
use super::*;
use archon_llm::provider::LlmError;
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/characterization/classifier_characterization_rows.rs"
));
mod main_reference {
    #![allow(dead_code)]
    use super::*;
    pub(super) fn classify_error(err: &anyhow::Error) -> RetryableError {
        // Check the chain for a concrete RetryableError.
        if let Some(re) = err.downcast_ref::<RetryableError>() {
            return re.clone();
        }

        // Fallback: scan the display string for HTTP status patterns.
        let msg = err.to_string();

        // Retryable status codes.
        for code in &[429u16, 500, 502, 503] {
            let pattern = format!("{}", code);
            if msg.contains(&pattern) {
                return RetryableError::Retryable {
                    status: *code,
                    message: msg.clone(),
                    retry_after: None,
                };
            }
        }

        // Non-retryable status codes.
        for code in &[400u16, 401, 403, 404, 422] {
            let pattern = format!("{}", code);
            if msg.contains(&pattern) {
                return RetryableError::NonRetryable {
                    status: *code,
                    message: msg.clone(),
                };
            }
        }

        // Default: non-retryable.
        RetryableError::NonRetryable {
            status: 0,
            message: msg,
        }
    }
}
fn signature(value: RetryableError) -> (bool, u16, Option<std::time::Duration>) {
    match value {
        RetryableError::Retryable {
            status,
            retry_after,
            ..
        } => (true, status, retry_after),
        RetryableError::NonRetryable { status, .. } => (false, status, None),
    }
}
// A source-carrying wrapper: its own display is all that main's scan could see.
#[derive(Debug, thiserror::Error)]
#[error("provider call failed")]
struct Wrapped(#[source] LlmError);
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct Shows(#[source] LlmError);
const CONTEXTS: [&str; 4] = ["HTTP 503", "ordinary context", "HTTP 400", "429 busy"];
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/characterization/classifier_characterization_variants.rs"
));
#[test]
fn c0_retry_main_equivalence() {
    let mut failures = Vec::new();
    for (row, raw) in characterization_rows().iter().enumerate() {
        for (kind, fault, display) in characterization_variants(raw) {
            let typed = anyhow::Error::new(fault.clone());
            let reference = anyhow::anyhow!(display.clone());
            check_row(
                &mut failures,
                row,
                kind,
                signature(classify_error(&typed)),
                signature(main_reference::classify_error(&reference)),
            );
            // Context wrappers: main's display is the outermost context only.
            for context in CONTEXTS
                .into_iter()
                .map(str::to_string)
                .chain([raw.clone()])
            {
                let typed = anyhow::Error::new(fault.clone()).context(context.clone());
                let reference = anyhow::anyhow!(display.clone()).context(context.clone());
                check_row(
                    &mut failures,
                    row,
                    &format!("context[{context}]/{kind}"),
                    signature(classify_error(&typed)),
                    signature(main_reference::classify_error(&reference)),
                );
            }
            // A wrapper formatting the fault: main saw its own fault Display.
            let typed = anyhow::Error::new(Shows(fault.clone()));
            check_row(
                &mut failures,
                row,
                &format!("shows/{kind}"),
                signature(classify_error(&typed)),
                signature(main_reference::classify_error(&anyhow::anyhow!(
                    display.clone()
                ))),
            );
            // Every workspace type that wraps a fault, formatted as it was.
            for (name, typed, shown) in [
                (
                    "auth_provider",
                    anyhow::Error::new(archon_llm::auth::AuthError::Provider(fault.clone())),
                    display.clone(),
                ),
                (
                    "compaction_provider",
                    anyhow::Error::new(archon_core::agent::autocompact::CompactionError::Provider(
                        fault.clone(),
                    )),
                    format!("provider summary failed: {display}"),
                ),
            ] {
                check_row(
                    &mut failures,
                    row,
                    &format!("{name}/{kind}"),
                    signature(classify_error(&typed)),
                    signature(main_reference::classify_error(&anyhow::anyhow!(shown))),
                );
            }
            let typed = anyhow::Error::new(Wrapped(fault.clone()));
            let reference = anyhow::anyhow!("provider call failed");
            check_row(
                &mut failures,
                row,
                &format!("wrapped/{kind}"),
                signature(classify_error(&typed)),
                signature(main_reference::classify_error(&reference)),
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
