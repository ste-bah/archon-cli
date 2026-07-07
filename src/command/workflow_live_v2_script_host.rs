struct WorkflowScriptHost {
    scaffold_hash: String,
    runner: WorkflowV2ScriptRunner,
    accumulator: Arc<Mutex<WorkflowScriptAccumulator>>,
}

impl WorkflowScriptHost {
    async fn execute(
        &self,
        method: String,
        payload: String,
    ) -> archon_workflow::WorkflowResult<String> {
        let request: ScriptHostRequest = serde_json::from_str(&payload)?;
        let execution = self.execution_from_request(&method, request)?;
        let mut source_metadata = dynamic_wave_source_metadata(
            &execution,
            self.runner.task_universe.as_ref(),
            self.runner.runtime.target_repository_root.as_deref(),
        );
        let input_hash = input_hash_with_source_fingerprint(
            &execution.input,
            source_metadata.source_fingerprint.as_deref(),
        );
        if let Some(record) = self.runner.v2_store.load_call_record(&execution.call.id)? {
            let source_metadata_reusable = !source_metadata.source_metadata_required
                || source_metadata.source_fingerprint.is_some();
            if source_metadata_reusable
                && record.is_reusable_for_source_and_scaffold(
                    &input_hash,
                    source_metadata.source_fingerprint.as_deref(),
                    Some(&self.scaffold_hash),
                )
                && reusable_record_has_required_completion_evidence(&record)
            {
                self.mark_reused(&record).await?;
                return result_view_json(&record.result);
            }
        }

        poll_v2_run_control(
            &self.runner.workflow_store,
            &self.runner.run_id,
            &execution.call.id,
        )?;
        mark_v2_call_running(
            &self.runner.workflow_store,
            &self.runner.run_id,
            &execution.call.id,
        )?;
        self.emit_v2_event(
            WorkflowEventKind::StageStarted,
            serde_json::json!({
                "event": "call_started",
                "call_id": execution.call.id.clone(),
                "method": execution.call.method.as_str(),
            }),
        );
        let _ = self.runner.client.tui_tx.send(TuiEvent::TextDelta(format!(
            "Workflow V2 script call running: {} via w.{}\n",
            execution.call.id,
            execution.call.method.as_str()
        )));
        let attempt = self
            .runner
            .v2_store
            .load_call_record(&execution.call.id)?
            .map_or(1, |record| record.attempt.saturating_add(1));
        if self.generated_decomposed_prd_run()
            && source_metadata.source_metadata_required
            && source_metadata.source_fingerprint.is_none()
        {
            return self
                .persist_source_metadata_review(execution, source_metadata, input_hash, attempt)
                .await;
        }
        let call_id = execution.call.id.clone();
        let result = match execute_v2_live_call(
            &self.runner.task,
            &self.runner.runtime,
            execution.clone(),
            self.runner.adapter.clone(),
            &self.runner.client,
            &self.runner.v2_store,
            &self.runner.workflow_store,
            &self.runner.run_id,
            self.runner.workspace_boundary_supported,
            self.runner.task_universe.as_ref(),
            source_metadata.source_task_graph.as_ref(),
        )
        .await
        {
            Ok(result) => result,
            Err(err) => {
                if matches!(
                    &err,
                    WorkflowError::ControlPaused(_) | WorkflowError::ControlCancelled(_)
                ) {
                    return Err(err);
                }
                failed_v2_result(&call_id, err)
            }
        };
        let mut result = normalize_result_for_call(&execution, result);
        mark_unresolved_dependency_metadata(&execution, &source_metadata, &mut result);
        let result = match result.validate() {
            Ok(()) => result,
            Err(err) => failed_v2_result(&call_id, WorkflowError::SpecInvalid(err.to_string())),
        };
        if let Some(graph) = source_metadata.source_task_graph.take() {
            source_metadata.source_task_graph = Some(complete_source_task_graph(graph, &result));
        }
        let status = result.status;
        let completion_evidence = completion_evidence_from_result(&result);
        let evidence_snapshot_hash = evidence_snapshot_hash(&completion_evidence);
        let record = WorkflowV2CallRecord::new(
            self.runner.v2_store.run_id(),
            execution.call.clone(),
            attempt,
            input_hash,
            result,
            execution.depends_on,
        )
        .with_source_metadata(
            source_metadata.source_fingerprint.clone(),
            source_metadata.source_task_graph.clone(),
        )
        .with_scaffold_hash(Some(self.scaffold_hash.clone()))
        .with_completion_evidence(completion_evidence)
        .with_evidence_snapshot_hash(evidence_snapshot_hash);
        self.runner.v2_store.save_call_record(&record)?;
        self.update_checkpoint(&record)?;
        self.mark_executed(&record, status).await;
        self.emit_call_finished_event(&record);
        poll_v2_run_control(&self.runner.workflow_store, &self.runner.run_id, "")?;
        if terminal_stop_for_call(&record.call, record.status) {
            let path = self.runner.v2_store.result_path(&record.call.id);
            let next_action = next_action_for_terminal_call(&record.call.id, record.status);
            self.mark_terminal(&record, path.display().to_string(), next_action.clone())
                .await;
            self.emit_v2_event(
                if record.status == WorkflowV2Status::Failed {
                    WorkflowEventKind::StageFailed
                } else {
                    WorkflowEventKind::StageStalled
                },
                serde_json::json!({
                    "event": "script_stopped",
                    "call_id": record.call.id.clone(),
                    "method": record.call.method.as_str(),
                    "status": record.status,
                    "result_path": path.display().to_string(),
                    "next_action": next_action,
                }),
            );
            return Err(WorkflowError::StageFailed(format!(
                "{TERMINAL_HOST_CALL_MARKER} {} ended with {:?}",
                record.call.id, record.status
            )));
        }
        result_view_json(&record.result)
    }

