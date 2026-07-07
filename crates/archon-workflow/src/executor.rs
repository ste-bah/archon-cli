use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::context;
use crate::control::{RunControl, RunControlDecision};
use crate::error::{WorkflowError, WorkflowResult};
use crate::events::{WorkflowEventKind, WorkflowEventLog};
use crate::exec_state::{
    finish_run, mark_finished, mark_started, pause_for_human_gate, report, stalled_running_reason,
};
use crate::executor_output::deterministic_stage_output;
use crate::learning::record_workflow_learning;
use crate::persistence;
use crate::policy::WorkflowPolicy;
use crate::reducers::ReducerRegistry;
use crate::run::{RunStatus, StageStatus, WorkflowRun};
use crate::runner::WorkflowStageRunner;
use crate::spec::{ReducerKind, StageKind, StageSpec, WorkflowSpec};
use crate::stage::{ordered_stages, stage_ready};
use crate::store::WorkflowStore;
use crate::{WorkflowBundle, WorkflowBundleOrigin, WorkflowHarness};

#[path = "executor_live.rs"]
mod executor_live;
#[path = "executor_live_retry.rs"]
mod executor_live_retry;
#[path = "executor_stage.rs"]
mod executor_stage;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionReport {
    pub run_id: String,
    pub completed: usize,
    pub blocked: usize,
    pub forced_accepted: usize,
    pub failed: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone)]
pub struct WorkflowExecutor {
    store: WorkflowStore,
    policy: WorkflowPolicy,
    reducers: ReducerRegistry,
}

impl WorkflowExecutor {
    pub fn new(store: WorkflowStore, policy: WorkflowPolicy) -> Self {
        Self {
            store,
            policy,
            reducers: ReducerRegistry,
        }
    }

    pub fn start(&self, spec: WorkflowSpec) -> WorkflowResult<WorkflowRun> {
        self.start_with_bundle_origin(spec, WorkflowBundleOrigin::GeneratedHarness)
    }

    pub fn start_imported_spec(&self, spec: WorkflowSpec) -> WorkflowResult<WorkflowRun> {
        self.start_with_bundle_origin(spec, WorkflowBundleOrigin::ImportedSpecWrapper)
    }

    pub fn start_with_harness(
        &self,
        spec: WorkflowSpec,
        harness_source: &str,
        origin: WorkflowBundleOrigin,
    ) -> WorkflowResult<WorkflowRun> {
        // Legacy runs execute spec stages; the harness is bundled for record
        // only. Source-text validation belongs to the QuickJS dry-run at the
        // V2 boundary, not to a second parser here.
        self.start_validated(spec, origin, Some(harness_source))
    }

    fn start_with_bundle_origin(
        &self,
        spec: WorkflowSpec,
        origin: WorkflowBundleOrigin,
    ) -> WorkflowResult<WorkflowRun> {
        self.start_validated(spec, origin, None)
    }

    fn start_validated(
        &self,
        mut spec: WorkflowSpec,
        origin: WorkflowBundleOrigin,
        harness_source: Option<&str>,
    ) -> WorkflowResult<WorkflowRun> {
        crate::required_artifact_contract::ensure_final_required_artifacts(&mut spec);
        if crate::required_artifact_heal::self_heal_requested(&spec) {
            crate::required_artifact_heal::ensure_required_artifact_self_heal(&mut spec);
        }
        spec.validate()?;
        self.policy.validate_spec(&spec)?;
        let mut run = self.store.create_run(spec)?;
        if let Some(harness_source) = harness_source {
            WorkflowBundle::create_for_run(&self.store, &run, harness_source, origin.clone())?;
        } else {
            match origin {
                WorkflowBundleOrigin::GeneratedHarness => {
                    let harness = WorkflowHarness::from_spec(&run.spec);
                    WorkflowBundle::create_for_run(
                        &self.store,
                        &run,
                        &harness.source,
                        WorkflowBundleOrigin::GeneratedHarness,
                    )?;
                }
                WorkflowBundleOrigin::ImportedSpecWrapper => {
                    WorkflowBundle::synthesize_for_imported_spec(&self.store, &run)?;
                }
                WorkflowBundleOrigin::SavedCommand => {
                    let harness = WorkflowHarness::from_spec(&run.spec);
                    WorkflowBundle::create_for_run(
                        &self.store,
                        &run,
                        &harness.source,
                        WorkflowBundleOrigin::SavedCommand,
                    )?;
                }
            }
        }
        run.status = RunStatus::Running;
        run.mark_updated();
        self.store.save_state(&run)?;
        let log = WorkflowEventLog::new(self.store.clone());
        log.emit(
            &run.id,
            1,
            WorkflowEventKind::Started,
            json!({"name": run.spec.name, "task": run.spec.task}),
        )?;
        Ok(run)
    }

