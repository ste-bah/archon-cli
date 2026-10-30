// C0: reference function bodies copied unchanged from 86fbab995.
// Intended classification differences: none. Legacy-only types keep raw test inputs.
// Only reference visibility is widened to pub(super); function bodies are unchanged.
use super::*;
use archon_llm::provider::LlmError;
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/classifier_characterization_rows.rs"
));
mod main_reference {
    #![allow(dead_code)]
    use crate::llm_client_port::WorkflowAgentCall;
    // Frozen 86fbab995 dependencies, so no baseline reaches current code.
    mod crate_error {
        pub const HOST_CALL_TIMEOUT_MARKER: &str = "host call timeout:";
        pub const NO_PROGRESS_STOP_MARKER: &str = "no-progress stop:";
        pub const INACTIVITY_TIMEOUT_MARKER: &str = "subagent inactivity timeout:";
        pub fn is_host_call_timeout_text(error: &str) -> bool {
            error.contains(HOST_CALL_TIMEOUT_MARKER)
        }
        pub fn is_no_progress_stop_text(error: &str) -> bool {
            error.contains(NO_PROGRESS_STOP_MARKER)
        }
        pub fn is_inactivity_timeout_text(error: &str) -> bool {
            error.contains(INACTIVITY_TIMEOUT_MARKER)
        }
    }
    pub(super) mod provider_fault {
        pub const TRANSPORT_STALL_MARKER: &str =
            "provider transport stall (no answer for a whole no-progress window; resumable):";
        pub fn status_field(text: &str) -> Option<u16> {
            let text = text.trim().to_ascii_lowercase();
            let value = [
                "http error: ",
                "http ",
                "status ",
                "server error (",
                "server error ",
            ]
            .iter()
            .find_map(|prefix| text.strip_prefix(prefix))
            .unwrap_or(&text);
            let digits: String = value.chars().take_while(char::is_ascii_digit).collect();
            (digits.len() == 3).then(|| digits.parse().ok()).flatten()
        }
        // Legacy LlmError shape: variants carry main's raw diagnostic text.
        pub enum LlmError {
            RateLimited,
            Overloaded,
            Transport(String),
            Server { status: u16 },
            Http(String),
            Other,
        }
        impl LlmError {
            #[rustfmt::skip]
            pub fn is_operational(&self) -> bool {
                match self {
                    Self::RateLimited { .. } | Self::Overloaded { .. } | Self::Transport(_) => true,
                    Self::Server { status, .. } => matches!(*status, 408 | 429 | 500..=599),
                    Self::Http(text) => {
                        if let Some(status) = status_field(text) {
                            return matches!(status, 408 | 429 | 500..=599);
                        }
                        // An opened stream's idle/protocol exhaustion is no-progress,
                        // not proof of an unavailable endpoint. It earns no clock credit.
                        if text.starts_with("protocol:")
                            || text.starts_with("parse_error:")
                            || text.contains(TRANSPORT_STALL_MARKER)
                            || text.contains("provider transport stalled:")
                            || text.contains("stream idle timeout")
                            || text.contains("no complete answer")
                        {
                            return false;
                        }
                        let text = text.to_ascii_lowercase();
                        [
                            "connection",
                            "socket",
                            "broken pipe",
                            "error sending request",
                            "request failed",
                            "http_error",
                            "overloaded",
                            "server_error",
                            "rate limit",
                            "timed out",
                            "timeout",
                            "temporar",
                        ]
                        .iter()
                        .any(|marker| text.contains(marker))
                    }
                    _ => false,
                }
            }
        }
    }