    fn execution_from_request(
        &self,
        method: &str,
        request: ScriptHostRequest,
    ) -> archon_workflow::WorkflowResult<WorkflowV2CallExecution> {
        let method = WorkflowV2HostMethod::parse(method).ok_or_else(|| {
            WorkflowError::SpecInvalid(format!(
                "workflow.js used unsupported host method w.{method}"
            ))
        })?;
        let (options, write_mode) = parse_script_options(&request.options)?;
        if method == WorkflowV2HostMethod::Implementation && write_mode.is_none() {
            return Err(WorkflowError::SpecInvalid(format!(
                "w.implementation('{}') requires explicit write mode serial, coordinated, or worktree",
                request.id
            )));
        }
        let mut input = serde_json::json!({
            "objective": self.runner.task.clone(),
            "call_id": request.id.clone(),
            "method": method.as_str(),
            "write_mode": write_mode,
            "options": request.options,
        });
        if let Some(source) = request.source {
            input["source_data"] = source;
        }
        let has_explicit_source = input.get("source_data").is_some();
        if let Some(inputs) = input
            .get("options")
            .and_then(|options| options.get("inputs"))
            .cloned()
        {
            input["inputs"] = inputs.clone();
            if !has_explicit_source {
                input["source_data"] = inputs;
            }
        }
        if let Some(source) = options.source.as_deref() {
            input["source"] = serde_json::Value::String(source.to_string());
        }
        Ok(WorkflowV2CallExecution {
            call: WorkflowV2HostCall {
                id: request.id,
                method,
                write_mode,
                options,
            },
            input,
            depends_on: Vec::new(),
        })
    }

