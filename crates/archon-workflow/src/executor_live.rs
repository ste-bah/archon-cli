use serde_json::json;

use crate::acceptance::{self, AcceptanceOutcome};
use crate::error::{WorkflowError, WorkflowResult};
use crate::events::{WorkflowEventKind, WorkflowEventLog};
use crate::exec_state::{mark_finished, mark_started, pause_for_human_gate};
use crate::executor_fanout;
use crate::executor_output::ensure_stage_output_usable;
use crate::persistence;
use crate::request::stage_request;
use crate::run::{StageStatus, WorkflowRun};
use crate::runner::{StageRunOutput, WorkflowStageRunner};
use crate::source_context;
use crate::spec::{StageKind, StageSpec};
use crate::work_unit_coverage::CoverageVerdict;
use crate::work_unit_gate;

#[path = "executor_live/direct_verify.rs"]
mod direct_verify;

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
        self.store.save_state_preserving_control(run)?;
        log.emit(
            &run.id,
            *seq,
            WorkflowEventKind::StageStarted,
            json!({"stage": stage.id, "kind": format!("{:?}", stage.kind), "agent": stage.agent}),
        )?;
        *seq += 1;
        if stage.kind == StageKind::HumanGate {
            pause_for_human_gate(run, stage, seq, &log)?;
            return self.store.save_state_preserving_control(run);
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
                run.status = crate::run::RunStatus::Paused;
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
                run.status = crate::run::RunStatus::Cancelled;
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
                if crate::stage::stage_failure_feeds_downstream_recovery(&run.spec, &stage.id, &err)
                {
                    mark_finished(
                        run,
                        stage,
                        StageStatus::ForcedAccepted,
                        Some(err.to_string()),
                    )?;
                    log.emit(
                        &run.id,
                        *seq,
                        WorkflowEventKind::StageCompleted,
                        json!({
                            "stage": stage.id,
                            "status": "forced_accepted_for_remediation",
                            "error_class": "stage_failed",
                            "reason": err.to_string(),
                        }),
                    )?;
                } else {
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
        }
        *seq += 1;
        self.store.save_state_preserving_control(run)
    }

    async fn dispatch_stage_with_runner(
        &self,
        run: &mut WorkflowRun,
        stage: &StageSpec,
        runner: &dyn WorkflowStageRunner,
    ) -> WorkflowResult<StageRunOutput> {
        if let Some(output) =
            crate::remediation_noop::attach_agent_noop_if_empty(&self.store, run, stage)?
        {
            return Ok(output);
        }
        match stage.kind {
            StageKind::Tool
                if stage.tool.as_deref()
                    == Some(crate::required_artifacts::REQUIRED_ARTIFACT_INVENTORY_TOOL) =>
            {
                self.run_required_artifact_inventory(run, stage)?;
                Ok(StageRunOutput::markdown(format!(
                    "Required artifact inventory `{}` complete.",
                    stage.id
                )))
            }
            StageKind::Agent | StageKind::Tool if direct_verify::should_run(stage) => {
                self.run_direct_verify_command(run, stage)
            }
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
        let mut request = stage_request(&self.store, run, stage)?;
        persistence::record_prompt(&self.store, &request)?;
        let mut output = runner.run_stage(request.clone()).await?;
        if let Some(retry) =
            super::executor_live_retry::confirmation_retry_request(&request, &output.body)
        {
            request = retry;
            persistence::record_prompt(&self.store, &request)?;
            output = runner.run_stage(request).await?;
        }
        self.attach_runner_output(run, stage, output).await
    }

    fn run_direct_verify_command(
        &self,
        run: &mut WorkflowRun,
        stage: &StageSpec,
    ) -> WorkflowResult<StageRunOutput> {
        let root = direct_verify::root(&self.store, run, stage)?;
        let report = crate::command_execution::run_verify_command(
            &self.store,
            run,
            stage,
            &root,
            stage.verify_command.as_deref(),
        )?;
        let Some(report) = report else {
            return Err(WorkflowError::StageFailed(format!(
                "local verification stage '{}' declared no verify_command",
                stage.id
            )));
        };
        let accepted = report.success();
        let body = direct_verify::body(stage, &root, &report, accepted);
        let output = StageRunOutput {
            body,
            extension: "json".into(),
            provider_id: Some("local-verify-command".into()),
            resolved_model: Some("shell".into()),
            tokens_in: 0,
            tokens_out: 0,
            cost_usd: 0.0,
            tool_uses: Vec::new(),
        };
        let artifact = persistence::write_attached_stage_artifact(
            &self.store,
            run,
            stage,
            &stage.id,
            &output.extension,
            output.body.clone(),
            accepted,
        )?;
        let error = (!accepted).then(|| report.failure_reason());
        persistence::record_agent_output(
            &self.store,
            &run.id,
            &stage.id,
            &stage.id,
            Some(&output),
            Some(&artifact),
            accepted,
            error.as_deref(),
        )?;
        if accepted {
            Ok(output)
        } else {
            Err(WorkflowError::StageFailed(error.unwrap_or_else(|| {
                format!("verify_command failed for stage '{}'", stage.id)
            })))
        }
    }

    async fn run_implementation_with_runner(
        &self,
        run: &mut WorkflowRun,
        stage: &StageSpec,
        runner: &dyn WorkflowStageRunner,
    ) -> WorkflowResult<StageRunOutput> {
        let root = source_context::implementation_root_for_payload_targets(
            &self.store,
            run,
            &stage.input,
            &stage.expected_target_files,
        )?;
        let request = stage_request(&self.store, run, stage)?;
        persistence::record_prompt(&self.store, &request)?;
        let output = runner.run_stage(request).await?;
        if let Err(err) = ensure_stage_output_usable(stage, &output.body) {
            self.record_unusable_output(run, stage, &output, &err)?;
            return Err(err);
        }
        let after = acceptance::snapshot_targets(&root, &stage.expected_target_files);
        let outcome = self.evaluate_implementation_acceptance(run, stage, &root, &after)?;
        self.attach_implementation_output(run, stage, output, outcome)
    }

    fn evaluate_implementation_acceptance(
        &self,
        run: &WorkflowRun,
        stage: &StageSpec,
        root: &std::path::Path,
        after: &acceptance::TargetFingerprints,
    ) -> WorkflowResult<AcceptanceOutcome> {
        if stage.expected_target_files.is_empty() {
            return Ok(AcceptanceOutcome::Rejected(
                "implementation stage declared no expected_target_files".to_string(),
            ));
        }
        let missing = acceptance::missing_targets(after);
        if !missing.is_empty() {
            return Ok(AcceptanceOutcome::Rejected(format!(
                "expected_target_files missing after implementation: {}",
                missing.join(", ")
            )));
        }
        let Some(report) = crate::command_execution::run_verify_command(
            &self.store,
            run,
            stage,
            root,
            stage.verify_command.as_deref(),
        )?
        else {
            return Ok(AcceptanceOutcome::Accepted);
        };
        if report.success() {
            Ok(AcceptanceOutcome::Accepted)
        } else {
            Ok(AcceptanceOutcome::Rejected(report.failure_reason()))
        }
    }

    async fn attach_runner_output(
        &self,
        run: &mut WorkflowRun,
        stage: &StageSpec,
        mut output: StageRunOutput,
    ) -> WorkflowResult<StageRunOutput> {
        match crate::remediation_inventory::repair_empty_inventory_output(
            &self.store,
            run,
            stage,
            &output.body,
        ) {
            Ok(Some(body)) => {
                output.body = body;
                output.extension = "json".into();
            }
            Ok(None) => {}
            Err(err) => {
                self.record_unusable_output(run, stage, &output, &err)?;
                return Err(err);
            }
        }
        if let Err(err) = ensure_stage_output_usable(stage, &output.body) {
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
        let coverage = work_unit_gate::evaluate_stage_output(run, stage, &output.body);
        let coverage_accepted = coverage
            .as_ref()
            .is_none_or(|coverage| coverage.verdict == CoverageVerdict::Accepted);
        let accepted = outcome.is_accepted() && coverage_accepted;
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
        if let Some(coverage) = coverage
            && coverage.verdict != CoverageVerdict::Accepted
        {
            let reason = work_unit_gate::error_summary(&coverage);
            work_unit_gate::write_coverage_artifact(&self.store, run, stage, &coverage, false)?;
            let remediation = crate::work_unit_remediation::write_missing_unit_items(
                &self.store,
                run,
                stage,
                &coverage,
                vec![crate::work_unit_remediation::source_from_stage(stage)],
                self.policy.missing_unit_remediation_max_attempts,
            )?;
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
            let message = format!("implementation stage '{}' rejected: {reason}", stage.id);
            if remediation.attempts_exhausted {
                return Err(WorkflowError::StageBlocked(message));
            }
            return Err(WorkflowError::StageFailed(message));
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
