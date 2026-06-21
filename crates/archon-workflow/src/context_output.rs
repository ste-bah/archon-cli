use serde_json::Value;

#[path = "context_output_blocking.rs"]
mod context_output_blocking;
#[path = "context_output_test_counts.rs"]
mod context_output_test_counts;

use context_output_blocking::{reports_missing_evidence_block, reports_waiting_for_confirmation};

pub fn output_reports_blocked(body: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    if reports_reject_verdict(body, &lower) {
        return Some("agent output declares reject/do-not-sign-off verdict".to_string());
    }
    output_reports_invalid_review_evidence_from_body(body, &lower)
}

pub fn output_reports_invalid_review_evidence(body: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    output_reports_invalid_review_evidence_from_body(body, &lower)
}

fn output_reports_invalid_review_evidence_from_body(body: &str, lower: &str) -> Option<String> {
    if reports_waiting_for_confirmation(&lower) {
        return Some(
            "agent output asks for confirmation instead of executing the stage".to_string(),
        );
    }
    if reports_explicit_blocked_status(body, lower) || reports_missing_evidence_block(&lower) {
        return Some("agent output declares blocked or missing evidence".to_string());
    }
    None
}

pub fn output_reports_failed_verification(body: &str) -> Option<String> {
    output_reports_failed_verification_with_options(body, true, true)
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
        "paper_trading_ready`: `false`",
        "\"paper_trading_ready\":false",
        "\"paper_trading_ready\": false",
        "status: **not ready",
        "not ready for paper trading",
    ];
    PHRASES.iter().any(|phrase| lower.contains(phrase))
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
    !(text_has_any(lower, FILE_EVIDENCE_FIELDS)
        && text_has_any(lower, VERIFICATION_EVIDENCE_FIELDS)
        && text_has_any(lower, COMPLETION_EVIDENCE_FIELDS))
}