    fn update_checkpoint(
        &self,
        record: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<()> {
        let mut checkpoint = self
            .runner
            .v2_store
            .load_checkpoint()?
            .unwrap_or_else(WorkflowV2Checkpoint::default);
        if is_reusable_status(record.status) {
            checkpoint.mark_completed(&record.call.id);
        } else {
            checkpoint.remove_completed_call(&record.call.id);
        }
        self.runner.v2_store.save_checkpoint(&checkpoint)
    }

    async fn mark_reused(
        &self,
        record: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<()> {
        let mut checkpoint = self
            .runner
            .v2_store
            .load_checkpoint()?
            .unwrap_or_else(WorkflowV2Checkpoint::default);
        checkpoint.mark_completed(&record.call.id);
        self.runner.v2_store.save_checkpoint(&checkpoint)?;
        let mut acc = self.accumulator.lock().await;
        acc.status = merge_v2_status(acc.status, record.status);
        acc.reused += 1;
        acc.completed += 1;
        acc.calls.push(record.call.clone());
        drop(acc);
        self.emit_v2_event(
            WorkflowEventKind::StageCompleted,
            serde_json::json!({
                "event": "call_reused",
                "call_id": record.call.id.clone(),
                "method": record.call.method.as_str(),
                "status": record.status,
                "result_path": self.runner.v2_store.result_path(&record.call.id).display().to_string(),
            }),
        );
        Ok(())
    }

    async fn mark_executed(&self, record: &WorkflowV2CallRecord, status: WorkflowV2Status) {
        let mut acc = self.accumulator.lock().await;
        // A final report is the script speaking for the whole run: its status
        // overrides accumulated call severities so script-recovered failures
        // do not doom an otherwise accepted run.
        if record.call.method == WorkflowV2HostMethod::FinalReport {
            acc.status = status;
        } else {
            acc.status = merge_v2_status(acc.status, status);
        }
        acc.executed += 1;
        if is_reusable_status(status) {
            acc.completed += 1;
        }
        acc.calls.push(record.call.clone());
    }

    async fn mark_terminal(
        &self,
        record: &WorkflowV2CallRecord,
        result_path: String,
        next_action: String,
    ) {
        let mut acc = self.accumulator.lock().await;
        if record.call.method == WorkflowV2HostMethod::FinalReport {
            acc.status = record.status;
        } else {
            acc.status = merge_v2_status(acc.status, record.status);
        }
        acc.failed_call = Some(record.call.id.clone());
        acc.failed_result_path = Some(result_path);
        acc.next_action = Some(next_action);
    }

    async fn mark_script_failure(&self, error: &str) -> WorkflowV2ScriptSummary {
        let next_action =
            "fix the workflow.js/runtime error, then resume or start a fresh workflow".to_string();
        let mut acc = self.accumulator.lock().await;
        acc.status = merge_v2_status(acc.status, WorkflowV2Status::Failed);
        acc.failed_call = Some("workflow.js".to_string());
        acc.failed_result_path = None;
        acc.next_action = Some(next_action.clone());
        drop(acc);
        self.emit_v2_event(
            WorkflowEventKind::StageFailed,
            serde_json::json!({
                "event": "script_stopped",
                "call_id": "workflow.js",
                "method": "script",
                "status": WorkflowV2Status::Failed,
                "error": error,
                "next_action": next_action,
            }),
        );
        self.emit_terminal_status(WorkflowV2Status::Failed);
        self.summary().await
    }

    fn generated_decomposed_prd_run(&self) -> bool {
        self.runner.task_universe.is_some()
    }

    async fn persist_source_metadata_review(
        &self,
        execution: WorkflowV2CallExecution,
        source_metadata: super::workflow_live_v2_source_graph::DynamicWaveSourceMetadata,
        input_hash: String,
        attempt: u32,
    ) -> archon_workflow::WorkflowResult<String> {
        let call_id = execution.call.id.clone();
        let reason = source_metadata.invalid_reason.clone().unwrap_or_else(|| {
            if source_metadata.unresolved_dependencies.is_empty() {
                "dynamic write fanout source metadata was incomplete".to_string()
            } else {
                format!(
                    "dynamic write fanout has unresolved dependencies: {}",
                    source_metadata.unresolved_dependencies.join(", ")
                )
            }
        });
        let result = WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            summary: reason.clone(),
            data: serde_json::json!({
                "status": "needs_review",
                "outcomes": [],
                "source_metadata_invalid": reason.clone(),
                "unresolved_dependencies": source_metadata.unresolved_dependencies.clone(),
            }),
            evidence: vec![WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Review,
                reason.clone(),
            )],
            residual_gaps: vec![WorkflowV2ResidualGap {
                id: format!(
                    "dynamic_wave_source_metadata_{}",
                    sanitize_v2_gap_id(&call_id)
                ),
                description: reason,
                severity: Some("review".to_string()),
            }],
            ..WorkflowV2Result::default()
        };
        let record = WorkflowV2CallRecord::new(
            self.runner.v2_store.run_id(),
            execution.call.clone(),
            attempt,
            input_hash,
            result,
            execution.depends_on,
        )
        .with_source_metadata(
            source_metadata.source_fingerprint.clone(),
            source_metadata.source_task_graph.clone(),
        )
        .with_scaffold_hash(Some(self.scaffold_hash.clone()));
        self.runner.v2_store.save_call_record(&record)?;
        self.update_checkpoint(&record)?;
        self.mark_executed(&record, record.status).await;
        self.emit_call_finished_event(&record);
        result_view_json(&record.result)
    }

