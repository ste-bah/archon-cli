use chrono::Utc;
use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::context;
use crate::error::{WorkflowError, WorkflowResult};
use crate::events::{WorkflowEventKind, WorkflowEventLog};
use crate::fanout::FanoutItem;
use crate::learning::WorkflowLearningSink;
use crate::policy::WorkflowPolicy;
use crate::reducers::ReducerRegistry;
use crate::run::{RunStatus, StageStatus, WorkflowRun};
use crate::runner::{StageRunOutput, StageRunRequest, WorkflowStageRunner};
use crate::spec::{ProviderTier, StageKind, StageSpec, WorkflowSpec};
use crate::stage::{ordered_stages, source_input_hash, stage_ready};
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
        }
        finish_run(&mut run);
        self.record_learning(&run, &mut seq)?;
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
        }
        finish_run(&mut run);
        self.record_learning(&run, &mut seq)?;
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
        let result = match stage.kind {
            StageKind::Agent | StageKind::Tool | StageKind::Checkpoint | StageKind::HumanGate => {
                self.write_stage_artifact(run, stage, "md", deterministic_stage_output(stage))
            }
            StageKind::Fanout => self.run_fanout(run, stage),
            StageKind::Reduce => self.run_reduce(run, stage),
            StageKind::QualityGate => self.run_quality_gate(run, stage),
            StageKind::Condition => self.write_stage_artifact(run, stage, "json", "{}".to_string()),
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
            StageKind::Agent | StageKind::Tool | StageKind::HumanGate => {
                let output = runner
                    .run_stage(stage_request(&self.store, run, stage)?)
                    .await?;
                ensure_output_usable(&output.body)?;
                self.write_stage_artifact(run, stage, &output.extension, output.body.clone())?;
                output
            }
            StageKind::Fanout => self.run_fanout_with_runner(run, stage, runner).await?,
            StageKind::Reduce => {
                self.run_reduce(run, stage)?;
                StageRunOutput::markdown(format!("Reducer stage `{}` complete.", stage.id))
            }
            StageKind::QualityGate => {
                self.run_quality_gate(run, stage)?;
                StageRunOutput::markdown(format!("Quality gate `{}` passed.", stage.id))
            }
            StageKind::Condition | StageKind::Checkpoint => {
                self.write_stage_artifact(run, stage, "json", "{}".to_string())?;
                StageRunOutput::markdown(format!("Checkpoint stage `{}` complete.", stage.id))
            }
        };
        Ok(output)
    }

    fn run_fanout(&self, run: &mut WorkflowRun, stage: &StageSpec) -> WorkflowResult<()> {
        let items = context::fanout_items(&self.store, run, stage)?;
        let body = format!(
            "# Fan-out Summary\n\nStage `{}` processed {} item(s).\n",
            stage.id,
            items.len()
        );
        self.write_stage_artifact(run, stage, "md", body)
    }

    async fn run_fanout_with_runner(
        &self,
        run: &mut WorkflowRun,
        stage: &StageSpec,
        runner: &dyn WorkflowStageRunner,
    ) -> WorkflowResult<StageRunOutput> {
        let items = context::fanout_items(&self.store, run, stage)?;
        let width = fanout_width(run, &self.policy);
        let mut completed = 0usize;
        for batch in items.chunks(width) {
            let mut jobs = Vec::new();
            for item in batch {
                let request = fanout_item_request(&self.store, run, stage, item)?;
                let item_id = item.id.clone();
                jobs.push(async move { (item_id, runner.run_stage(request).await) });
            }
            for (item_id, result) in join_all(jobs).await {
                let output = result?;
                ensure_output_usable(&output.body)
                    .map_err(|err| WorkflowError::StageFailed(format!("{item_id}: {err}")))?;
                self.write_stage_artifact(run, stage, &output.extension, output.body)?;
                completed += 1;
            }
        }
        Ok(StageRunOutput::markdown(format!(
            "Fan-out stage `{}` completed {} item(s) with width {}.",
            stage.id, completed, width
        )))
    }

    fn run_reduce(&self, run: &mut WorkflowRun, stage: &StageSpec) -> WorkflowResult<()> {
        let reducer = stage
            .reducer
            .ok_or_else(|| WorkflowError::MissingReducer(stage.id.clone()))?;
        let inputs = context::reducer_inputs(&self.store, run, stage)?;
        let output = self.reducers.reduce(reducer, &inputs)?;
        self.write_stage_artifact(run, stage, "md", output.body)
    }

    fn run_quality_gate(&self, run: &mut WorkflowRun, stage: &StageSpec) -> WorkflowResult<()> {
        if let Some(reason) = context::quality_gate_failure(&self.store, run, stage)? {
            return Err(WorkflowError::StageFailed(reason));
        }
        self.write_stage_artifact(
            run,
            stage,
            "json",
            json!({"status": "accepted", "checked_dependencies": stage.depends_on}).to_string(),
        )
    }

    fn write_stage_artifact(
        &self,
        run: &mut WorkflowRun,
        stage: &StageSpec,
        extension: &str,
        body: String,
    ) -> WorkflowResult<()> {
        let input_hash = source_input_hash(stage);
        let mut artifact = self.store.write_artifact(
            &run.id,
            &stage.id,
            &input_hash,
            extension,
            body.as_bytes(),
        )?;
        artifact.accepted = true;
        let state = run
            .stage_mut(&stage.id)
            .ok_or_else(|| WorkflowError::SpecInvalid(format!("missing stage {}", stage.id)))?;
        state.artifacts.push(artifact);
        Ok(())
    }

    fn record_learning(&self, run: &WorkflowRun, seq: &mut u64) -> WorkflowResult<()> {
        if !matches!(run.status, RunStatus::Completed | RunStatus::Failed) {
            return Ok(());
        }
        let log = WorkflowEventLog::new(self.store.clone());
        match WorkflowLearningSink::new(self.store.clone()).record(run) {
            Ok(summary) => {
                log.emit(
                    &run.id,
                    *seq,
                    WorkflowEventKind::LearningRecorded,
                    json!({
                        "records": summary.records,
                        "durable_records": summary.durable_records,
                        "adapter_records": summary.adapter_records,
                        "proposal_records": summary.proposal_records,
                    }),
                )?;
                *seq += 1;
            }
            Err(err) => {
                log.emit(
                    &run.id,
                    *seq,
                    WorkflowEventKind::LearningRecorded,
                    json!({"status": "degraded", "error_class": err.to_string()}),
                )?;
                *seq += 1;
            }
        }
        Ok(())
    }
}

