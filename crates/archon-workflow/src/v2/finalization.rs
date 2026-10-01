//! Provider-neutral terminal finalization and run-end observer contracts.
//!
//! The host persists these records; this module owns only the closed state
//! machine. The observer itself stays observe-only (it writes no status), and
//! an omitted launch snapshot remains the legacy-silent representation.
//!
//! ACC-A9: the observation runs BEFORE the terminal commit. A pending
//! observation is completed or failed on the uncommitted record, and a failed
//! one may be reopened in place ([`FinalizationRecordV1::reopen_before_commit`])
//! while the finalizer re-enters acceptance and re-decides the outcome
//! ([`FinalizationRecordV1::restate`]); the committed outcome is the one the
//! last observation left. A record committed with a pending observation
//! (written by an older binary) may still finish it after the commit.
//!
//! The authored (v3) lifecycle adds its own, separate rule: its acceptance
//! stage's final round is recorded on the finalization record as
//! [`AuthoredAcceptanceGateV1`], and a record whose gate carries a failing
//! check cannot carry a completing terminal status. The R2 observer contract
//! above is untouched by it — the gate is a distinct field the R2 lifecycle
//! never sets.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::error::{WorkflowError, WorkflowResult};
use crate::run::RunStatus;
use crate::v2::{WorkflowRunKind, WorkflowV2Status};

