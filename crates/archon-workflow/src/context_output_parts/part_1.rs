use serde_json::Value;

#[path = "../context_output_blocking.rs"]
mod context_output_blocking;
#[path = "../context_output_test_counts.rs"]
mod context_output_test_counts;

use context_output_blocking::{reports_missing_evidence_block, reports_waiting_for_confirmation};

pub fn output_reports_blocked(body: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    if reports_reject_verdict(body, &lower) {
        return Some("agent output declares reject/do-not-sign-off verdict".to_string());
    }
    output_reports_invalid_review_evidence_from_body(body, &lower)
}

fn output_reports_invalid_review_evidence_from_body(body: &str, lower: &str) -> Option<String> {
    if reports_waiting_for_confirmation(lower) {
        return Some(
            "agent output asks for confirmation instead of executing the stage".to_string(),
        );
    }
    if reports_explicit_blocked_status(body, lower) || reports_missing_evidence_block(lower) {
        return Some("agent output declares blocked or missing evidence".to_string());
    }
    None
}

pub fn output_reports_failed_verification(body: &str) -> Option<String> {
    output_reports_failed_verification_with_options(body, true, true)
}

/// True when the text reports a test command that matched zero tests — a
/// filtered run that executed nothing is never verification evidence. Exposed
/// so the read-only focused-verification path applies the same fail-closed
/// rule as write-output validation.
pub fn output_reports_zero_matched_tests(body: &str) -> bool {
    context_output_test_counts::reports_zero_test_filter(body, &body.to_ascii_lowercase())
}

pub fn output_reports_failed_execution(body: &str) -> Option<String> {
    output_reports_failed_verification_with_options(body, false, true)
}

pub fn output_reports_failed_execution_without_test_counts(body: &str) -> Option<String> {
    output_reports_failed_verification_with_options(body, false, false)
}

fn output_reports_failed_verification_with_options(
    body: &str,
    require_acceptance_evidence: bool,
    check_zero_test_filter: bool,
) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    if json_reports_failed_verification(body)
        .unwrap_or_else(|| reports_failed_verification_status(&lower))
    {
        return Some(
            "agent output declares failed or unverifiable verification status".to_string(),
        );
    }
    if reports_accepted_false(body, &lower) {
        return Some("agent output declares accepted=false in verification content".to_string());
    }
    if check_zero_test_filter && context_output_test_counts::reports_zero_test_filter(body, &lower)
    {
        return Some("agent output reports a filtered test command matched zero tests".to_string());
    }
    if require_acceptance_evidence && reports_accepted_without_evidence(body, &lower) {
        return Some(
            "implementation artifact declares accepted status without required evidence fields"
                .to_string(),
        );
    }
    reports_conditional_completion(&lower)
        .then(|| "agent output declares conditional completion or deferred blockers".to_string())
}

fn reports_reject_verdict(body: &str, lower: &str) -> bool {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        return json_reports_reject_verdict(&value);
    }
    lower.lines().any(|line| {
        let normalized = normalized_status_line(line);
        normalized.starts_with("verdict:reject")
            || normalized.starts_with("verdict-reject")
            || normalized.starts_with("verdict—reject")
            || normalized.starts_with("status:rejected")
            || normalized.starts_with("status=rejected")
    })
}

fn json_reports_reject_verdict(value: &Value) -> bool {
    let Value::Object(fields) = value else {
        return false;
    };
    value_string_field_is(fields, "verdict", "reject")
        || value_string_field_is(fields, "status", "rejected")
        || ["body", "result", "output"]
            .iter()
            .filter_map(|field| fields.get(*field))
            .any(json_reports_reject_verdict)
}

fn reports_explicit_blocked_status(body: &str, lower: &str) -> bool {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        return json_reports_stage_blocked_status(&value);
    }
    let lines: Vec<_> = lower.lines().collect();
    lines.iter().enumerate().any(|(idx, line)| {
        let normalized = normalized_status_line(line);
        normalized.starts_with("status:blocked")
            || normalized.starts_with("-status:blocked")
            || normalized.starts_with("status=blocked")
            || (normalized == "status" && next_line_is_blocked(&lines, idx))
    })
}

fn json_reports_stage_blocked_status(value: &Value) -> bool {
    let Value::Object(fields) = value else {
        return false;
    };
    value_string_field_is(fields, "status", "blocked")
        || ["body", "result", "output"]
            .iter()
            .filter_map(|field| fields.get(*field))
            .any(json_reports_stage_blocked_status)
}

