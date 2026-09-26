//! Historical judgments are immutable; proposals never count as resolutions.
use super::{AuditContract, AuditReport, RequiredAction};
use crate::{WorkflowError, WorkflowResult};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

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
    /// Declared paths the repository ignores: project artifacts the audit
    /// reclaimed from its jurisdiction (Issue-26, `super::ignored`).
    #[serde(default)]
    pub ignored_paths: BTreeSet<String>,
    /// Absence obligations the owning task's accepted verification discharged
    /// (Issue-104, `super::discharge`).
    #[serde(default)]
    pub discharges: Vec<super::discharge::Discharge>,
    /// Declared paths whose declarers' verified landings disagree
    /// (Issue-112, `super::contest`): unresolved, never re-delivered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contests: Vec<super::contest::Contest>,
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
        contract
            .validate_report(&report)
            .map_err(|e| WorkflowError::ArtifactInvalid(e.to_string()))?;
        if let Some(previous) = self.history.last() {
            for record in &previous.records {
                if !contract.declared_paths.contains(&record.declared_path)
                    && !self.ignored_paths.contains(&record.declared_path)
                {
                    return Err(WorkflowError::ArtifactInvalid(
                        "audit refresh cannot silently drop a declared path".into(),
                    ));
                }
            }
        }
        for record in &report.records {
            if record.required_action != RequiredAction::None {
                let obligation = self
                    .obligations
                    .entry(record.declared_path.clone())
                    .or_insert_with(|| Obligation {
                        opened_snapshot: report.snapshot.clone(),
                        proposed_explanation: None,
                        applied_commit: None,
                        resolved_snapshot: None,
                    });
                // A later negative judgment reopens the obligation. Old applied
                // evidence cannot resolve a new defect without another apply.
                if obligation.resolved_snapshot.is_some() {
                    obligation.applied_commit = None;
                }
                obligation.resolved_snapshot = None;
            } else if let Some(obligation) = self.obligations.get_mut(&record.declared_path) {
                obligation.resolved_snapshot = (obligation.applied_commit.is_some()
                    || (obligation.resolved_snapshot.is_some()
                        && self
                            .corrections
                            .iter()
                            .any(|c| c.declared_path == record.declared_path)))
                .then(|| report.snapshot.clone());
            }
        }
        self.history.push(report);
        Ok(())
    }
    pub fn unresolved(&self, snapshot: &str) -> WorkflowResult<Vec<String>> {
        if !self
            .history
            .last()
            .is_some_and(|report| report.snapshot == snapshot)
        {
            return Err(WorkflowError::ArtifactInvalid(
                "audit assessment missing or stale for requested snapshot".into(),
            ));
        }
        let mut open: BTreeSet<String> = self
            .obligations
            .iter()
            .filter(|(path, obligation)| {
                obligation.resolved_snapshot.as_deref() != Some(snapshot)
                    && !self.is_waived(path, snapshot)
                    && !self.is_discharged(path, snapshot)
            })
            .map(|(path, _)| path.clone())
            .collect();
        // Issue-112: a contested path is open whether or not the audit asks
        // for delivery; only the declarers' agreement or a waiver closes it.
        open.extend(
            self.contested(snapshot)
                .into_iter()
                .filter(|contest| !self.is_waived(&contest.declared_path, snapshot))
                .map(|contest| contest.declared_path.clone()),
        );
        Ok(open.into_iter().collect())
    }
    /// The contests recorded for `snapshot`.
    pub fn contested(&self, snapshot: &str) -> Vec<&super::contest::Contest> {
        self.contests
            .iter()
            .filter(|contest| contest.snapshot == snapshot)
            .collect()
    }
    /// Whether `path` is contested in `snapshot`.
    pub fn is_contested(&self, path: &str, snapshot: &str) -> bool {
        self.contested(snapshot)
            .iter()
            .any(|contest| contest.declared_path == path)
    }
    /// [`Self::unresolved`], each contested path named with its contest.
    pub fn describe_unresolved(&self, snapshot: &str) -> WorkflowResult<Vec<String>> {
        Ok(self
            .unresolved(snapshot)?
            .into_iter()
            .map(|path| {
                self.contested(snapshot)
                    .into_iter()
                    .find(|contest| contest.declared_path == path)
                    .map_or(path, |contest| contest.describe())
            })
            .collect())
    }
    /// Record contests for the latest report, replacing any for its snapshot.
    pub fn record_contests(&mut self, contests: Vec<super::contest::Contest>) {
        let Some(snapshot) = self.history.last().map(|report| report.snapshot.clone()) else {
            return;
        };
        self.contests.retain(|c| c.snapshot != snapshot);
        self.contests
            .extend(contests.into_iter().filter(|c| c.snapshot == snapshot));
    }
    pub fn is_waived(&self, path: &str, snapshot: &str) -> bool {
        self.waivers.iter().any(|w| {
            w.declared_path == path
                && w.snapshot == snapshot
                && w.assessment_count == self.history.len()
        })
    }
    /// A discharge binds the snapshot it was recorded for and the path's
    /// absent verdict in that snapshot's report: a path that exists again is
    /// judged as it exists.
    pub fn is_discharged(&self, path: &str, snapshot: &str) -> bool {
        let absent = self.history.last().is_some_and(|report| {
            report.snapshot == snapshot
                && report.records.iter().any(|record| {
                    record.declared_path == path && record.verdict == super::Verdict::Absent
                })
        });
        absent
            && self
                .discharges
                .iter()
                .any(|d| d.declared_path == path && d.snapshot == snapshot)
    }
    /// Record discharges for the latest report, replacing any for its snapshot.
    pub fn record_discharges(&mut self, discharges: Vec<super::discharge::Discharge>) {
        let Some(snapshot) = self.history.last().map(|report| report.snapshot.clone()) else {
            return;
        };
        self.discharges.retain(|d| d.snapshot != snapshot);
        self.discharges
            .extend(discharges.into_iter().filter(|d| d.snapshot == snapshot));
    }
    pub fn pending_reassessments(&self, snapshot: &str) -> Vec<Reassessment> {
        self.reassessments
            .iter()
            .filter(|r| !r.attempted && r.snapshot == snapshot)
            .cloned()
            .collect()
    }
    pub fn propose(&mut self, path: &str, explanation: String) {
        if let Some(obligation) = self.obligations.get_mut(path) {
            obligation.proposed_explanation = Some(explanation);
        }
    }
    /// Call only after checking the canonical apply record, not agent claims.
    pub fn record_applied(&mut self, path: &str, commit: String) {
        if !commit.is_empty()
            && let Some(obligation) = self.obligations.get_mut(path)
        {
            obligation.applied_commit = Some(commit);
            obligation.resolved_snapshot = None;
        }
    }
}
