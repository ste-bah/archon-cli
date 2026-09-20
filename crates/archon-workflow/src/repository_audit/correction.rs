//! Explicit semantic reclassification, authorized only by a pending reassessment.
use super::{AuditReport, RequiredAction, contract::validate_path, ledger::Reassessment};
use crate::WorkflowV2AgentError;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Correction {
    pub declared_path: String,
    pub snapshot: String,
    pub action_id: String,
    pub reason: String,
    pub evidence_paths: Vec<String>,
}
fn invalid(message: impl std::fmt::Display) -> WorkflowV2AgentError {
    WorkflowV2AgentError::InvalidResult(format!("audit correction: {message}"))
}
pub(crate) fn validate(
    raw: Option<&serde_json::Value>,
    report: &AuditReport,
    pending: &[Reassessment],
    root: Option<&Path>,
) -> Result<Vec<Correction>, WorkflowV2AgentError> {
    let corrections: Vec<Correction> =
        serde_json::from_value(raw.cloned().unwrap_or_else(|| serde_json::json!([])))
            .map_err(invalid)?;
    let mut seen = BTreeSet::new();
    for correction in &corrections {
        if correction.snapshot != report.snapshot
            || !seen.insert(&correction.declared_path)
            || !pending.iter().any(|r| {
                r.action_id == correction.action_id
                    && r.snapshot == correction.snapshot
                    && r.declared_path == correction.declared_path
            })
            || !report.records.iter().any(|r| {
                r.declared_path == correction.declared_path
                    && r.required_action == RequiredAction::None
            })
            || correction.reason.trim().is_empty()
            || correction.reason.len() > 2048
            || correction.evidence_paths.is_empty()
        {
            return Err(invalid(
                "correction must uniquely identify this disputed finding, positive assessment and evidence",
            ));
        }
        let root = root
            .ok_or_else(|| invalid("sealed evidence root is missing"))?
            .canonicalize()
            .map_err(invalid)?;
        for path in &correction.evidence_paths {
            validate_path(path)?;
            if !root
                .join(path)
                .canonicalize()
                .map_err(invalid)?
                .starts_with(&root)
            {
                return Err(invalid("evidence escapes sealed repository"));
            }
        }
    }
    Ok(corrections)
}
