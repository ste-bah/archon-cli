use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::acceptance::{self, AcceptanceOutcome};
use crate::context;
use crate::error::{WorkflowError, WorkflowResult};
use crate::events::{WorkflowEventKind, WorkflowEventLog};
use crate::exec_state::{finish_run, mark_finished, mark_started, pause_for_human_gate, report};
use crate::executor_fanout;
use crate::executor_output::{deterministic_stage_output, ensure_output_usable};
use crate::learning::record_workflow_learning;
use crate::persistence;
use crate::policy::WorkflowPolicy;
use crate::reducers::ReducerRegistry;
use crate::request::stage_request;
use crate::run::{RunStatus, StageStatus, WorkflowRun};
use crate::runner::{StageRunOutput, WorkflowStageRunner};
use crate::spec::{ReducerKind, StageKind, StageSpec, WorkflowSpec};
use crate::stage::{ordered_stages, stage_ready};
use crate::store::WorkflowStore;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionReport {
    pub run_id: String,
    pub completed: usize,
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
        spec.validate()?;
        self.policy.validate_spec(&spec)?;
        let mut run = self.store.create_run(spec)?;
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
        let ordered = ordered_stages(&run.spec)?;
        let mut seq = self.store.next_event_seq(&run.id)?;
        for stage in ordered {
            if !stage_ready(&run, &stage) {
                continue;
            }
            self.run_stage(&mut run, &stage, &mut seq)?;
            if !matches!(run.status, RunStatus::Running) {
                break;
            }
        }
        finish_run(&mut run);
        record_workflow_learning(&self.store, &run, &mut seq)?;
        self.store.save_state(&run)?;
        Ok(report(&run))
    }

    pub async fn execute_with_runner(
        &self,
        mut run: WorkflowRun,
        runner: &dyn WorkflowStageRunner,
    ) -> WorkflowResult<ExecutionReport> {
        let ordered = ordered_stages(&run.spec)?;
        let mut seq = self.store.next_event_seq(&run.id)?;
        for stage in ordered {
            if !stage_ready(&run, &stage) {
                continue;
            }
            self.run_stage_with_runner(&mut run, &stage, &mut seq, runner)
                .await?;
            if !matches!(run.status, RunStatus::Running) {
                break;
            }
        }
        finish_run(&mut run);
        record_workflow_learning(&self.store, &run, &mut seq)?;
        self.store.save_state(&run)?;
        Ok(report(&run))
    }

    fn run_stage(
        &self,
        run: &mut WorkflowRun,
        stage: &StageSpec,
        seq: &mut u64,
    ) -> WorkflowResult<()> {
        let log = WorkflowEventLog::new(self.store.clone());
        mark_started(run, stage)?;
        self.store.save_state(run)?;
        log.emit(
            &run.id,
            *seq,
            WorkflowEventKind::StageStarted,
            json!({"stage": stage.id, "kind": format!("{:?}", stage.kind)}),
        )?;
        *seq += 1;
        if stage.kind == StageKind::HumanGate {
            pause_for_human_gate(run, stage, seq, &log)?;
            return self.store.save_state(run);
        }
        let result = match stage.kind {
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
            Err(err) => {
                self.record_runner_failure_if_missing(run, stage, &err)?;
                mark_finished(run, stage, StageStatus::Failed, Some(err.to_string()))?;
                log.emit(
                    &run.id,
                    *seq,
                    WorkflowEventKind::StageFailed,
                    json!({"stage": stage.id, "error_class": "stage_failed"}),
                )?;
            }
        }
        *seq += 1;
        self.store.save_state(run)
    }

    async fn run_stage_with_runner(
        &self,
        run: &mut WorkflowRun,
        stage: &StageSpec,
        seq: &mut u64,
        runner: &dyn WorkflowStageRunner,
    ) -> WorkflowResult<()> {
        let log = WorkflowEventLog::new(self.store.clone());
        mark_started(run, stage)?;
        self.store.save_state(run)?;
        log.emit(
            &run.id,
            *seq,
            WorkflowEventKind::StageStarted,
            json!({"stage": stage.id, "kind": format!("{:?}", stage.kind), "agent": stage.agent}),
        )?;
        *seq += 1;
        if stage.kind == StageKind::HumanGate {
            pause_for_human_gate(run, stage, seq, &log)?;
            return self.store.save_state(run);
        }
        let result = self.dispatch_stage_with_runner(run, stage, runner).await;
        match result {
            Ok(output) => {
                log.emit(
                    &run.id,
                    *seq,
                    WorkflowEventKind::StageCompleted,
                    json!({
                        "stage": stage.id,
                        "status": "accepted",
                        "provider": output.provider_id,
                        "model": output.resolved_model,
                    }),
                )?;
                mark_finished(run, stage, StageStatus::Accepted, None)?;
            }
            Err(err) => {
                self.record_runner_failure_if_missing(run, stage, &err)?;
                mark_finished(run, stage, StageStatus::Failed, Some(err.to_string()))?;
                log.emit(
                    &run.id,
                    *seq,
                    WorkflowEventKind::StageFailed,
                    json!({"stage": stage.id, "error_class": "stage_failed"}),
                )?;
            }
        }
        *seq += 1;
        self.store.save_state(run)
    }

    async fn dispatch_stage_with_runner(
        &self,
        run: &mut WorkflowRun,
        stage: &StageSpec,
        runner: &dyn WorkflowStageRunner,
    ) -> WorkflowResult<StageRunOutput> {
        let output = match stage.kind {
            StageKind::Agent | StageKind::Tool => {
                let request = stage_request(&self.store, run, stage)?;
                persistence::record_prompt(&self.store, &request)?;
                let output = runner.run_stage(request).await?;
                ensure_output_usable(&output.body)?;
                let artifact = persistence::write_attached_stage_artifact(
                    &self.store,
                    run,
                    stage,
                    &stage.id,
                    &output.extension,
                    output.body.clone(),
                    true,
                )?;
                persistence::record_agent_output(
                    &self.store,
                    &run.id,
                    &stage.id,
                    &stage.id,
                    Some(&output),
                    Some(&artifact),
                    true,
                    None,
                )?;
                output
            }
            StageKind::Implementation => {
                self.run_implementation_with_runner(run, stage, runner)
                    .await?
            }
            StageKind::Fanout => {
                executor_fanout::run_fanout_with_runner(
                    &self.store,
                    &self.policy,
                    run,
                    stage,
                    runner,
                )
                .await?
            }
            StageKind::Reduce => {
                self.run_reduce(run, stage)?;
                StageRunOutput::markdown(format!("Reducer stage `{}` complete.", stage.id))
            }
            StageKind::QualityGate => {
                self.run_quality_gate(run, stage)?;
                StageRunOutput::markdown(format!("Quality gate `{}` passed.", stage.id))
            }
            StageKind::Condition | StageKind::Checkpoint => {
                self.write_condition_artifact(run, stage)?;
                StageRunOutput::markdown(format!("Checkpoint stage `{}` complete.", stage.id))
            }
            StageKind::HumanGate => unreachable!("human gates pause before dispatch"),
        };
        Ok(output)
    }

    /// Execute a write-capable implementation stage with acceptance binding
    /// (Phase B). The stage is accepted only when every `expected_target_files`
    /// entry was mutated AND the `verify_command` (if any) exits 0; otherwise
    /// the artifact is recorded as not-accepted and the stage fails.
    async fn run_implementation_with_runner(
        &self,
        run: &mut WorkflowRun,
        stage: &StageSpec,
        runner: &dyn WorkflowStageRunner,
    ) -> WorkflowResult<StageRunOutput> {
        let root = run.root.clone();
        let before = acceptance::snapshot_targets(&root, &stage.expected_target_files);
        let request = stage_request(&self.store, run, stage)?;
        persistence::record_prompt(&self.store, &request)?;
        let output = runner.run_stage(request).await?;
        ensure_output_usable(&output.body)?;
        let after = acceptance::snapshot_targets(&root, &stage.expected_target_files);
        let outcome = acceptance::evaluate(
            &root,
            &stage.expected_target_files,
            &before,
            &after,
            stage.verify_command.as_deref(),
        );
        let accepted = outcome.is_accepted();
        let artifact = persistence::write_attached_stage_artifact(
            &self.store,
            run,
            stage,
            &stage.id,
            &output.extension,
            output.body.clone(),
            accepted,
        )?;
        if let AcceptanceOutcome::Rejected(reason) = outcome {
            persistence::record_agent_output(
                &self.store,
                &run.id,
                &stage.id,
                &stage.id,
                Some(&output),
                Some(&artifact),
                false,
                Some(&reason),
            )?;
            return Err(WorkflowError::StageFailed(format!(
                "implementation stage '{}' rejected: {reason}",
                stage.id
            )));
        }
        persistence::record_agent_output(
            &self.store,
            &run.id,
            &stage.id,
            &stage.id,
            Some(&output),
            Some(&artifact),
            true,
            None,
        )?;
        Ok(output)
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
        let output = self.reducers.reduce(reducer, &inputs)?;
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
        let artifact = persistence::write_attached_stage_artifact(
            &self.store,
            run,
            stage,
            &stage.id,
            "json",
            json!({"status": "accepted", "checked_dependencies": stage.depends_on}).to_string(),
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
