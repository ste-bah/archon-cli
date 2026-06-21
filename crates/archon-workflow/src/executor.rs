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
use crate::{HarnessCompiler, WorkflowBundle, WorkflowBundleOrigin, WorkflowHarness};

#[path = "executor_live.rs"]
mod executor_live;
#[path = "executor_live_retry.rs"]
mod executor_live_retry;

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
        HarnessCompiler::default().validate(harness_source)?;
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

    fn run_stage(
        &self,
        run: &mut WorkflowRun,
        stage: &StageSpec,
        seq: &mut u64,
    ) -> WorkflowResult<()> {
        let log = WorkflowEventLog::new(self.store.clone());
        mark_started(run, stage)?;
        self.store.save_state_preserving_control(run)?;
        log.emit(
            &run.id,
            *seq,
            WorkflowEventKind::StageStarted,
            json!({"stage": stage.id, "kind": format!("{:?}", stage.kind)}),
        )?;
        *seq += 1;
        if stage.kind == StageKind::HumanGate {
            pause_for_human_gate(run, stage, seq, &log)?;
            return self.store.save_state_preserving_control(run);
        }
        let result = match stage.kind {
            StageKind::Tool
                if stage.tool.as_deref()
                    == Some(crate::required_artifacts::REQUIRED_ARTIFACT_INVENTORY_TOOL) =>
            {
                self.run_required_artifact_inventory(run, stage)
            }
            StageKind::Agent | StageKind::Tool | StageKind::Checkpoint => {
                persistence::write_attached_stage_artifact(
                    &self.store,
                    run,
                    stage,
                    &stage.id,
                    "md",
                    deterministic_stage_output(stage),
                    true,
                )
                .map(|_| ())
            }
            StageKind::Fanout => self.run_fanout(run, stage),
            StageKind::Reduce => self.run_reduce(run, stage),
            StageKind::QualityGate => self.run_quality_gate(run, stage),
            StageKind::Condition => self.write_condition_artifact(run, stage).map(|_| ()),
            StageKind::Implementation => Err(WorkflowError::StageFailed(format!(
                "implementation stage '{}' requires a live stage runner",
                stage.id
            ))),
            StageKind::HumanGate => unreachable!("human gates pause before dispatch"),
        };
        match result {
            Ok(()) => {
                mark_finished(run, stage, StageStatus::Accepted, None)?;
                log.emit(
                    &run.id,
                    *seq,
                    WorkflowEventKind::StageCompleted,
                    json!({"stage": stage.id, "status": "accepted"}),
                )?;
            }
            Err(WorkflowError::StageBlocked(reason)) => {
                mark_finished(run, stage, StageStatus::Blocked, Some(reason.clone()))?;
                log.emit(
                    &run.id,
                    *seq,
                    WorkflowEventKind::StageCompleted,
                    json!({
                        "stage": stage.id,
                        "status": "blocked",
                        "error_class": "stage_blocked",
                        "reason": reason,
                    }),
                )?;
            }
            Err(WorkflowError::ControlPaused(reason)) => {
                run.status = RunStatus::Paused;
                if let Some(state) = run.stage_mut(&stage.id) {
                    state.status = StageStatus::Paused;
                    state.error = Some(reason.clone());
                    state.completed_at = None;
                }
                log.emit(
                    &run.id,
                    *seq,
                    WorkflowEventKind::Paused,
                    json!({
                        "stage": stage.id,
                        "action": "control_pause",
                        "reason": reason,
                    }),
                )?;
            }
            Err(WorkflowError::ControlCancelled(reason)) => {
                run.status = RunStatus::Cancelled;
                mark_finished(run, stage, StageStatus::Cancelled, Some(reason.clone()))?;
                log.emit(
                    &run.id,
                    *seq,
                    WorkflowEventKind::Cancelled,
                    json!({
                        "stage": stage.id,
                        "action": "control_cancel",
                        "reason": reason,
                    }),
                )?;
            }
            Err(err) => {
                self.record_runner_failure_if_missing(run, stage, &err)?;
                mark_finished(run, stage, StageStatus::Failed, Some(err.to_string()))?;
                log.emit(
                    &run.id,
                    *seq,
                    WorkflowEventKind::StageFailed,
                    json!({
                        "stage": stage.id,
                        "error_class": "stage_failed",
                        "reason": err.to_string(),
                    }),
                )?;
            }
        }
        *seq += 1;
        self.store.save_state_preserving_control(run)
    }

    fn run_fanout(&self, run: &mut WorkflowRun, stage: &StageSpec) -> WorkflowResult<()> {
        let items = context::fanout_items(&self.store, run, stage)?;
        let body = format!(
            "# Fan-out Summary\n\nStage `{}` processed {} item(s).\n",
            stage.id,
            items.len()
        );
        persistence::write_attached_stage_artifact(
            &self.store,
            run,
            stage,
            &stage.id,
            "md",
            body,
            true,
        )
        .map(|_| ())
    }

    fn run_reduce(&self, run: &mut WorkflowRun, stage: &StageSpec) -> WorkflowResult<()> {
        let reducer = stage.reducer.unwrap_or(ReducerKind::EvidenceWeightedReport);
        let inputs = context::reducer_inputs(&self.store, run, stage)?;
        let mut output = crate::remediation_items::structured_items_output(stage, &inputs)
            .map(Ok)
            .unwrap_or_else(|| self.reducers.reduce(reducer, &inputs))?;
        if let Some(body) = crate::remediation_inventory::repair_empty_inventory_output(
            &self.store,
            run,
            stage,
            &output.body,
        )? {
            output.body = body;
            output.title = "Structured Remediation Items".to_string();
        }
        let artifact = persistence::write_attached_stage_artifact(
            &self.store,
            run,
            stage,
            &stage.id,
            "md",
            output.body.clone(),
            true,
        )?;
        persistence::record_reducer(
            &self.store,
            &run.id,
            stage,
            reducer,
            &inputs,
            &output,
            &artifact,
        )
    }

    pub(crate) fn run_required_artifact_inventory(
        &self,
        run: &mut WorkflowRun,
        stage: &StageSpec,
    ) -> WorkflowResult<()> {
        persistence::write_attached_stage_artifact(
            &self.store,
            run,
            stage,
            &stage.id,
            "json",
            crate::required_artifacts::inventory_body(&self.store, run, stage),
            true,
        )
        .map(|_| ())
    }

    fn run_quality_gate(&self, run: &mut WorkflowRun, stage: &StageSpec) -> WorkflowResult<()> {
        if let Some(reason) = context::quality_gate_failure(&self.store, run, stage)? {
            persistence::record_quality(
                &self.store,
                &run.id,
                stage,
                "failed",
                Some(&reason),
                None,
            )?;
            return Err(WorkflowError::StageFailed(reason));
        }
        let artifact_report =
            crate::required_artifacts::check_required_artifacts(&self.store, run, stage);
        if let Some(report) = artifact_report.as_ref()
            && let Some(reason) = report.failure_reason()
        {
            let artifact = persistence::write_attached_stage_artifact(
                &self.store,
                run,
                stage,
                &stage.id,
                "json",
                serde_json::to_string(report)?,
                false,
            )?;
            persistence::record_quality(
                &self.store,
                &run.id,
                stage,
                "failed",
                Some(&reason),
                Some(&artifact),
            )?;
            return Err(WorkflowError::StageFailed(reason));
        }
        let artifact = persistence::write_attached_stage_artifact(
            &self.store,
            run,
            stage,
            &stage.id,
            "json",
            json!({
                "status": "accepted",
                "checked_dependencies": stage.depends_on,
                "required_artifacts": artifact_report,
            })
            .to_string(),
            true,
        )?;
        persistence::record_quality(
            &self.store,
            &run.id,
            stage,
            "accepted",
            None,
            Some(&artifact),
        )
    }

    fn write_condition_artifact(
        &self,
        run: &mut WorkflowRun,
        stage: &StageSpec,
    ) -> WorkflowResult<crate::run::ArtifactRef> {
        persistence::write_attached_stage_artifact(
            &self.store,
            run,
            stage,
            &stage.id,
            "json",
            "{}".to_string(),
            true,
        )
    }

    fn record_runner_failure_if_missing(
        &self,
        run: &WorkflowRun,
        stage: &StageSpec,
        err: &WorkflowError,
    ) -> WorkflowResult<()> {
        if !matches!(
            stage.kind,
            StageKind::Agent | StageKind::Tool | StageKind::Implementation
        ) || persistence::agent_output_exists(&self.store, &run.id, &stage.id, &stage.id)
        {
            return Ok(());
        }
        persistence::record_agent_output(
            &self.store,
            &run.id,
            &stage.id,
            &stage.id,
            None,
            None,
            false,
            Some(&err.to_string()),
        )
    }
}