    pub(super) fn retry_error_class(message: &str) -> &'static str {
        let text = message.to_ascii_lowercase();
        if text.contains("rate limit") || text.contains("rate limited") || text.contains("429") {
            "rate_limit"
        } else if text.contains("overloaded") {
            "overload"
        } else if text.contains("connection") || text.contains("socket") || text.contains("request")
        {
            "connection"
        } else {
            "transient"
        }
    }
    pub(super) fn transient_live_agent_error(message: &str) -> bool {
        // Issue-213 C2d: the runner's own stop, never a provider blink.
        // Issue 364: a host's completed no-progress decision is not a provider
        // failure to replay. Ordinary connection faults are retried here.
        if crate_error::is_no_progress_stop_text(message) {
            return false;
        }
        let text = message.to_ascii_lowercase();
        if let Some(status) = provider_fault::status_field(&text) {
            return matches!(status, 408 | 429 | 500..=599);
        }
        if non_retryable_provider_fault(&text) {
            return false;
        }
        [
            "llm stream error",
            "server_error",
            "error decoding response body",
            "error sending request",
            "request failed",
            "connection reset",
            "connection closed",
            "connection refused",
            "broken pipe",
            "timed out",
            "timeout",
            "temporar",
            "rate limit",
            "server overloaded",
        ]
        .iter()
        .any(|needle| text.contains(needle))
    }
    pub(super) fn transient_live_agent_error_for_request(
        request: &WorkflowAgentCall,
        message: &str,
    ) -> bool {
        if crate_error::is_host_call_timeout_text(message) {
            return false;
        }
        if request.disable_auto_background && foreground_subagent_timeout_or_cancel(message) {
            return false;
        }
        transient_live_agent_error(message)
    }
    fn foreground_subagent_timeout_or_cancel(message: &str) -> bool {
        let text = message.to_ascii_lowercase();
        crate_error::is_inactivity_timeout_text(message)
            || text.contains("subagent timed out")
            || text.contains("subagent cancelled")
            || text.contains("subagent auto-backgrounded")
    }
    pub(super) fn non_retryable_provider_fault(text: &str) -> bool {
        if [
            "context window exceeded",
            "context-too-large",
            "context too large",
        ]
        .iter()
        .any(|marker| text.contains(marker))
        {
            return true;
        }

        false
    }
}
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/classifier_characterization_variants.rs"
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
// Wrappers whose own Display formats the contained fault, as host ports do.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct Shows(#[source] LlmError);
#[derive(Debug, thiserror::Error)]
#[error("request failed: {0}")]
struct ShowsAfter(#[source] LlmError);
const CONTEXTS: [&str; 4] = ["HTTP 503", "ordinary context", "request failed", "HTTP 400"];
// The legacy fault main's source-chain lookup found: same variant, raw text.
fn main_fault(fault: &LlmError, raw: &str) -> main_reference::provider_fault::LlmError {
    use main_reference::provider_fault::LlmError as Main;
    match fault {
        LlmError::RateLimited { .. } => Main::RateLimited,
        LlmError::Overloaded { .. } => Main::Overloaded,
        LlmError::Transport(_) => Main::Transport(raw.into()),
        LlmError::Server { status, .. } => Main::Server { status: *status },
        LlmError::Http(_) => Main::Http(raw.into()),
        _ => Main::Other,
    }
}
// Main's agent loop: typed is_operational() from the source chain, OR the
// request-aware text policy over the visible display. Both are frozen copies.
fn main_agent_retryable(
    request: &WorkflowAgentCall,
    fault: &main_reference::provider_fault::LlmError,
    display: &str,
) -> bool {
    fault.is_operational()
        || main_reference::transient_live_agent_error_for_request(request, display)
}
#[test]
fn c0_llm_retry_main_equivalence() {
    let mut failures = Vec::new();
    for (row, raw) in characterization_rows().iter().enumerate() {
        for (kind, fault, display) in characterization_variants(raw) {
            for disable in [false, true] {
                let request = super::tests::request(disable);
                check_row(
                    &mut failures,
                    row,
                    &format!("agent[{disable}]/{kind}"),
                    // Production passes today's Display; main saw its own.
                    {
                        let error = WorkflowError::port(fault.clone());
                        agent_error_retryable(&request, &error, &error.to_string())
                    },
                    main_agent_retryable(&request, &main_fault(&fault, raw), &display),
                );
                for context in CONTEXTS.into_iter().map(str::to_string).chain([
                    raw.clone(),
                    format!("request failed: {display}"),
                    format!("host call timeout: {display}"),
                ]) {
                    check_row(
                        &mut failures,
                        row,
                        &format!("agent[{disable}][{context}]/{kind}"),
                        agent_error_retryable(
                            &request,
                            &visible(context.clone(), Some(fault.clone())),
                            &context,
                        ),
                        main_agent_retryable(&request, &main_fault(&fault, raw), &context),
                    );
                }
            }
            for context in CONTEXTS
                .into_iter()
                .map(str::to_string)
                .chain([raw.clone()])
            {
                let wrapped = visible(context.clone(), Some(fault.clone()));
                check_row(
                    &mut failures,
                    row,
                    &format!("send[{context}]/{kind}"),
                    transient_workflow_error(&wrapped),
                    main_reference::transient_live_agent_error(&context),
                );
                check_row(
                    &mut failures,
                    row,
                    &format!("log[{context}]/{kind}"),
                    retry_error_class(&wrapped),
                    main_reference::retry_error_class(&context),
                );
            }
            // 86fbab995 saw the wrapper text with that commit's fault Display.
            // `typed`: 86fbab995's source-chain lookup found the fault. A
            // transparent AuthError::Provider hides it from that lookup.
            for (name, wrapped, shown, typed) in [
                (
                    "shows",
                    WorkflowError::port(Shows(fault.clone())),
                    display.clone(),
                    true,
                ),
                (
                    "shows_after",
                    WorkflowError::port(ShowsAfter(fault.clone())),
                    format!("request failed: {display}"),
                    true,
                ),
                (
                    "auth_provider",
                    WorkflowError::port(archon_llm::auth::AuthError::Provider(fault.clone())),
                    display.clone(),
                    false,
                ),
            ] {
                let main = if typed {
                    main_fault(&fault, raw)
                } else {
                    main_reference::provider_fault::LlmError::Other
                };
                for disable in [false, true] {
                    let request = super::tests::request(disable);
                    check_row(
                        &mut failures,
                        row,
                        &format!("agent[{disable}][{name}]/{kind}"),
                        agent_error_retryable(&request, &wrapped, &wrapped.to_string()),
                        main_agent_retryable(&request, &main, &shown),
                    );
                }
                check_row(
                    &mut failures,
                    row,
                    &format!("send[{name}]/{kind}"),
                    transient_workflow_error(&wrapped),
                    main_reference::transient_live_agent_error(&shown),
                );
                check_row(
                    &mut failures,
                    row,
                    &format!("log[{name}]/{kind}"),
                    retry_error_class(&wrapped),
                    main_reference::retry_error_class(&shown),
                );
            }
            let error = WorkflowError::port(fault);
            check_row(
                &mut failures,
                row,
                &format!("send/{kind}"),
                transient_workflow_error(&error),
                main_reference::transient_live_agent_error(&display),
            );
            check_row(
                &mut failures,
                row,
                &format!("log/{kind}"),
                retry_error_class(&error),
                main_reference::retry_error_class(&display),
            );
        }
        check_row(
            &mut failures,
            row,
            "text",
            transient_live_agent_error(raw),
            main_reference::transient_live_agent_error(raw),
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