pub const FINALIZATION_RECORD_SCHEMA_VERSION: u32 = 1;
pub const RUN_END_OBSERVER_SNAPSHOT_SCHEMA_VERSION: u32 = 1;
pub const RUN_END_OBSERVER_EXPECTED_ARTIFACT_PATHS: [&str; 5] = [
    "acceptance-contract.json",
    "acceptance-contract.lock",
    "task-skeleton.json",
    "task-skeleton.lock",
    "acceptance-pin.json",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableAcceptanceIdentityV1 {
    pub freeze_event_id: String,
    pub acceptance_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skeleton_digest: Option<String>,
}

/// Non-blocking launch-time evidence that opts a run into end-of-run checking.
///
/// Absence—not an explicit legacy value—is the compatibility representation.
/// Once present, later deletion cannot turn the run back into legacy-silent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunEndAcceptanceObserverSnapshotV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_execution: Option<serde_json::Value>,
    pub schema_version: u32,
    pub canonical_task_root_identity: String,
    pub expected_artifact_paths: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portable_acceptance_identity: Option<PortableAcceptanceIdentityV1>,
    /// `task_set_lineage::LINEAGE_RECORDING_V1` when the launching binary
    /// records lineage for every sanctioned republish; absent on a snapshot
    /// taken before lineage recording.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_recording: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObserverAuthority {
    ObserveOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunEndObserverOutcomeV1 {
    pub authority: ObserverAuthority,
    pub evaluated_floor_count: usize,
    pub policy_finding_count: usize,
    pub operational_deferral_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RunEndObserverStateV1 {
    Pending,
    Completed { outcome: RunEndObserverOutcomeV1 },
    Failed { reason: String },
}

/// What the authored run's final acceptance round recorded (Obs-32).
///
/// Written by the authored lifecycle alone, from the last record under
/// `v2/acceptance/`. `failing_check_ids` non-empty means the terminal status
/// is `NeedsReview`, never `Completed`; `with_acceptance_gate` refuses the
/// other combination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoredAcceptanceGateV1 {
    pub final_round: u32,
    pub attempt: u32,
    pub record_path: String,
    pub contract_present: bool,
    #[serde(default)]
    pub failing_check_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unowned_failing_check_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operational_errors: Vec<String>,
}

impl AuthoredAcceptanceGateV1 {
    /// A failing check, an unevaluable round, or no contract at all: an
    /// authored run never completes on checks it did not run (A8).
    pub fn blocks_completion(&self) -> bool {
        !self.failing_check_ids.is_empty()
            || !self.operational_errors.is_empty()
            || !self.contract_present
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalizationRecordV1 {
    pub schema_version: u32,
    pub run_kind: WorkflowRunKind,
    pub terminal_status: RunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_v2_status: Option<WorkflowV2Status>,
    pub terminal_state_committed: bool,
    pub terminal_event_committed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observer_snapshot: Option<RunEndAcceptanceObserverSnapshotV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observer_state: Option<RunEndObserverStateV1>,
    /// Authored lifecycle only; absent for every other run kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance_gate: Option<AuthoredAcceptanceGateV1>,
    /// Reasons of earlier run-end observations that failed and were reopened.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prior_observer_failures: Vec<String>,
}

impl FinalizationRecordV1 {
    pub fn new(
        run_kind: WorkflowRunKind,
        terminal_status: WorkflowV2Status,
        observer_snapshot: Option<RunEndAcceptanceObserverSnapshotV1>,
    ) -> Self {
        let terminal_run_status = run_status_from_v2(terminal_status);
        let eligible = observer_eligible(run_kind, &terminal_run_status, Some(terminal_status))
            && observer_snapshot.is_some();
        Self {
            schema_version: FINALIZATION_RECORD_SCHEMA_VERSION,
            run_kind,
            terminal_status: terminal_run_status,
            terminal_v2_status: Some(terminal_status),
            terminal_state_committed: true,
            terminal_event_committed: false,
            observer_snapshot: eligible.then_some(observer_snapshot).flatten(),
            observer_state: eligible.then_some(RunEndObserverStateV1::Pending),
            acceptance_gate: None,
            prior_observer_failures: Vec::new(),
        }
    }

    /// Attach the authored run's acceptance gate. A gate that blocks
    /// completion is incompatible with a completing terminal status: the
    /// authored lifecycle downgrades the summary before it gets here, and this
    /// refuses the record if it did not.
    pub fn with_acceptance_gate(mut self, gate: AuthoredAcceptanceGateV1) -> WorkflowResult<Self> {
        if self.run_kind != WorkflowRunKind::AuthoredTaskWorkflow {
            return Err(WorkflowError::StateCorrupt(
                "acceptance gate applies to authored task workflows only".to_string(),
            ));
        }
        if gate.blocks_completion() && self.is_completing() {
            return Err(WorkflowError::StateCorrupt(format!(
                "authored run cannot finalize as complete while acceptance round {} blocks it: {}",
                gate.final_round,
                if gate.contract_present {
                    gate.failing_check_ids
                        .iter()
                        .chain(&gate.operational_errors)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                } else {
                    "no acceptance contract was run".to_string()
                }
            )));
        }
        self.acceptance_gate = Some(gate);
        Ok(self)
    }

    pub fn for_run_status(run_kind: WorkflowRunKind, terminal_status: RunStatus) -> Self {
        Self {
            schema_version: FINALIZATION_RECORD_SCHEMA_VERSION,
            run_kind,
            terminal_status,
            terminal_v2_status: None,
            terminal_state_committed: true,
            terminal_event_committed: false,
            observer_snapshot: None,
            observer_state: None,
            acceptance_gate: None,
            prior_observer_failures: Vec::new(),
        }
    }

    /// Whether the outcome this record committed finished the run.
    ///
    /// A completing run refuses resume, so its committed outcome is final and
    /// nothing may later contradict it. Every other outcome is resumable, so a
    /// later execution attempt may legitimately record a different one.
    pub fn is_completing(&self) -> bool {
        matches!(
            self.terminal_v2_status,
            Some(WorkflowV2Status::Accepted | WorkflowV2Status::Noop)
        ) || self.terminal_status == RunStatus::Completed
    }

    pub fn mark_terminal_event_committed(&mut self) {
        self.terminal_event_committed = true;
    }

    pub fn complete_observer(&mut self, outcome: RunEndObserverOutcomeV1) -> WorkflowResult<()> {
        self.require_pending()?;
        if outcome.authority != ObserverAuthority::ObserveOnly {
            return Err(WorkflowError::StateCorrupt(
                "R2 run-end observer authority must remain observe_only".to_string(),
            ));
        }
        self.observer_state = Some(RunEndObserverStateV1::Completed { outcome });
        Ok(())
    }

    pub fn fail_observer(&mut self, reason: String) -> WorkflowResult<()> {
        self.require_pending()?;
        self.observer_state = Some(RunEndObserverStateV1::Failed { reason });
        Ok(())
    }

    /// Keep a failed pre-commit observation's `reason` in
    /// `prior_observer_failures` and leave the observation pending, so it runs
    /// again after acceptance is re-entered. Refused once the terminal event
    /// is committed: a committed outcome is reopened only by
    /// [`Self::reopen_observer`], which never changes it.
    pub fn reopen_before_commit(&mut self, reason: String) -> WorkflowResult<()> {
        if self.terminal_event_committed || self.observer_snapshot.is_none() {
            return Err(WorkflowError::StateCorrupt(
                "a pre-commit run-end observation reopens only on an uncommitted record with a launch snapshot"
                    .to_string(),
            ));
        }
        self.require_pending()?;
        self.prior_observer_failures.push(reason);
        Ok(())
    }

    /// Re-decide the uncommitted outcome after acceptance was re-entered: the
    /// terminal status and the acceptance gate the re-entered round recorded.
    /// The same rule as [`Self::with_acceptance_gate`] holds: a blocking gate
    /// never sits beside a completing status. The pending observation stays
    /// armed, so the status must remain observer-eligible.
    pub fn restate(
        &mut self,
        terminal_status: WorkflowV2Status,
        acceptance_gate: Option<AuthoredAcceptanceGateV1>,
    ) -> WorkflowResult<()> {
        let terminal_run_status = run_status_from_v2(terminal_status);
        if self.terminal_event_committed
            || !observer_eligible(self.run_kind, &terminal_run_status, Some(terminal_status))
        {
            return Err(WorkflowError::StateCorrupt(format!(
                "an outcome is restated only before the terminal commit and to an observer-eligible status; {terminal_status:?} committed={}",
                self.terminal_event_committed
            )));
        }
        let mut restated = self.clone();
        restated.terminal_status = terminal_run_status;
        restated.terminal_v2_status = Some(terminal_status);
        restated.acceptance_gate = None;
        if let Some(gate) = acceptance_gate {
            restated = restated.with_acceptance_gate(gate)?;
        }
        *self = restated;
        Ok(())
    }

    /// Return a failed run-end observation to pending so it may run again.
    /// The failure is kept in `prior_observer_failures` and returned. A
    /// pending observation a reopen left behind (the re-observation was
    /// interrupted) may be reopened too; the finalizer's own first pending
    /// observation may not.
    pub fn reopen_observer(&mut self) -> WorkflowResult<String> {
        if !self.terminal_event_committed || self.observer_snapshot.is_none() {
            return Err(WorkflowError::StateCorrupt(
                "run-end observer reopen requires a committed terminal event and a launch snapshot"
                    .to_string(),
            ));
        }
        let reason = match self.observer_state.clone() {
            Some(RunEndObserverStateV1::Failed { reason }) => reason,
            Some(RunEndObserverStateV1::Pending) if !self.prior_observer_failures.is_empty() => {
                "an earlier re-observation was interrupted before it recorded an outcome"
                    .to_string()
            }
            state => {
                return Err(WorkflowError::StateCorrupt(format!(
                    "run-end observer can be reopened only after it failed; its state is {state:?}"
                )));
            }
        };
        self.prior_observer_failures.push(reason.clone());
        self.observer_state = Some(RunEndObserverStateV1::Pending);
        Ok(reason)
    }

    /// An observation finishes from the durable pending state, on either
    /// side of the terminal commit (see the module doc).
    fn require_pending(&self) -> WorkflowResult<()> {
        if self.observer_state != Some(RunEndObserverStateV1::Pending) {
            return Err(WorkflowError::StateCorrupt(
                "run-end observer transition requires durable observer_pending state".to_string(),
            ));
        }
        Ok(())
    }
}

pub fn observer_eligible(
    run_kind: WorkflowRunKind,
    status: &RunStatus,
    v2_status: Option<WorkflowV2Status>,
) -> bool {
    run_kind == WorkflowRunKind::AuthoredTaskWorkflow
        && matches!(status, RunStatus::Completed | RunStatus::NeedsReview)
        && matches!(
            v2_status,
            Some(
                WorkflowV2Status::Accepted | WorkflowV2Status::Noop | WorkflowV2Status::NeedsReview
            )
        )
}

fn run_status_from_v2(status: WorkflowV2Status) -> RunStatus {
    match status {
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop => RunStatus::Completed,
        WorkflowV2Status::NeedsReview => RunStatus::NeedsReview,
        WorkflowV2Status::Blocked => RunStatus::Blocked,
        WorkflowV2Status::Failed => RunStatus::Failed,
        WorkflowV2Status::Cancelled => RunStatus::Cancelled,
        WorkflowV2Status::Pending => RunStatus::Planned,
        WorkflowV2Status::Running => RunStatus::Running,
    }
}
