use super::*;

impl WorkflowExecutor {
    pub(super) fn run_stage(
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

    pub(super) fn run_reduce(
        &self,
        run: &mut WorkflowRun,
        stage: &StageSpec,
    ) -> WorkflowResult<()> {
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

    pub(super) fn run_quality_gate(
        &self,
        run: &mut WorkflowRun,
        stage: &StageSpec,
    ) -> WorkflowResult<()> {
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

    pub(super) fn write_condition_artifact(
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

    pub(super) fn record_runner_failure_if_missing(
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