    async fn summary(&self) -> WorkflowV2ScriptSummary {
        let acc = self.accumulator.lock().await;
        WorkflowV2ScriptSummary {
            status: acc.status,
            completed: acc.completed,
            executed: acc.executed,
            reused: acc.reused,
            calls: acc.calls.clone(),
            failed_call: acc.failed_call.clone(),
            failed_result_path: acc.failed_result_path.clone(),
            next_action: acc.next_action.clone(),
        }
    }

    fn emit_call_finished_event(&self, record: &WorkflowV2CallRecord) {
        self.emit_v2_event(
            match record.status {
                WorkflowV2Status::Failed | WorkflowV2Status::Cancelled => {
                    WorkflowEventKind::StageFailed
                }
                WorkflowV2Status::Blocked | WorkflowV2Status::NeedsReview => {
                    WorkflowEventKind::StageStalled
                }
                _ => WorkflowEventKind::StageCompleted,
            },
            serde_json::json!({
                "event": match record.status {
                    WorkflowV2Status::Failed | WorkflowV2Status::Cancelled => "call_failed",
                    WorkflowV2Status::Blocked | WorkflowV2Status::NeedsReview => "call_needs_review",
                    _ => "call_finished",
                },
                "call_id": record.call.id.clone(),
                "method": record.call.method.as_str(),
                "status": record.status,
                "result_path": self.runner.v2_store.result_path(&record.call.id).display().to_string(),
            }),
        );
    }

    fn emit_v2_event(&self, kind: WorkflowEventKind, detail: serde_json::Value) {
        let Ok(seq) = self
            .runner
            .workflow_store
            .next_event_seq(&self.runner.run_id)
        else {
            return;
        };
        let _ = WorkflowEventLog::new(self.runner.workflow_store.clone()).emit(
            &self.runner.run_id,
            seq,
            kind,
            detail,
        );
    }

    fn emit_terminal_status(&self, status: WorkflowV2Status) {
        self.emit_v2_event(
            match status {
                WorkflowV2Status::Accepted | WorkflowV2Status::Noop => {
                    WorkflowEventKind::StageCompleted
                }
                WorkflowV2Status::Failed | WorkflowV2Status::Cancelled => {
                    WorkflowEventKind::StageFailed
                }
                WorkflowV2Status::Blocked | WorkflowV2Status::NeedsReview => {
                    WorkflowEventKind::StageStalled
                }
                WorkflowV2Status::Pending | WorkflowV2Status::Running => {
                    WorkflowEventKind::StageStarted
                }
            },
            serde_json::json!({
                "event": "terminal_status",
                "status": status,
            }),
        );
    }
}