fn reports_accepted_status(body: &str, lower: &str) -> bool {
    if serde_json::from_str::<Value>(body)
        .ok()
        .is_some_and(|value| value_has_accepted_status(&value))
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
                return json_has_non_empty_field(value, FILE_EVIDENCE_FIELDS)
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

fn text_has_any(lower: &str, fields: &[&str]) -> bool {
    fields.iter().any(|field| lower.contains(field))
}

fn json_has_field(value: &Value, names: &[&str]) -> bool {
    match value {
        Value::Object(fields) => fields.iter().any(|(field, value)| {
            names.iter().any(|name| field == name)
                || (!is_non_final_attempts_field(field) && json_has_field(value, names))
        }),
        Value::Array(values) => values.iter().any(|value| json_has_field(value, names)),
        _ => false,
    }
}

fn json_has_non_empty_field(value: &Value, names: &[&str]) -> bool {
    match value {
        Value::Object(fields) => fields.iter().any(|(field, value)| {
            (names.iter().any(|name| field == name) && json_value_has_evidence(value))
                || (!is_non_final_attempts_field(field) && json_has_non_empty_field(value, names))
        }),
        Value::Array(values) => values
            .iter()
            .any(|value| json_has_non_empty_field(value, names)),
        _ => false,
    }
}

fn json_value_has_evidence(value: &Value) -> bool {
    match value {
        Value::Array(values) => !values.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
        Value::String(value) => !value.trim().is_empty(),
        Value::Bool(_) | Value::Number(_) => true,
        Value::Null => false,
    }
}

fn next_line_has_blocking_status(lines: &[&str], idx: usize) -> bool {
    lines
        .iter()
        .skip(idx + 1)
        .map(|line| normalized_status_value(line.trim()))
        .find(|line| !line.is_empty())
        .is_some_and(|line| starts_with_blocking_status_value(&line))
}

fn json_reports_failed_verification(body: &str) -> Option<bool> {
    serde_json::from_str::<Value>(body)
        .ok()
        .map(|value| value_reports_failed_verification(&value))
}

fn value_reports_failed_verification(value: &Value) -> bool {
    match value {
        Value::Object(fields) => fields.iter().any(|(field, value)| {
            if is_non_final_attempts_field(field) {
                return false;
            }
            (is_verification_status_field(field) && value_is_blocking_status(value))
                || value_reports_failed_verification(value)
        }),
        Value::Array(values) => values.iter().any(value_reports_failed_verification),
        _ => false,
    }
}

fn value_has_accepted_status(value: &Value) -> bool {
    match value {
        Value::Object(fields) => {
            object_has_accepted_status(fields)
                || fields
                    .iter()
                    .filter(|(field, _)| !is_non_final_attempts_field(field))
                    .any(|(_, value)| value_has_accepted_status(value))
        }
        Value::Array(values) => values.iter().any(value_has_accepted_status),
        _ => false,
    }
}

fn object_has_accepted_status(fields: &serde_json::Map<String, Value>) -> bool {
    fields.get("status").is_some_and(|value| {
        value
            .as_str()
            .map(normalized_status_value)
            .is_some_and(|status| status == "accepted")
    })
}

fn is_non_final_attempts_field(field: &str) -> bool {
    matches!(
        field,
        "non_final_attempts" | "prior_attempts" | "previous_attempts" | "retry_history"
    )
}

fn is_verification_status_field(field: &str) -> bool {
    matches!(
        field,
        "status"
            | "verification_status"
            | "overall_status"
            | "overall_result"
            | "result"
            | "final_status"
    )
}

fn value_is_blocking_status(value: &Value) -> bool {
    value
        .as_str()
        .map(normalized_status_value)
        .is_some_and(|value| starts_with_blocking_status_value(&value))
}

fn contains_blocking_structured_field(lower: &str) -> bool {
    const FIELDS: &[&str] = &[
        "status",
        "verification_status",
        "overall_status",
        "overall_result",
        "result",
        "final_status",
    ];
    const VALUES: &[&str] = &[
        "failed",
        "failure",
        "failed_timeout",
        "failed_validation_timeout",
        "completed_with_timeout",
        "completed_with_timeouts",
        "timed_out",
        "timeout",
        "command_timeout",
        "completed_with_residual_failure",
        "partial_pass_with_timeout_residual",
        "unverifiable",
        "not_verified",
        "not_fully_verified",
        "blocked",
        "partial_success",
        "partial_failure",
    ];
    FIELDS.iter().any(|field| {
        VALUES.iter().any(|value| {
            lower.contains(&format!("\"{field}\":\"{value}\""))
                || lower.contains(&format!("\"{field}\": \"{value}\""))
        })
    })
}

fn starts_with_blocking_status_field(normalized: &str) -> bool {
    status_fields().iter().any(|field| {
        [":", "="].iter().any(|sep| {
            let prefix = format!("{field}{sep}");
            normalized
                .strip_prefix(&prefix)
                .or_else(|| normalized.strip_prefix(&format!("-{prefix}")))
                .is_some_and(starts_with_blocking_status_value)
        })
    })
}

fn is_verification_status_heading(normalized: &str) -> bool {
    status_fields().contains(&normalized)
}

fn status_fields() -> &'static [&'static str] {
    &[
        "status",
        "verificationstatus",
        "overallstatus",
        "overallresult",
        "result",
        "finalstatus",
    ]
}

fn starts_with_blocking_status_value(value: &str) -> bool {
    if value.contains("timeout") || value.contains("timedout") {
        return true;
    }
    [
        "failed",
        "failure",
        "failedtimeout",
        "failedvalidationtimeout",
        "completedwithtimeout",
        "completedwithtimeouts",
        "timedout",
        "timeout",
        "commandtimeout",
        "completedwithresidualfailure",
        "partialpasswithtimeoutresidual",
        "unverifiable",
        "notverified",
        "notfullyverified",
        "blocked",
        "partialsuccess",
        "partialfailure",
    ]
    .iter()
    .any(|blocking| value.starts_with(blocking))
}

fn normalized_status_value(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| !matches!(ch, '*' | '_' | '`' | ' ' | '-' | '"' | '\'' | ','))
        .collect()
}

fn normalized_status_line(line: &str) -> String {
    line.trim()
        .trim_start_matches('#')
        .chars()
        .filter(|ch| !matches!(ch, '*' | '_' | '`' | ' ' | '"' | '\'' | ','))
        .collect::<String>()
}

#[cfg(test)]
#[path = "context_output_tests.rs"]
mod tests;
