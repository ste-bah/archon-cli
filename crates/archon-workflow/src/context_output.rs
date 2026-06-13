use serde_json::Value;

pub fn output_reports_blocked(body: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    let rejection = reports_reject_verdict(&lower);
    let blocked = reports_explicit_blocked_status(&lower);
    let empty_findings = lower.contains("findings: []") || lower.contains("\"findings\":[]");
    let audit_impossible = lower.contains("cannot audit")
        || lower.contains("could not audit")
        || lower.contains("can't audit")
        || lower.contains("unable to audit")
        || lower.contains("missing required evidence")
        || lower.contains("missing source evidence")
        || lower.contains("source evidence is missing")
        || lower.contains("source files are missing")
        || lower.contains("source files or upstream artifacts are absent")
        || lower.contains("no source evidence")
        || lower.contains("no source files")
        || lower.contains("no file content")
        || lower.contains("insufficient context")
        || lower.contains("no tool execution results are available")
        || lower.contains("cannot truthfully report pass/fail")
        || lower.contains("without executing the commands");
    if rejection {
        return Some("agent output declares reject/do-not-sign-off verdict".to_string());
    }
    if blocked || (empty_findings && audit_impossible) {
        return Some("agent output declares blocked or missing evidence".to_string());
    }
    None
}

pub fn output_reports_failed_verification(body: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    (json_reports_failed_verification(body)
        .unwrap_or_else(|| reports_failed_verification_status(&lower)))
    .then(|| "agent output declares failed or unverifiable verification status".to_string())
}

fn reports_reject_verdict(lower: &str) -> bool {
    if lower.contains("\"verdict\":\"reject\"")
        || lower.contains("\"verdict\": \"reject\"")
        || lower.contains("\"status\":\"rejected\"")
        || lower.contains("\"status\": \"rejected\"")
    {
        return true;
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

fn reports_explicit_blocked_status(lower: &str) -> bool {
    if lower.contains("\"status\":\"blocked\"") || lower.contains("\"status\": \"blocked\"") {
        return true;
    }
    let lines: Vec<_> = lower.lines().collect();
    lines.iter().enumerate().any(|(idx, line)| {
        let normalized = normalized_status_line(line);
        normalized.starts_with("status:blocked")
            || normalized.starts_with("-status:blocked")
            || normalized.starts_with("status=blocked")
            || normalized.contains("status:blocked")
            || normalized.contains("status=blocked")
            || (normalized == "status" && next_line_is_blocked(&lines, idx))
    })
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
    const FIELDS: &[&str] = &[
        "status",
        "verificationstatus",
        "overallstatus",
        "overallresult",
        "result",
        "finalstatus",
    ];
    FIELDS.iter().any(|field| {
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
    matches!(
        normalized,
        "status"
            | "verificationstatus"
            | "overallstatus"
            | "overallresult"
            | "result"
            | "finalstatus"
    )
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
