use std::collections::BTreeSet;
use serde::{Deserialize, Serialize};
use crate::{WorkflowV2AgentError, WorkflowV2AgentRequest, WorkflowV2Result, WorkflowV2Status};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuditContract {
    pub schema_version: u32,
    pub snapshot: String,
    pub declared_paths: Vec<String>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Verdict { ExistsAsDeclared, Absent, ExistsElsewhere, Unreachable }
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RequiredAction { None, Deliver, WireOrMigrate }
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuditRecord {
    pub declared_path: String,
    pub verdict: Verdict,
    pub equivalents: Vec<String>,
    pub required_action: RequiredAction,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuditReport {
    pub schema_version: u32,
    pub snapshot: String,
    pub records: Vec<AuditRecord>,
}

fn invalid(reason: impl std::fmt::Display) -> WorkflowV2AgentError {
    WorkflowV2AgentError::InvalidResult(format!("repository audit: {reason}"))
}

/// Lexical validation only. Filesystem confinement is checked in the sealed view.
pub fn validate_path(path: &str) -> Result<(), WorkflowV2AgentError> {
    if path.is_empty() || path.len() > 4096 || path.contains(['\\', '\0', ':'])
        || path.chars().any(char::is_control)
        || path.split('/').any(|part| part.is_empty() || part == "." || part == ".." || part == ".git") {
        return Err(invalid(format!("declared_path/equivalent must be a normalized root-relative path: {path:?}")));
    }
    Ok(())
}
impl AuditContract {
    pub fn validate_report(&self, report: &AuditReport) -> Result<(), WorkflowV2AgentError> {
        if self.schema_version != 1 || report.schema_version != 1 || self.snapshot.is_empty()
            || self.snapshot != report.snapshot {
            return Err(invalid("schema_version or snapshot does not match the host contract"));
        }
        let mut declared = BTreeSet::new();
        for path in &self.declared_paths { validate_path(path)?; declared.insert(path.as_str()); }
        let mut seen = BTreeSet::new();
        for record in &report.records {
            validate_path(&record.declared_path)?;
            if !declared.contains(record.declared_path.as_str()) || !seen.insert(record.declared_path.as_str()) {
                return Err(invalid(format!("unexpected or duplicate declared_path: {}", record.declared_path)));
            }
            if record.reason.trim().is_empty() || record.reason.len() > 2048 {
                return Err(invalid(format!("reason must contain 1..=2048 bytes for {}",record.declared_path)));
            }
            let mut equivalents = BTreeSet::new();
            for path in &record.equivalents {
                validate_path(path)?;
                if path == &record.declared_path || !equivalents.insert(path) {
                    return Err(invalid("equivalents must be distinct other paths"));
                }
            }
            let valid = match record.verdict {
                Verdict::ExistsAsDeclared => record.required_action == RequiredAction::None && equivalents.is_empty(),
                Verdict::Absent => record.required_action == RequiredAction::Deliver && equivalents.is_empty(),
                Verdict::ExistsElsewhere => record.required_action == RequiredAction::WireOrMigrate && !equivalents.is_empty(),
                Verdict::Unreachable => record.required_action == RequiredAction::WireOrMigrate,
            };
            if !valid { return Err(invalid(format!("incompatible verdict, equivalents and required_action for {}",record.declared_path))); }
        }
        if seen != declared {
            return Err(invalid(format!("missing declared_path records: {:?}", declared.difference(&seen).collect::<Vec<_>>())));
        }
        Ok(())
    }
}

/// This marker requests validation only, not audit authority. Only the host
/// assessor may persist a validated report as the run's authoritative snapshot.
pub(crate) fn enforce(request: &WorkflowV2AgentRequest, result: &WorkflowV2Result) -> Result<(), WorkflowV2AgentError> {
    let Some(raw) = request.call.options.extra.get("repository_audit_contract") else { return Ok(()); };
    if result.status != WorkflowV2Status::Accepted { return Ok(()); }
    let contract: AuditContract = serde_json::from_value(raw.clone()).map_err(invalid)?;
    let raw = result.data.get("repository_audit").ok_or_else(|| invalid("data.repository_audit is absent"))?;
    let report: AuditReport = serde_path_to_error::deserialize(raw.clone()).map_err(|e| {
        // Unknown-only objects otherwise name only the unexpected key, leaving
        // the author without the mandatory record fields needed for repair.
        invalid(format!("{e}; records require declared_path, verdict, equivalents, required_action, reason"))
    })?;
    contract.validate_report(&report)
}
