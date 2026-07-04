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
#[path = "../context_output_tests.rs"]
mod tests;
