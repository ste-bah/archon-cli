//! Provider-neutral terminal finalization and run-end observer contracts.
//!
//! The host persists these records; this module owns only the closed state
//! machine. Observer authority is fixed to observe-only in R2, and an omitted
//! launch snapshot remains the legacy-silent representation.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::error::{WorkflowError, WorkflowResult};
use crate::v2::{WorkflowRunKind, WorkflowV2Status};

pub const FINALIZATION_RECORD_SCHEMA_VERSION: u32 = 1;
pub const RUN_END_OBSERVER_SNAPSHOT_SCHEMA_VERSION: u32 = 1;

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
    pub schema_version: u32,
    pub canonical_task_root_identity: String,
    pub expected_artifact_paths: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portable_acceptance_identity: Option<PortableAcceptanceIdentityV1>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalizationRecordV1 {
    pub schema_version: u32,
    pub run_kind: WorkflowRunKind,
    pub terminal_status: WorkflowV2Status,
    pub terminal_state_committed: bool,
    pub terminal_event_committed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observer_snapshot: Option<RunEndAcceptanceObserverSnapshotV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observer_state: Option<RunEndObserverStateV1>,
}

impl FinalizationRecordV1 {
    pub fn new(
        run_kind: WorkflowRunKind,
        terminal_status: WorkflowV2Status,
        observer_snapshot: Option<RunEndAcceptanceObserverSnapshotV1>,
    ) -> Self {
        let eligible = observer_eligible(run_kind, terminal_status) && observer_snapshot.is_some();
        Self {
            schema_version: FINALIZATION_RECORD_SCHEMA_VERSION,
            run_kind,
            terminal_status,
            terminal_state_committed: true,
            terminal_event_committed: false,
            observer_snapshot: eligible.then_some(observer_snapshot).flatten(),
            observer_state: eligible.then_some(RunEndObserverStateV1::Pending),
        }
    }

    pub fn mark_terminal_event_committed(&mut self) {
        self.terminal_event_committed = true;
    }

    pub fn complete_observer(&mut self, outcome: RunEndObserverOutcomeV1) -> WorkflowResult<()> {
        self.require_pending_after_terminal_event()?;
        if outcome.authority != ObserverAuthority::ObserveOnly {
            return Err(WorkflowError::StateCorrupt(
                "R2 run-end observer authority must remain observe_only".to_string(),
            ));
        }
        self.observer_state = Some(RunEndObserverStateV1::Completed { outcome });
        Ok(())
    }

    pub fn fail_observer(&mut self, reason: String) -> WorkflowResult<()> {
        self.require_pending_after_terminal_event()?;
        self.observer_state = Some(RunEndObserverStateV1::Failed { reason });
        Ok(())
    }

    fn require_pending_after_terminal_event(&self) -> WorkflowResult<()> {
        if !self.terminal_event_committed {
            return Err(WorkflowError::StateCorrupt(
                "run-end observer cannot finish before the terminal event is committed".to_string(),
            ));
        }
        if self.observer_state != Some(RunEndObserverStateV1::Pending) {
            return Err(WorkflowError::StateCorrupt(
                "run-end observer transition requires durable observer_pending state".to_string(),
            ));
        }
        Ok(())
    }
}

pub fn observer_eligible(run_kind: WorkflowRunKind, status: WorkflowV2Status) -> bool {
    run_kind == WorkflowRunKind::AuthoredTaskWorkflow
        && matches!(
            status,
            WorkflowV2Status::Accepted | WorkflowV2Status::Noop | WorkflowV2Status::NeedsReview
        )
}
