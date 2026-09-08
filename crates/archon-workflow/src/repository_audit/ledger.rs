//! Historical judgments are immutable; proposals never count as resolutions.
use std::collections::BTreeMap;
use super::{AuditContract, AuditReport, RequiredAction};
use crate::{WorkflowError, WorkflowResult};
use serde::{Serialize, Deserialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Obligation {
    pub opened_snapshot: String,
    pub proposed_explanation: Option<String>,
    pub applied_commit: Option<String>,
    pub resolved_snapshot: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditLedger {
    pub history: Vec<AuditReport>,
    pub obligations: BTreeMap<String, Obligation>,
    #[serde(default)]
    pub waivers: Vec<Waiver>,
    #[serde(default)]
    pub reassessments: Vec<Reassessment>,
    #[serde(default)]
    pub corrections: Vec<super::correction::Correction>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Waiver {
    pub declared_path: String,
    pub snapshot: String,
    pub action_id: String,
    pub reason: String,
    pub assessment_count: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reassessment {
    pub declared_path: String,
    pub snapshot: String,
    pub action_id: String,
    pub reason: String,
    pub attempted: bool,
}
impl AuditLedger {
    /// Only a host-owned assessor invocation may call this after validating
    /// filesystem evidence and successful-apply provenance for this snapshot.
    pub fn accept(&mut self, contract: AuditContract, report: AuditReport) -> WorkflowResult<()> {
        contract.validate_report(&report).map_err(|e| WorkflowError::ArtifactInvalid(e.to_string()))?;
        if let Some(previous) = self.history.last() {
            for record in &previous.records {
                if !contract.declared_paths.contains(&record.declared_path) {
                    return Err(WorkflowError::ArtifactInvalid("audit refresh cannot silently drop a declared path".into()));
                }
            }
        }
        for record in &report.records {
            if record.required_action != RequiredAction::None {
                let obligation = self.obligations.entry(record.declared_path.clone()).or_insert_with(|| Obligation {
                    opened_snapshot: report.snapshot.clone(), proposed_explanation: None,
                    applied_commit: None, resolved_snapshot: None,
                });
                // A later negative judgment reopens the obligation. Old applied
                // evidence cannot resolve a new defect without another apply.
                if obligation.resolved_snapshot.is_some() { obligation.applied_commit = None; }
                obligation.resolved_snapshot = None;
            } else if let Some(obligation) = self.obligations.get_mut(&record.declared_path) {
                obligation.resolved_snapshot = (obligation.applied_commit.is_some()
                    || (obligation.resolved_snapshot.is_some() && self.corrections.iter().any(|c| c.declared_path == record.declared_path)))
                    .then(|| report.snapshot.clone());
            }
        }
        self.history.push(report);
        Ok(())
    }
    pub fn unresolved(&self, snapshot: &str) -> WorkflowResult<Vec<String>> {
        if !self.history.last().is_some_and(|report| report.snapshot == snapshot) {
            return Err(WorkflowError::ArtifactInvalid("audit assessment missing or stale for requested snapshot".into()));
        }
        Ok(self.obligations.iter().filter(|(path, obligation)| obligation.resolved_snapshot.as_deref() != Some(snapshot)
            && !self.is_waived(path, snapshot))
            .map(|(path, _)| path.clone()).collect())
    }
    pub fn is_waived(&self, path: &str, snapshot: &str) -> bool {
        self.waivers.iter().any(|w| w.declared_path == path && w.snapshot == snapshot
            && w.assessment_count == self.history.len())
    }
    pub fn pending_reassessments(&self, snapshot: &str) -> Vec<Reassessment> {
        self.reassessments.iter().filter(|r| !r.attempted && r.snapshot == snapshot).cloned().collect()
    }
    pub fn propose(&mut self, path: &str, explanation: String) {
        if let Some(obligation) = self.obligations.get_mut(path) {
            obligation.proposed_explanation = Some(explanation);
        }
    }
    /// Call only after checking the canonical apply record, not agent claims.
    pub fn record_applied(&mut self, path: &str, commit: String) {
        if !commit.is_empty() && let Some(obligation) = self.obligations.get_mut(path) {
            obligation.applied_commit = Some(commit);
            obligation.resolved_snapshot = None;
        }
    }
}
