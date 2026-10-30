// C0: reference function bodies copied unchanged from 86fbab995.
// Intended classification differences: none. Legacy-only types keep raw test inputs.
// Only reference visibility is widened to pub(super); function bodies are unchanged.
use super::*;
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/characterization/classifier_characterization_rows.rs"
));
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/characterization/classifier_characterization_variants.rs"
));
#[path = "provider_fault_characterization_reference_tests.rs"]
mod main_reference;
// Adapt legacy test-only types; every reference function body stays unchanged.
fn check_pressure_variant(
    failures: &mut Vec<String>,
    row: usize,
    name: &str,
    fault: &LlmError,
    raw: &str,
) {
    use main_reference::LlmError as MainError;
    let reference = match fault {
        LlmError::Http(_) => MainError::Http(raw.into()),
        // The server message was a redacted DiagnosticMessage at 86fbab995.
        LlmError::Server { status, .. } => MainError::Server {
            status: *status,
            message: archon_observability::redaction::redact_text(raw),
        },
        LlmError::Rejected { error_type, .. } => MainError::Rejected {
            error_type: error_type.to_string(),
            message: raw.into(),
        },
        LlmError::ContextWindowExceeded { .. } => MainError::ContextWindowExceeded {
            provider_message: raw.into(),
            provider: None,
            model: None,
        },
        // Main's getters use their wildcard arm for all remaining families.
        _ => MainError::Other,
    };
    check_row(
        failures,
        row,
        &format!("pressure/{name}"),
        fault.request_pressure_kind(),
        reference.request_pressure_kind(),
    );
    check_row(
        failures,
        row,
        &format!("context/{name}"),
        fault.is_context_window_exceeded(),
        reference.is_context_window_exceeded(),
    );
}
fn variant(fault: &LlmError) -> &'static str {
    match fault {
        LlmError::Http(_) => "http",
        LlmError::Transport(_) => "transport",
        LlmError::RateLimited { .. } => "rate_limited",
        LlmError::Overloaded { .. } => "overloaded",
        LlmError::QuotaExceeded(_) => "quota",
        LlmError::Aborted => "aborted",
        LlmError::Server { .. } => "server",
        LlmError::Rejected { .. } => "rejected",
        LlmError::ContextWindowExceeded { .. } => "context",
        _ => "other",
    }
}
fn check_stream(
    failures: &mut Vec<String>,
    row: usize,
    name: &str,
    fault: LlmError,
    reference: main_reference::LlmError,
) {
    check_row(
        failures,
        row,
        &format!("{name}/variant"),
        variant(&fault),
        reference.variant(),
    );
    check_row(
        failures,
        row,
        &format!("{name}/operational"),
        fault.is_operational(),
        reference.is_operational(),
    );
    // Status evidence in the display text, as pipeline retry scans it.
    for code in [400u16, 401, 403, 404, 422, 429, 500, 502, 503] {
        check_row(
            failures,
            row,
            &format!("{name}/mentions_{code}"),
            fault.mentions_status(code),
            reference.display().contains(&code.to_string()),
        );
    }
    check_row(
        failures,
        row,
        &format!("{name}/permanent"),
        fault.is_permanent(),
        reference.is_permanent(),
    );
    check_row(
        failures,
        row,
        &format!("{name}/pressure"),
        fault.request_pressure_kind(),
        reference.request_pressure_kind(),
    );
    check_row(
        failures,
        row,
        &format!("{name}/context"),
        fault.is_context_window_exceeded(),
        reference.is_context_window_exceeded(),
    );
}
// Stream kinds: main composed "{kind}: {message}" for unknown kinds before classifying.
const STREAM_KINDS: [&str; 15] = [
    "protocol",
    "parse_error",
    "peer",
    "context",
    "HTTP 503",
    "503",
    "",
    "transport_idle",
    "aborted",
    "network",
    "rate_limited",
    "overloaded_error",
    "billing_error",
    "invalid_request_error",
    "request_too_large",
];
// Prefixes the production code composes, plus a split duration.
const RESET_PREFIXES: [&str; 4] = ["", "peer: ", "retry in 5 ", "server error (429): "];
#[test]
fn c0_reset_hint_main_equivalence() {
    let observed = chrono::DateTime::parse_from_rfc3339("2026-10-29T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let mut failures = Vec::new();
    for (row, raw) in characterization_rows().iter().enumerate() {
        for prefix in RESET_PREFIXES {
            check_row(
                &mut failures,
                row,
                &format!("reset[{prefix}]"),
                crate::provider::DiagnosticMessage::from(raw.as_str())
                    .with_prefix(prefix)
                    .reset_hint_at(observed),
                main_reference::reset_hint_from_text(&format!("{prefix}{raw}"), observed),
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
#[test]
fn c0_stream_composition_main_equivalence() {
    let mut failures = Vec::new();
    for (row, raw) in characterization_rows().iter().enumerate() {
        for kind in STREAM_KINDS {
            check_stream(
                &mut failures,
                row,
                &format!("stream[{kind}]"),
                from_stream_error(kind, raw.as_str()),
                main_reference::from_stream_error(kind, raw),
            );
            // The pre-content probe composes a second prefix before the same mapping.
            // The production pre-content forwarder, against its frozen copy.
            check_stream(
                &mut failures,
                row,
                &format!("pre_content[{kind}]"),
                crate::retry::stream_error_before_content(kind, raw.as_str()),
                main_reference::stream_error_before_content(kind, raw),
            );
            let prefix = format!("stream failed before producing content ({kind}): ");
            check_stream(
                &mut failures,
                row,
                &format!("nested[{kind}]"),
                from_stream_error(
                    kind,
                    crate::provider::DiagnosticMessage::from(raw.as_str()).with_prefix(&prefix),
                ),
                main_reference::from_stream_error(kind, &format!("{prefix}{raw}")),
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
#[test]
fn c0_provider_fault_main_equivalence() {
    let mut failures = Vec::new();
    for (row, raw) in characterization_rows().iter().enumerate() {
        for (name, fault, _) in characterization_variants(raw) {
            check_pressure_variant(&mut failures, row, name, &fault, raw);
        }
        // Also exercise pressure assembled from the structured kind and body.
        let contextual = LlmError::Rejected {
            error_type: "context_length_exceeded".into(),
            message: raw.into(),
        };
        check_pressure_variant(
            &mut failures,
            row,
            "rejected_context_kind",
            &contextual,
            raw,
        );
        let diagnostic = crate::provider::DiagnosticMessage::from(raw);
        check_row(
            &mut failures,
            row,
            "cancel",
            diagnostic.is_user_cancelled(),
            main_reference::is_cancelled_stream_error("", raw),
        );
        check_row(
            &mut failures,
            row,
            "operational",
            LlmError::Http(diagnostic.clone()).is_operational(),
            main_reference::LlmError::Http(raw.clone()).is_operational(),
        );
        for status in [
            None,
            Some(400),
            Some(413),
            Some(422),
            Some(429),
            Some(500),
            Some(503),
        ] {
            for kind in [
                None,
                Some("context_length_exceeded"),
                Some("invalid_request_error"),
            ] {
                check_row(
                    &mut failures,
                    row,
                    "pressure",
                    crate::context_window::classify_request_pressure_error(status, kind, None, raw),
                    main_reference::classify_request_pressure_error(status, kind, None, raw),
                );
                check_row(
                    &mut failures,
                    row,
                    "context",
                    crate::context_window::classify_context_window_error(
                        status, kind, None, raw, None, None,
                    )
                    .is_some(),
                    main_reference::classify_context_window_error(
                        status, kind, None, raw, None, None,
                    )
                    .is_some(),
                );
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// The redactor is a classifier input (86fbab995's Display and stored text
// called it), so it must be that commit's redactor byte for byte.
#[test]
fn c0_redactor_is_frozen_86fbab995() {
    assert_eq!(
        include_str!("../../archon-observability/src/shared_redactor.rs"),
        include_str!("../../../tests/frozen_86fbab995/shared_redactor.rs")
    );
    assert_eq!(
        include_str!("../../archon-observability/src/redaction.rs"),
        include_str!("../../../tests/frozen_86fbab995/redaction.rs")
    );
}