    pub fn execute(&self, mut run: WorkflowRun) -> WorkflowResult<ExecutionReport> {
        self.ensure_bundle_ready(&run)?;
        let ordered = ordered_stages(&run.spec)?;
        let mut seq = self.store.next_event_seq(&run.id)?;
        let mut progressed = false;
        for stage in ordered {
            if !self.control_checkpoint(&mut run, &mut seq, "before_stage")? {
                break;
            }
            if !stage_ready(&run, &stage) {
                continue;
            }
            progressed = true;
            self.run_stage(&mut run, &stage, &mut seq)?;
            if !self.control_checkpoint(&mut run, &mut seq, "after_stage")? {
                break;
            }
            if !matches!(run.status, RunStatus::Running) {
                break;
            }
        }
        self.fail_if_stalled(&mut run, progressed, &mut seq)?;
        finish_run(&mut run);
        record_workflow_learning(&self.store, &run, &mut seq)?;
        self.store.save_state_preserving_control(&run)?;
        Ok(report(&run))
    }

    pub async fn execute_with_runner(
        &self,
        mut run: WorkflowRun,
        runner: &dyn WorkflowStageRunner,
    ) -> WorkflowResult<ExecutionReport> {
        self.ensure_bundle_ready(&run)?;
        let ordered = ordered_stages(&run.spec)?;
        let mut seq = self.store.next_event_seq(&run.id)?;
        let mut progressed = false;
        for stage in ordered {
            if !self.control_checkpoint(&mut run, &mut seq, "before_stage")? {
                break;
            }
            if !stage_ready(&run, &stage) {
                continue;
            }
            progressed = true;
            self.run_stage_with_runner(&mut run, &stage, &mut seq, runner)
                .await?;
            if !self.control_checkpoint(&mut run, &mut seq, "after_stage")? {
                break;
            }
            if !matches!(run.status, RunStatus::Running) {
                break;
            }
        }
        self.fail_if_stalled(&mut run, progressed, &mut seq)?;
        finish_run(&mut run);
        record_workflow_learning(&self.store, &run, &mut seq)?;
        self.store.save_state_preserving_control(&run)?;
        Ok(report(&run))
    }

    fn ensure_bundle_ready(&self, run: &WorkflowRun) -> WorkflowResult<()> {
        let run_dir = self.store.run_dir(&run.id);
        if !run_dir.join(crate::bundle::HARNESS_FILE).exists()
            || !run_dir.join(crate::bundle::COMPILED_SPEC_FILE).exists()
        {
            WorkflowBundle::synthesize_for_imported_spec(&self.store, run)?;
            return WorkflowBundle::verify(&self.store, &run.id).map(|_| ());
        }
        match WorkflowBundle::verify(&self.store, &run.id) {
            Ok(_) => Ok(()),
            Err(WorkflowError::Io { .. }) => {
                WorkflowBundle::synthesize_for_imported_spec(&self.store, run)?;
                WorkflowBundle::verify(&self.store, &run.id).map(|_| ())
            }
            Err(err) => Err(err),
        }
    }

    fn control_checkpoint(
        &self,
        run: &mut WorkflowRun,
        seq: &mut u64,
        checkpoint: &str,
    ) -> WorkflowResult<bool> {
        match RunControl::new(self.store.clone(), run.id.clone()).checkpoint(run)? {
            RunControlDecision::Continue => Ok(true),
            RunControlDecision::Paused { generation } => {
                WorkflowEventLog::new(self.store.clone()).emit(
                    &run.id,
                    *seq,
                    WorkflowEventKind::Paused,
                    json!({
                        "action": "control_checkpoint",
                        "checkpoint": checkpoint,
                        "generation": generation,
                    }),
                )?;
                *seq += 1;
                self.store.save_state_preserving_control(run)?;
                Ok(false)
            }
            RunControlDecision::Cancelled { generation } => {
                WorkflowEventLog::new(self.store.clone()).emit(
                    &run.id,
                    *seq,
                    WorkflowEventKind::Cancelled,
                    json!({
                        "action": "control_checkpoint",
                        "checkpoint": checkpoint,
                        "generation": generation,
                    }),
                )?;
                *seq += 1;
                self.store.save_state_preserving_control(run)?;
                Ok(false)
            }
        }
    }

    fn fail_if_stalled(
        &self,
        run: &mut WorkflowRun,
        progressed: bool,
        seq: &mut u64,
    ) -> WorkflowResult<()> {
        if progressed {
            return Ok(());
        }
        let Some(reason) = stalled_running_reason(run) else {
            return Ok(());
        };
        run.status = RunStatus::Failed;
        run.mark_updated();
        WorkflowEventLog::new(self.store.clone()).emit(
            &run.id,
            *seq,
            WorkflowEventKind::StageFailed,
            json!({
                "stage": "scheduler",
                "error_class": "no_runnable_stage",
                "reason": reason.clone(),
            }),
        )?;
        *seq += 1;
        self.store.save_state_preserving_control(run)?;
        Err(WorkflowError::StageFailed(reason))
    }
}