fn value_string_field_is(
    fields: &serde_json::Map<String, Value>,
    field: &str,
    expected: &str,
) -> bool {
    fields
        .get(field)
        .and_then(Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

fn next_line_is_blocked(lines: &[&str], idx: usize) -> bool {
    lines
        .iter()
        .skip(idx + 1)
        .map(|line| normalized_status_value(line.trim()))
        .find(|line| !line.is_empty())
        .is_some_and(|line| line.starts_with("blocked"))
}

fn reports_failed_verification_status(lower: &str) -> bool {
    let lines: Vec<_> = lower.lines().collect();
    contains_blocking_structured_field(lower)
        || lines.iter().enumerate().any(|(idx, line)| {
            let normalized = normalized_status_line(line);
            starts_with_blocking_status_field(&normalized)
                || (is_verification_status_heading(&normalized)
                    && next_line_has_blocking_status(&lines, idx))
        })
}

fn reports_conditional_completion(lower: &str) -> bool {
    const PHRASES: &[&str] = &[
        "conditionally accepted",
        "conditional acceptance",
        "non-blocking fail-closed deferral",
        "accepted without claiming",
        "accepted without implementing",
        "implementation is accepted without",
        "no verified source writer",
        "status: **not ready",
    ];
    PHRASES.iter().any(|phrase| lower.contains(phrase)) || reports_readiness_flag_false(lower)
}

/// Generic readiness contradiction: any `*_ready` flag reported false (JSON or
/// backticked markdown) while the output otherwise claims completion — covers
/// deployment_ready, production_ready, and any domain-specific readiness flag
/// without naming a domain.
fn reports_readiness_flag_false(lower: &str) -> bool {
    ["_ready\":false", "_ready\": false", "_ready`: `false`"]
        .iter()
        .any(|needle| lower.contains(needle))
}

fn reports_accepted_false(body: &str, lower: &str) -> bool {
    if serde_json::from_str::<Value>(body)
        .ok()
        .is_some_and(|value| value_has_accepted_false(&value))
    {
        return true;
    }

    lower.lines().any(|line| {
        let normalized = normalized_status_line(line);
        normalized.starts_with("accepted:false") || normalized.starts_with("accepted=false")
    })
}

fn value_has_accepted_false(value: &Value) -> bool {
    match value {
        Value::Object(fields) => fields.iter().any(|(field, value)| {
            if is_non_final_attempts_field(field) {
                return false;
            }
            (field == "accepted" && value == &Value::Bool(false)) || value_has_accepted_false(value)
        }),
        Value::Array(values) => values.iter().any(value_has_accepted_false),
        _ => false,
    }
}

fn reports_accepted_without_evidence(body: &str, lower: &str) -> bool {
    if !reports_accepted_status(body, lower) {
        return false;
    }
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        return !json_has_acceptance_evidence(&value);
    }
    !((text_has_any(lower, FILE_EVIDENCE_FIELDS) || text_has_any(lower, ARTIFACT_EVIDENCE_FIELDS))
        && text_has_any(lower, VERIFICATION_EVIDENCE_FIELDS)
        && text_has_any(lower, COMPLETION_EVIDENCE_FIELDS))
}

fn reports_accepted_status(body: &str, lower: &str) -> bool {
    if serde_json::from_str::<Value>(body)
        .ok()
        .is_some_and(|value| value_has_accepted_envelope_status(&value))
    {
        return true;
    }
    lower.lines().any(|line| {
        matches!(
            normalized_status_line(line).as_str(),
            "status:accepted" | "status=accepted"
        )
    })
}

fn json_has_acceptance_evidence(value: &Value) -> bool {
    match value {
        Value::Object(fields) => {
            if object_has_accepted_status(fields) {
                return (json_has_non_empty_field(value, FILE_EVIDENCE_FIELDS)
                    || json_has_non_empty_field(value, ARTIFACT_EVIDENCE_FIELDS))
                    && json_has_non_empty_field(value, VERIFICATION_EVIDENCE_FIELDS)
                    && json_has_field(value, COMPLETION_EVIDENCE_FIELDS);
            }
            fields
                .iter()
                .filter(|(field, _)| !is_non_final_attempts_field(field))
                .any(|(_, value)| json_has_acceptance_evidence(value))
        }
        Value::Array(values) => values.iter().any(json_has_acceptance_evidence),
        _ => false,
    }
}

const FILE_EVIDENCE_FIELDS: &[&str] = &[
    "target_files",
    "changed_files",
    "files_changed",
    "source_files_changed",
    "source_files",
    "declared_target_files",
];

const ARTIFACT_EVIDENCE_FIELDS: &[&str] = &["artifacts", "artifact_paths", "artifacts_checked"];

const VERIFICATION_EVIDENCE_FIELDS: &[&str] = &[
    "acceptance_checks",
    "commands_run",
    "verification",
    "tests",
    "test_results",
    "tests_run",
    "focused_tests",
    "required_tests",
];

const COMPLETION_EVIDENCE_FIELDS: &[&str] = &[
    "residual_gaps",
    "remaining_gaps",
    "remaining_blockers",
    "unresolved_blockers",
    "implementation_summary",
    "summary",
    "notes",
    "file_size_check",
    "line_count_evidence",
    "line_counts",
    "safety_audit",
];
