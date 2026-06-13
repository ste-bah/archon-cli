use serde_json::json;

use crate::acceptance::{self, AcceptanceOutcome};
use crate::error::{WorkflowError, WorkflowResult};
use crate::events::{WorkflowEventKind, WorkflowEventLog};
use crate::exec_state::{mark_finished, mark_started, pause_for_human_gate};
use crate::executor_fanout;
use crate::executor_output::ensure_output_usable;
use crate::persistence;
use crate::request::stage_request;
use crate::run::{StageStatus, WorkflowRun};
use crate::runner::{StageRunOutput, WorkflowStageRunner};
use crate::source_context;
use crate::spec::{StageKind, StageSpec};

use super::WorkflowExecutor;

impl WorkflowExecutor {
    pub(super) async fn run_stage_with_runner(
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
        match stage.kind {
            StageKind::Agent | StageKind::Tool => self.run_agent_like(run, stage, runner).await,
            StageKind::Implementation => {
                self.run_implementation_with_runner(run, stage, runner)
                    .await
            }
            StageKind::Fanout => {
                executor_fanout::run_fanout_with_runner(
                    &self.store,
                    &self.policy,
                    run,
                    stage,
                    runner,
                )
                .await
            }
            StageKind::Reduce => {
                self.run_reduce(run, stage)?;
                Ok(StageRunOutput::markdown(format!(
                    "Reducer stage `{}` complete.",
                    stage.id
                )))
            }
            StageKind::QualityGate => {
                self.run_quality_gate(run, stage)?;
                Ok(StageRunOutput::markdown(format!(
                    "Quality gate `{}` passed.",
                    stage.id
                )))
            }
            StageKind::Condition | StageKind::Checkpoint => {
                self.write_condition_artifact(run, stage)?;
                Ok(StageRunOutput::markdown(format!(
                    "Checkpoint stage `{}` complete.",
                    stage.id
                )))
            }
            StageKind::HumanGate => unreachable!("human gates pause before dispatch"),
        }
    }

    async fn run_agent_like(
        &self,
        run: &mut WorkflowRun,
        stage: &StageSpec,
        runner: &dyn WorkflowStageRunner,
    ) -> WorkflowResult<StageRunOutput> {
        let request = stage_request(&self.store, run, stage)?;
        persistence::record_prompt(&self.store, &request)?;
        let output = runner.run_stage(request).await?;
        self.attach_runner_output(run, stage, output).await
    }

    async fn run_implementation_with_runner(
        &self,
        run: &mut WorkflowRun,
        stage: &StageSpec,
        runner: &dyn WorkflowStageRunner,
    ) -> WorkflowResult<StageRunOutput> {
        let root = source_context::implementation_root_for_targets(
            &self.store,
            run,
            &stage.expected_target_files,
        )?;
        let before = acceptance::snapshot_targets(&root, &stage.expected_target_files);
        let request = stage_request(&self.store, run, stage)?;
        persistence::record_prompt(&self.store, &request)?;
        let output = runner.run_stage(request).await?;
        if let Err(err) = ensure_output_usable(&output.body) {
            self.record_unusable_output(run, stage, &output, &err)?;
            return Err(err);
        }
        let after = acceptance::snapshot_targets(&root, &stage.expected_target_files);
        let outcome = acceptance::evaluate(
            &root,
            &stage.expected_target_files,
            &before,
            &after,
            stage.verify_command.as_deref(),
        );
        self.attach_implementation_output(run, stage, output, outcome)
    }

    async fn attach_runner_output(
        &self,
        run: &mut WorkflowRun,
        stage: &StageSpec,
        output: StageRunOutput,
    ) -> WorkflowResult<StageRunOutput> {
        if let Err(err) = ensure_output_usable(&output.body) {
            self.record_unusable_output(run, stage, &output, &err)?;
            return Err(err);
        }
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
        Ok(output)
    }

    fn attach_implementation_output(
        &self,
        run: &mut WorkflowRun,
        stage: &StageSpec,
        output: StageRunOutput,
        outcome: AcceptanceOutcome,
    ) -> WorkflowResult<StageRunOutput> {
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

    fn record_unusable_output(
        &self,
        run: &mut WorkflowRun,
        stage: &StageSpec,
        output: &StageRunOutput,
        err: &WorkflowError,
    ) -> WorkflowResult<()> {
        let artifact = persistence::write_attached_stage_artifact(
            &self.store,
            run,
            stage,
            &stage.id,
            &output.extension,
            output.body.clone(),
            false,
        )?;
        persistence::record_agent_output(
            &self.store,
            &run.id,
            &stage.id,
            &stage.id,
            Some(output),
            Some(&artifact),
            false,
            Some(&err.to_string()),
        )
    }
}
