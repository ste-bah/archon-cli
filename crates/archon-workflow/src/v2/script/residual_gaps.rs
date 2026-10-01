//! Issue-117: the gaps one accepted verifier's record carries, as the plan
//! and the gate read them.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;

use super::super::remediation_escalation::unit_task_ids;
use super::super::residual_paths::named_files_at;
use super::super::{WorkflowV2CallRecord, remediation_contract};
use super::{Residual, ResidualSeverity};
use crate::v2::verification::{FLAGGED_SEVERITY_MARKER, UNOWNED_PATH_GAP_PREFIX};

/// Every gap `record` carries: its own and each branch view's, de-duplicated.
pub(super) fn gaps_of(record: &WorkflowV2CallRecord) -> Vec<(String, String, Option<String>)> {
    let text = |gap: &Value, key: &str| {
        gap.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let mut gaps: Vec<(String, String, Option<String>)> = record
        .result
        .residual_gaps
        .iter()
        .map(|gap| {
            (
                gap.id.clone(),
                gap.description.clone(),
                gap.severity.clone(),
            )
        })
        .collect();
    for view in record
        .result
        .data
        .get("outcomes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for gap in view
            .pointer("/result/residual_gaps")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let severity = gap
                .get("severity")
                .and_then(Value::as_str)
                .map(str::to_string);
            gaps.push((text(gap, "id"), text(gap, "description"), severity));
        }
    }
    let mut seen = BTreeSet::new();
    gaps.retain(|(id, description, _)| seen.insert((id.clone(), description.clone())));
    gaps
}

/// Every gap `record` carries that is work (Batch O: no severity label
/// drops one; see [`ResidualSeverity::parse`]). A gap the host flagged as
/// naming only undeclared paths (Issue-81) is weighed at the severity it had
/// before, read from its text; one flagged before that marker existed has
/// none to read and is MEDIUM, planned like any other.
pub fn residuals_of(record: &WorkflowV2CallRecord, root: Option<&Path>) -> Vec<Residual> {
    let unit = remediation_contract(&record.call)
        .map(unit_task_ids)
        .unwrap_or_default();
    let judged = super::super::remediation_escalation::judged_commit(&record.result);
    let refused = !super::super::is_reusable_status(record.status);
    gaps_of(record)
        .into_iter()
        // Batch G2: the host's own environment and operational records are
        // resolved by the host (restore, re-run, operational error); they are
        // never work for a task.
        .filter(|(id, description, _)| !host_environment_gap(id, description))
        // Batch O: on a REFUSED verdict, the host's own bookkeeping gap (its
        // `review` label, never an agent's flagged one) states why it
        // refused; the refusal itself is what the host weighs and plans
        // again (the round stands, or its next pass retries it), so it is
        // not a second, separate piece of work.
        .filter(|(id, _, severity)| {
            !(refused
                && !id.starts_with(UNOWNED_PATH_GAP_PREFIX)
                && severity
                    .as_deref()
                    .is_some_and(|s| s.trim().eq_ignore_ascii_case("review")))
        })
        .map(|(id, description, severity)| {
            let severity = if id.starts_with(UNOWNED_PATH_GAP_PREFIX) {
                ResidualSeverity::parse(flagged_severity(&description))
            } else {
                ResidualSeverity::parse(severity.as_deref())
            };
            Residual {
                recorded_by: record.call.id.clone(),
                severity,
                files: root.map_or_else(Vec::new, |root| {
                    named_files_at(&description, root, judged.as_deref())
                }),
                id,
                description,
                unit_tasks: unit.clone(),
                recorded_summary: record.result.summary.clone(),
                host_built: false,
            }
        })
        .collect()
}

/// A gap that records the host's environment, not the work: a Batch G
/// environment violation, or one whose text BEGINS with the host's
/// operational error marker (`crate::error::HOST_OPERATIONAL_ERROR_MARKER`).
/// Both are host namespaces: the adapter drops an agent's gap in either.
pub fn host_environment_gap(id: &str, description: &str) -> bool {
    id.starts_with("environment-violation-")
        || crate::error::is_host_operational_text(description)
        // PLAN-11: a held check-source change is the acceptance judge's to
        // settle, and unreadable pins are the host's to restore.
        || id.starts_with(crate::check_source_requests::CHECK_SOURCE_HELD_GAP_PREFIX)
        || id.starts_with(crate::check_source_requests::CHECK_SOURCE_PINS_UNAVAILABLE_GAP_PREFIX)
}

/// The severity a flagged gap had before the host replaced it.
pub(super) fn flagged_severity(description: &str) -> Option<&str> {
    let (_, tail) = description.rsplit_once(FLAGGED_SEVERITY_MARKER)?;
    tail.strip_suffix(']').map(str::trim)
}

#[cfg(test)]
mod host_environment_tests {
    use super::*;
    use crate::{
        WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2ResidualGap, WorkflowV2Result,
    };

    /// Batch G2 (G2-3): the host's environment and operational records are
    /// never residual work for a task; a gap about the work still is.
    #[test]
    fn environment_and_operational_gaps_are_never_residual_work() {
        let gap = |id: &str, description: &str| WorkflowV2ResidualGap {
            id: id.into(),
            description: description.into(),
            severity: Some("high".into()),
        };
        let mut result = WorkflowV2Result::default();
        result.residual_gaps = vec![
            gap(
                "environment-violation-verify-x-1",
                "ENVIRONMENT VIOLATION: verify-x-1 changed the project's acceptance inputs",
            ),
            gap(
                "invalid_write_branch_output_x",
                &format!(
                    "{} the declared artifact verifier gave no verdict twice",
                    crate::error::HOST_OPERATIONAL_ERROR_MARKER
                ),
            ),
            gap("verifier-finding", "src/lib.rs still panics on empty input"),
        ];
        let record: WorkflowV2CallRecord = serde_json::from_value(serde_json::json!({
            "run_id": "wf", "attempt": 1, "schema_version": "1", "started_at": "t",
            "finished_at": "t", "input_hash": "i", "output_hash": "o", "status": "accepted",
            "call": WorkflowV2HostCall {
                id: "verify-x-1".into(),
                method: WorkflowV2HostMethod::Agent,
                write_mode: None,
                options: Default::default(),
            },
            "result": result,
        }))
        .expect("a call record");
        let ids: Vec<String> = residuals_of(&record, None)
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, ["verifier-finding"]);
    }
}
