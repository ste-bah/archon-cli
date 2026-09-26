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

/// The in-scope gaps `record` carries. A gap the host flagged as naming only
/// undeclared paths (Issue-81) carries the severity it had before in its
/// text; one flagged before that marker existed has none to read, and is
/// listed by [`flagged_of`] instead.
pub fn residuals_of(record: &WorkflowV2CallRecord, root: Option<&Path>) -> Vec<Residual> {
    let unit = remediation_contract(&record.call)
        .map(unit_task_ids)
        .unwrap_or_default();
    let judged = super::super::remediation_escalation::judged_commit(&record.result);
    gaps_of(record)
        .into_iter()
        .filter_map(|(id, description, severity)| {
            let severity = if id.starts_with(UNOWNED_PATH_GAP_PREFIX) {
                ResidualSeverity::parse(Some(flagged_severity(&description)?))?
            } else {
                ResidualSeverity::parse(severity.as_deref())?
            };
            Some(Residual {
                recorded_by: record.call.id.clone(),
                severity,
                files: root.map_or_else(Vec::new, |root| {
                    named_files_at(&description, root, judged.as_deref())
                }),
                id,
                description,
                unit_tasks: unit.clone(),
                recorded_summary: record.result.summary.clone(),
            })
        })
        .collect()
}

/// The severity a flagged gap had before the host replaced it.
pub(super) fn flagged_severity(description: &str) -> Option<&str> {
    let (_, tail) = description.rsplit_once(FLAGGED_SEVERITY_MARKER)?;
    tail.strip_suffix(']').map(str::trim)
}

/// Labels of the gaps the host flagged as naming only unowned paths
/// (Issue-81): their severity was replaced by `review`, so no rule can
/// weigh them; the final gate lists them.
pub fn flagged_of(record: &WorkflowV2CallRecord) -> Vec<String> {
    gaps_of(record)
        .into_iter()
        .filter(|(id, description, _)| {
            id.starts_with(UNOWNED_PATH_GAP_PREFIX) && flagged_severity(description).is_none()
        })
        .map(|(id, _, _)| format!("`{id}` (recorded by `{}`)", record.call.id))
        .collect()
}
