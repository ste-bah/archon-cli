//! Validation for the decomposition log's transition and finding lines.

use std::collections::{BTreeMap, BTreeSet};

/// One subject key, never both, matching the progress-line rule.
pub(crate) fn subject_key(
    line: usize,
    fields: &BTreeMap<&str, &str>,
) -> Result<&'static str, String> {
    match (
        fields.contains_key("subject"),
        fields.contains_key("subject_digest"),
    ) {
        (true, false) => Ok("subject"),
        (false, true) => Ok("subject_digest"),
        _ => Err(format!(
            "decomposition log line {line} requires exactly one subject field"
        )),
    }
}

/// A transition that is not a call of its own: a phase boundary, a provider
/// request going in flight, findings being observed, or a terminal failure
/// reason.
pub(crate) fn validate_transition_fields(
    line: usize,
    fields: &BTreeMap<&str, &str>,
) -> Result<(), String> {
    if matches!(fields["transition"], "run_failed" | "run_needs_review") {
        let expected = BTreeSet::from(["transition", "field", "text"]);
        super::evidence::require_exact_log_keys(line, fields, &expected)?;
        return if matches!(
            fields["field"],
            "failed_call" | "failed_result" | "next_action"
        ) {
            Ok(())
        } else {
            Err(format!(
                "decomposition log line {line} names an unknown failure field"
            ))
        };
    }
    let expected = BTreeSet::from([
        "event_id",
        "phase",
        subject_key(line, fields)?,
        "transition",
    ]);
    super::evidence::require_exact_log_keys(line, fields, &expected)?;
    if fields["event_id"].parse::<u64>().is_err()
        || !matches!(
            fields["transition"],
            "decomposition_phase_started"
                | "model_call_in_flight"
                | "shadow_findings_observed"
                | "host_command_completed"
        )
    {
        return Err(format!(
            "decomposition log transition line {line} has invalid typed values"
        ));
    }
    Ok(())
}

/// One policy finding, carrying its exact text. The count alone is what made a
/// defective task set unreadable.
pub(crate) fn validate_finding_fields(
    line: usize,
    fields: &BTreeMap<&str, &str>,
) -> Result<(), String> {
    let expected = BTreeSet::from([
        "event_id",
        "phase",
        subject_key(line, fields)?,
        "finding",
        "text",
    ]);
    super::evidence::require_exact_log_keys(line, fields, &expected)?;
    let (index, total) = fields["finding"]
        .split_once('/')
        .ok_or_else(|| format!("decomposition log finding line {line} is not index/total"))?;
    let index: usize = index
        .parse()
        .map_err(|_| format!("decomposition log finding line {line} has a non-numeric index"))?;
    let total: usize = total
        .parse()
        .map_err(|_| format!("decomposition log finding line {line} has a non-numeric total"))?;
    if index == 0 || index > total || fields["event_id"].parse::<u64>().is_err() {
        return Err(format!(
            "decomposition log finding line {line} has invalid typed values"
        ));
    }
    Ok(())
}
