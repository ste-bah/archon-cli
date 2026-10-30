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
    pub(super) fn retryable_error(err: &WorkflowError) -> bool {
        let text = err.to_string().to_ascii_lowercase();
        text.contains("429")
            || text.contains("rate limit")
            || text.contains("timeout")
            || text.contains("temporar")
            || text.contains("error decoding response body")
            || text.contains("error sending request")
            || text.contains("request failed")
            || text.contains("connection reset")
            || text.contains("connection closed")
            || text.contains("broken pipe")
            || text.contains("502")
            || text.contains("503")
            || text.contains("504")
    }
}
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/characterization/classifier_characterization_variants.rs"
));
// A port error whose display is `text`; `source` is reachable only via source().
#[derive(Debug, thiserror::Error)]
#[error("{text}")]
struct Visible {
    text: String,
    #[source]
    source: Option<LlmError>,
}
fn visible(text: impl Into<String>, source: Option<LlmError>) -> WorkflowError {
    WorkflowError::port(Visible {
        text: text.into(),
        source,
    })
}
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct Shows(#[source] LlmError);
const CONTEXTS: [&str; 4] = ["HTTP 503", "ordinary context", "request failed", "HTTP 400"];
#[test]
fn c0_fanout_main_equivalence() {
    let mut failures = Vec::new();
    for (row, raw) in characterization_rows().iter().enumerate() {
        for (kind, fault, display) in characterization_variants(raw) {
            // A transparent port displays exactly the provider fault's text.
            let reference = visible(display.clone(), None);
            check_row(
                &mut failures,
                row,
                &format!("typed/{kind}"),
                retryable_error(&WorkflowError::port(fault.clone())),
                main_reference::retryable_error(&reference),
            );
            // 86fbab995 saw a formatting wrapper's text with its fault Display.
            check_row(
                &mut failures,
                row,
                &format!("shows/{kind}"),
                retryable_error(&WorkflowError::port(Shows(fault.clone()))),
                main_reference::retryable_error(&visible(display.clone(), None)),
            );
            check_row(
                &mut failures,
                row,
                &format!("auth_provider/{kind}"),
                retryable_error(&WorkflowError::port(archon_llm::auth::AuthError::Provider(
                    fault.clone(),
                ))),
                main_reference::retryable_error(&visible(display.clone(), None)),
            );
            for context in CONTEXTS
                .into_iter()
                .map(str::to_string)
                .chain([raw.clone()])
            {
                check_row(
                    &mut failures,
                    row,
                    &format!("wrapped[{context}]/{kind}"),
                    retryable_error(&visible(context.clone(), Some(fault.clone()))),
                    main_reference::retryable_error(&visible(context, None)),
                );
            }
        }
        let plain = WorkflowError::StageFailed(raw.clone());
        check_row(
            &mut failures,
            row,
            "text",
            retryable_error(&plain),
            main_reference::retryable_error(&plain),
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