fn finish_run(run: &mut WorkflowRun) {
    if run.stages.values().all(|stage| stage.is_terminal()) {
        run.status = if run.stages.values().any(|s| s.status == StageStatus::Failed) {
            RunStatus::Failed
        } else {
            RunStatus::Completed
        };
    }
    run.mark_updated();
}

fn stage_request(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> WorkflowResult<StageRunRequest> {
    let attempt = run
        .stages
        .get(&stage.id)
        .map(|state| state.attempt)
        .unwrap_or(1);
    Ok(StageRunRequest {
        run_id: run.id.clone(),
        stage_id: stage.id.clone(),
        stage_kind: stage.kind,
        agent: stage.agent.clone(),
        task: run.spec.task.clone(),
        attempt,
        provider_tier: stage.provider_tier.unwrap_or(ProviderTier::Planner),
        depends_on: stage.depends_on.clone(),
        input: context::stage_input(store, run, stage)?,
    })
}

fn fanout_item_request(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
    item: &FanoutItem,
) -> WorkflowResult<StageRunRequest> {
    let mut request = stage_request(store, run, stage)?;
    request.stage_id = item.id.clone();
    request.input = context::fanout_input(store, run, stage, item)?;
    Ok(request)
}

fn fanout_width(run: &WorkflowRun, policy: &WorkflowPolicy) -> usize {
    run.spec.max_parallelism.min(policy.max_parallelism).max(1) as usize
}

fn deterministic_stage_output(stage: &StageSpec) -> String {
    format!(
        "# Stage {}\n\nKind: `{:?}`\nAgent: `{}`\n",
        stage.id,
        stage.kind,
        stage.agent.as_deref().unwrap_or("none")
    )
}

fn ensure_output_usable(body: &str) -> WorkflowResult<()> {
    if let Some(reason) = context::output_reports_blocked(body) {
        return Err(WorkflowError::StageFailed(reason));
    }
    Ok(())
}

fn mark_started(run: &mut WorkflowRun, stage: &StageSpec) -> WorkflowResult<()> {
    let state = run
        .stage_mut(&stage.id)
        .ok_or_else(|| WorkflowError::SpecInvalid(format!("missing stage {}", stage.id)))?;
    state.status = StageStatus::Running;
    state.attempt += 1;
    state.started_at = Some(Utc::now());
    state.error = None;
    run.mark_updated();
    Ok(())
}

fn mark_finished(
    run: &mut WorkflowRun,
    stage: &StageSpec,
    status: StageStatus,
    error: Option<String>,
) -> WorkflowResult<()> {
    let state = run
        .stage_mut(&stage.id)
        .ok_or_else(|| WorkflowError::SpecInvalid(format!("missing stage {}", stage.id)))?;
    state.status = status;
    state.completed_at = Some(Utc::now());
    state.error = error;
    run.mark_updated();
    Ok(())
}

fn report(run: &WorkflowRun) -> ExecutionReport {
    ExecutionReport {
        run_id: run.id.clone(),
        completed: run
            .stages
            .values()
            .filter(|s| {
                matches!(
                    s.status,
                    StageStatus::Accepted | StageStatus::ForcedAccepted
                )
            })
            .count(),
        failed: run
            .stages
            .values()
            .filter(|s| s.status == StageStatus::Failed)
            .count(),
        skipped: run
            .stages
            .values()
            .filter(|s| s.status == StageStatus::Skipped)
            .count(),
    }
}
