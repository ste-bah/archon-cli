use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::acceptance::{self, AcceptanceOutcome};
use crate::context;
use crate::error::{WorkflowError, WorkflowResult};
use crate::events::{WorkflowEventKind, WorkflowEventLog};
use crate::exec_state::{finish_run, mark_finished, mark_started, pause_for_human_gate, report};
use crate::fanout;
use crate::learning::WorkflowLearningSink;
use crate::policy::WorkflowPolicy;
use crate::reducers::ReducerRegistry;
use crate::request::{fanout_item_request, stage_request};
use crate::run::{RunStatus, StageStatus, WorkflowRun};
use crate::runner::{StageRunOutput, WorkflowStageRunner};
use crate::spec::{ReducerKind, StageKind, StageSpec, WorkflowSpec};
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
            if !matches!(run.status, RunStatus::Running) {
                break;
            }
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
            if !matches!(run.status, RunStatus::Running) {
                break;
            }
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
        if stage.kind == StageKind::HumanGate {
            pause_for_human_gate(run, stage, seq, &log)?;
            return self.store.save_state(run);
        }
        let result = match stage.kind {
            StageKind::Agent | StageKind::Tool | StageKind::Checkpoint => {
                self.write_stage_artifact(run, stage, "md", deterministic_stage_output(stage))
            }
            StageKind::Fanout => self.run_fanout(run, stage),
            StageKind::Reduce => self.run_reduce(run, stage),
            StageKind::QualityGate => self.run_quality_gate(run, stage),
            StageKind::Condition => self.write_stage_artifact(run, stage, "json", "{}".to_string()),
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
                let output = runner
                    .run_stage(stage_request(&self.store, run, stage)?)
                    .await?;
                ensure_output_usable(&output.body)?;
                self.write_stage_artifact(run, stage, &output.extension, output.body.clone())?;
                output
            }
            StageKind::Implementation => {
                self.run_implementation_with_runner(run, stage, runner)
                    .await?
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
        let output = runner
            .run_stage(stage_request(&self.store, run, stage)?)
            .await?;
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
        self.write_stage_artifact_with_acceptance(
            run,
            stage,
            &output.extension,
            output.body.clone(),
            accepted,
        )?;
        if let AcceptanceOutcome::Rejected(reason) = outcome {
            return Err(WorkflowError::StageFailed(format!(
                "implementation stage '{}' rejected: {reason}",
                stage.id
            )));
        }
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
        let width = fanout::width(run, &self.policy);
        let max_agents = self.stage_max_agents(run, stage);
        if items.len() > max_agents {
            return Err(WorkflowError::PolicyDenied(format!(
                "fan-out item count {} exceeds max_agents {max_agents}",
                items.len()
            )));
        }
        let requests = items
            .iter()
            .filter(|item| !fanout::accepted_item_cached(run, &item.id))
            .map(|item| {
                Ok((
                    item.id.clone(),
                    fanout_item_request(&self.store, run, stage, item)?,
                ))
            })
            .collect::<WorkflowResult<Vec<_>>>()?;
        let mut completed = items.len().saturating_sub(requests.len());
        let mut failed = 0usize;
        let results = fanout::run_items_with_runner(
            requests,
            runner,
            width,
            max_agents,
            stage.retry.max_attempts,
        )
        .await?;
        for (item_id, result) in results {
            match result {
                Ok(output) => {
                    ensure_output_usable(&output.body)
                        .map_err(|err| WorkflowError::StageFailed(format!("{item_id}: {err}")))?;
                    let artifact = self.write_stage_artifact_with_acceptance(
                        run,
                        stage,
                        &output.extension,
                        output.body,
                        true,
                    )?;
                    fanout::record_item(
                        run,
                        stage,
                        item_id,
                        StageStatus::Accepted,
                        Some(artifact),
                        None,
                    );
                    completed += 1;
                }
                Err(err) => {
                    let body = format!(
                        "# Fan-out Item Failed\n\nitem: `{item_id}`\nstatus: failed\nerror: {err}\n"
                    );
                    let artifact =
                        self.write_stage_artifact_with_acceptance(run, stage, "md", body, false)?;
                    fanout::record_item(
                        run,
                        stage,
                        item_id,
                        StageStatus::Failed,
                        Some(artifact),
                        Some(err.to_string()),
                    );
                    failed += 1;
                }
            }
        }
        if completed == 0 && failed > 0 {
            return Err(WorkflowError::StageFailed(format!(
                "all {failed} fan-out item(s) failed"
            )));
        }
        Ok(StageRunOutput::markdown(format!(
            "Fan-out stage `{}` completed {} item(s), failed {} item(s), width {}.",
            stage.id, completed, failed, width
        )))
    }

    /// Effective fan-out agent cap for a stage. Local-only provider tiers are
    /// capped by `local_provider_max_agents` (OQ-DWF-003 / EC-DWF-21) so that
    /// Ollama/LM Studio style backends are not overwhelmed by wide fan-out.
    fn stage_max_agents(&self, run: &WorkflowRun, stage: &StageSpec) -> usize {
        let base = run.spec.max_agents.min(self.policy.max_agents_per_run);
        let effective = if stage.provider_tier == Some(crate::spec::ProviderTier::Local) {
            base.min(self.policy.local_provider_max_agents)
        } else {
            base
        };
        effective as usize
    }

    fn run_reduce(&self, run: &mut WorkflowRun, stage: &StageSpec) -> WorkflowResult<()> {
        let reducer = stage.reducer.unwrap_or(ReducerKind::EvidenceWeightedReport);
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
        self.write_stage_artifact_with_acceptance(run, stage, extension, body, true)?;
        Ok(())
    }

    fn write_stage_artifact_with_acceptance(
        &self,
        run: &mut WorkflowRun,
        stage: &StageSpec,
        extension: &str,
        body: String,
        accepted: bool,
    ) -> WorkflowResult<crate::run::ArtifactRef> {
        let input_hash = source_input_hash(stage);
        let mut artifact = self.store.write_artifact(
            &run.id,
            &stage.id,
            &input_hash,
            extension,
            body.as_bytes(),
        )?;
        artifact.accepted = accepted;
        let state = run
            .stage_mut(&stage.id)
            .ok_or_else(|| WorkflowError::SpecInvalid(format!("missing stage {}", stage.id)))?;
        state.artifacts.push(artifact.clone());
        Ok(artifact)
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

fn deterministic_stage_output(stage: &StageSpec) -> String {
    // A stage that declares itself a structured fan-out items producer must emit
    // a parseable `items:` document even in the deterministic (no-live-runner)
    // path, otherwise downstream foreach fan-outs would fail-fast with no items.
    if crate::spec::stage_declares_items_producer(stage) {
        return format!(
            r#"{{"items":[{{"stage":"{}","deterministic":true}}]}}"#,
            stage.id
        );
    }
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
