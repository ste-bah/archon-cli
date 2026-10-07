// One of three inherent `impl WorkflowScriptHost` blocks split out of
// `workflow_live_v2_script_host.rs` to hold the 500-line ceiling.

// The free task-id helpers, the remediation replay and the completed-task
// reuse lookup live beside this file to hold the 500-line ceiling.
#[path = "workflow_live_v2_script_host_completed_reuse.rs"]
mod completed_reuse;
#[path = "workflow_live_v2_script_host_dispatch.rs"]
mod dispatch;
#[path = "workflow_live_v2_script_host_remediation.rs"]
mod remediation;
#[path = "workflow_live_v2_script_host_task_ids.rs"]
mod task_ids;
use super::*;
use remediation::asks_the_same;

impl WorkflowScriptHost {
    async fn fixed_host_record_reusable(
        &self,
        record: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        if record.call.method != WorkflowV2HostMethod::HostCommand {
            return self.refresh_audit_for_cache(record).await;
        }
        self.runner
            .host_command_executor
            .as_ref()
            .ok_or_else(|| {
                WorkflowError::PolicyDenied(
                    "HostCommand reuse requires the trusted fixed executor".to_string(),
                )
            })?
            .record_is_reusable(record)
    }

    /// One host call, once the session's control refusal gate has passed
    /// (`execute`, `workflow_live_v2_script_host_control_refusal.rs`).
    pub(super) async fn execute_host_call(
        &self,
        method: String,
        payload: String,
    ) -> archon_workflow::WorkflowResult<String> {
        let retry = (method.clone(), payload.clone());
        {
            // Issue-285: only script calls are refused; host fallbacks run.
            let acc = self.accumulator.lock().await;
            if acc.terminal_locked() {
                return Err(WorkflowError::TerminalHostCall(format!(
                    "run {} has already stopped with {:?}",
                    self.runner.run_id, acc.status
                )));
            }
        }
        // Issue 291: a session a resume replaced dispatches nothing -- no
        // tool, pause or workflow call -- and is told so. An unreadable state
        // proves nothing here; the paths below report it as before.
        if let Err(stale @ WorkflowError::ControlCancelled(_)) = self.owned_generation() {
            return Err(stale);
        }
        // #189 Phase 4. Intercepted before the call is turned into a
        // `WorkflowV2CallExecution`: a tool call is not a workflow call. It
        // produces no stored record, takes part in no reuse, and has nothing to
        // verify — routing it through that machinery would make every one of
        // those concepts mean something weaker.
        if method == crate::command::workflow_live::workflow_script_tools::RUN_TOOL_METHOD {
            return self.run_script_tool(&payload).await;
        }
        // Issue 299: any other host call is other work, after which a repeated
        // tool call may read a changed world; it starts a new repeat streak.
        self.tool_budget
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .break_streak();
        // Issue 261: a pause request is control flow, not a workflow call.
        if method == archon_workflow::v2::script::SCRIPT_PAUSE_METHOD {
            return self.request_script_pause(&payload).await;
        }
        let request: ScriptHostRequest = serde_json::from_str(&payload)?;
        let mut execution = self.execution_from_request(&method, request)?;
        if execution.call.method == WorkflowV2HostMethod::HostCommand {
            let request = execution
                .call
                .options
                .host_command
                .as_ref()
                .ok_or_else(|| {
                    WorkflowError::SpecInvalid(
                        "HostCommand call is missing its typed request".to_string(),
                    )
                })?;
            let executor = self.runner.host_command_executor.as_ref().ok_or_else(|| {
                WorkflowError::PolicyDenied(
                    "HostCommand is available only to a trusted fixed workflow run".to_string(),
                )
            })?;
            let identity = executor.call_identity(request)?;
            let occurrence = self.host_occurrences.next(&identity);
            crate::command::workflow_host_command_occurrence::stamp_occurrence(
                &mut execution.call,
                &mut execution.input,
                &identity,
                occurrence,
            );
        }
        if let Some(view) = self.escalation_refused_view(&execution)? {
            return Ok(view);
        }
        let mut source_metadata = dynamic_wave_source_metadata(
            &execution,
            self.runner.task_universe.as_ref(),
            self.runner.runtime.target_repository_root.as_deref(),
        );
        let execution_generation = self.fixed_execution_generation()?;
        // A stall belongs to the generation at entry, including a non-fixed
        // script. Never re-sample ownership after an awaited dispatch.
        let view_generation = self.owned_generation()?;
        let input_hash = input_hash_with_source_fingerprint(
            &execution.input,
            source_metadata.source_fingerprint.as_deref(),
        );
        poll_v2_run_control(
            &self.runner.workflow_store,
            &self.runner.run_id,
            &execution.call.id,
        )?;
        // An acceptance round measures the repository as it is NOW; a stored
        // round describes a tree that may no longer exist, so it is never
        // replayed, by any reuse path. Re-running costs one round of checks.
        let reusable_kind = !archon_workflow::v2::script::is_acceptance_stage_call(&execution.call);
        // Issue 261: what a taken pause covers replays verbatim, first.
        if reusable_kind
            && let Some(view) = self
                .replay_covered_attempt(&execution, &input_hash, execution_generation)
                .await?
        {
            return Ok(view);
        }
        if reusable_kind
            && let Some(view) = self
                .replay_superseded_history(&execution, &input_hash, execution_generation)
                .await?
        {
            return Ok(view);
        }
        if reusable_kind
            && let Some(candidate) = self
                .runner
                .v2_store
                .call_record_for_reuse(&execution.call, &input_hash)?
        {
            let (record, from_history) = (candidate.record, candidate.from_history);
            // Restart/resume from a task: a call whose tasks are ALL already
            // recorded complete must be reused directly — including its
            // verification — without re-checking scaffold/input hashes. A
            // re-authored script or a fresh verifier input would otherwise
            // fail the hash match and force every prior task to re-validate
            // from the top, which is exactly what `restart task <id>` must not
            // do. Only accepted/noop, non-invalidated, still-valid records for
            // tasks in the completed set qualify.
            //
            // The hash is deliberately still not consulted (a re-authored
            // script legitimately changes a done task's call input), but the
            // waiver is BOUNDED to work this run has not touched: once an
            // upstream task re-executed here, downstream records are stale and
            // fall through to the content-keyed paths below.
            // Batch H: a remediation answer older than its question's latest
            // observation answers another question, by every path below; and
            // the waiver below never covers another question (findings).
            let predates = self.answer_predates_question(&execution, &record)?;
            let asked =
                !archon_workflow::v2::script::resume_drift::is_remediation_call(&execution.call)
                    || asks_the_same(&record.call, &execution.call);
            if !predates
                && asked
                && record_tasks_all_completed(&record, &self.runner.resume_completed_ids)
                && !self.hash_free_reuse_stale(&record)
                && is_reusable_status(record.status)
                && record.invalidated_by.is_none()
                && record.result.validate().is_ok()
                && reusable_record_has_required_completion_evidence(&record)
                && self.verdict_vouches(&record)?
                && self.fixed_host_record_reusable(&record).await?
            {
                self.restore_reused_record(&record, from_history, execution_generation)?;
                self.mark_reused(&record, execution_generation).await?;
                return self.result_view_in_generation(&record, view_generation);
            }
            let source_metadata_reusable = !source_metadata.source_metadata_required
                || source_metadata.source_fingerprint.is_some();
            let strict_reuse = source_metadata_reusable
                && record.is_reusable_for_source_and_scaffold(
                    &input_hash,
                    source_metadata.source_fingerprint.as_deref(),
                    Some(&self.scaffold_hash),
                );
            // Frontier adoption must not resurrect results whose dynamic
            // source graph diverged: when this call requires source metadata,
            // the recorded fingerprint has to match the current one.
            let frontier_reuse = self.runner.adopt_accepted_cache
                && frontier_resume_record_reusable(&record, &input_hash, &self.scaffold_hash)
                && (!source_metadata.source_metadata_required
                    || (source_metadata.source_fingerprint.is_some()
                        && record.source_fingerprint == source_metadata.source_fingerprint));
            if (strict_reuse || frontier_reuse)
                && !predates
                && reusable_record_has_required_completion_evidence(&record)
                && self.verdict_vouches(&record)?
                && self.fixed_host_record_reusable(&record).await?
            {
                poll_v2_run_control(
                    &self.runner.workflow_store,
                    &self.runner.run_id,
                    &execution.call.id,
                )?;
                if !self.fixed_decomposition_state_present() {
                    self.runner
                        .client
                        .ui_sink
                        .emit(WorkflowUiEvent::Text(format!(
                            "Workflow V2 script call reused: {} via w.{}\n",
                            execution.call.id,
                            execution.call.method.as_str()
                        )))
                        .await
                        .map_err(|error| {
                            WorkflowError::NotificationDelivery(format!(
                                "workflow call reuse status delivery failed: run_id={} stage_id={} status=reused: {error}",
                                self.runner.run_id, execution.call.id
                            ))
                        })?;
                }
                self.restore_reused_record(&record, from_history, execution_generation)?;
                self.mark_reused(&record, execution_generation).await?;
                return self.result_view_in_generation(&record, view_generation);
            }
        }

        // Ordinal-drift resilience: v3 call ids embed a global ordinal that
        // shifts across re-runs when reused tasks skip their remediation loops,
        // so a completed task's call arrives under a NEW id with no record at
        // `execution.call.id`. When the call belongs to a task already in the
        // completed set, reuse that task's accepted record of the same kind
        // (implement vs verify) regardless of the ordinal — this is what makes
        // `restart`/continue actually skip 010–079 instead of re-validating.
        if reusable_kind
            && let Some(record) = self.reusable_completed_task_record(&execution)?
            && self.refresh_audit_for_cache(&record).await?
        {
            self.mark_reused(&record, execution_generation).await?;
            return self.result_view_in_generation(&record, view_generation);
        }
        // Review remediation under a shifted ordinal, or a round a later round
        // superseded: replayed by content (`remediation_replay`). A write is
        // refused by the audit gate here and reused per branch instead.
        if reusable_kind
            && let Some(record) = self.remediation_replay(&execution)?
            && self.refresh_audit_for_cache(&record).await?
        {
            self.mark_reused(&record, execution_generation).await?;
            return self.result_view_in_generation(&record, view_generation);
        }
        self.note_fix_runs(&execution);

        if !self.fixed_decomposition_state_present() {
            self.runner
                .client
                .ui_sink
                .emit(WorkflowUiEvent::Text(format!(
                    "Workflow V2 script call running: {} via w.{}\n",
                    execution.call.id,
                    execution.call.method.as_str()
                )))
                .await
                .map_err(|error| {
                    WorkflowError::NotificationDelivery(format!(
                        "workflow call status delivery failed: run_id={} stage_id={} status=running: {error}",
                        self.runner.run_id, execution.call.id
                    ))
                })?;
        }
        poll_v2_run_control(
            &self.runner.workflow_store,
            &self.runner.run_id,
            &execution.call.id,
        )?;
        self.mark_call_running_owned(&execution.call.id)?;
        self.emit_v2_event(
            WorkflowEventKind::StageStarted,
            serde_json::json!({
                "event": "call_started",
                "call_id": execution.call.id.clone(),
                "method": execution.call.method.as_str(),
            }),
        );
        // A superseded or interrupted dispatch's admission keeps its attempt;
        // this dispatch is a new admission (Issue 291, round 4).
        let attempt = self
            .runner
            .v2_store
            .next_dispatch_attempt(&execution.call.id, &input_hash)?;
        if self.generated_decomposed_prd_run()
            && source_metadata.source_metadata_required
            && source_metadata.source_fingerprint.is_none()
        {
            return self
                .persist_source_metadata_review(execution, source_metadata, input_hash, attempt)
                .await;
        }
        self.require_fixed_generation_owned(execution_generation)?;
        // Track before the started projection's asynchronous UI delivery:
        // a sibling stop must close that projection even while delivery waits.
        let dispatched_at = std::time::Instant::now();
        let call_generation = Some(self.call_generation()?);
        self.track_pending_call(
            &execution,
            attempt,
            &input_hash,
            source_metadata.source_fingerprint.clone(),
            call_generation,
            dispatched_at,
        )?;
        if let Err(err) = self
            .persist_fixed_call_started(&execution, attempt, &input_hash, execution_generation)
            .await
        {
            // Issue 303: a saved started record never outlives this executor.
            self.close_unstarted_call(
                &execution,
                attempt,
                &input_hash,
                source_metadata.source_fingerprint.clone(),
                execution_generation,
                dispatched_at,
                &err,
            )
            .await;
            return Err(err);
        }
        let call_id = execution.call.id.clone();
        // The generation this dispatch runs under (Issue 263): only it may
        // pause the run for a never-started streak.
        let dispatch_generation = execution_generation.or(call_generation);
        let dispatched = self
            .refreshing_inflight(
                &execution,
                attempt,
                &input_hash,
                // Boxed (#246): by value, the dispatch future was copied into
                // every wrapper and their frames overflowed a 2 MiB stack.
                Box::pin(self.dispatch_live(
                    &execution,
                    source_metadata.source_task_graph.as_ref(),
                    execution_generation,
                    call_generation,
                )),
            )
            .await;
        // Round 5 (#253): a result from before a lifecycle edit is dropped.
        let dispatched = match dispatched {
            Ok(Some(result)) if !self.call_superseded(&execution, call_generation) => Ok(result),
            Err(err) if !self.call_superseded(&execution, call_generation) => Err(err),
            _ => {
                return self
                    .redispatch_superseded(&execution, retry.0, retry.1)
                    .await;
            }
        };
        let dispatched = match dispatched {
            // Issue 263: a streak of never-started dispatches pauses the run,
            // recorded below like any other pause.
            Err(err) => Err(self
                .pause_on_never_started_streak(&call_id, dispatch_generation, err)
                .await),
            ok => ok,
        };
        let result = match dispatched {
            Ok(result) => result,
            Err(err)
                if control_interruption_reason(&err).is_none()
                    && !matches!(&err, WorkflowError::NotificationDelivery(_))
                    && !self.stops_on_host_fault(&err) =>
            {
                self.result_for_failed_dispatch(&call_id, err).await?
            }
            Err(err) => {
                let control = control_interruption_reason(&err);
                let control_or_fault = control.or(self
                    .stops_on_host_fault(&err)
                    .then_some(workflow_live_v2_script_host_interrupt::HOST_FAULT_REASON));
                if control.is_some() {
                    // Issue-134: the run's call trees end before any record.
                    archon_tools::bash::end_process_groups_of(&self.runner.run_id);
                    // Round 7 (#285): a prior terminal stop records the call.
                    if !self.fixed_generation_may_record_interruption(execution_generation)
                        || self.accumulator.lock().await.terminal_host_stop
                    {
                        return Err(err);
                    }
                }
                // Issue-213 C5: an undelivered call's record says why.
                let reason = control_or_fault.unwrap_or(
                    workflow_live_v2_script_host_interrupt::NOTIFICATION_DELIVERY_REASON,
                );
                if let Err(save_err) = self
                    .save_interrupted_call_record(
                        &execution,
                        reason,
                        &err,
                        dispatched_at.elapsed(),
                        attempt,
                        &input_hash,
                        source_metadata.source_fingerprint.clone(),
                        execution_generation,
                    )
                    .await
                {
                    // Round 7: the original stop stays the outcome.
                    tracing::warn!(%call_id, %save_err, "interruption record not saved");
                }
                return Err(err);
            }
        };
        let mut result = normalize_and_attach_review_findings(
            &execution,
            result,
            &self.runner.v2_store,
            self.runner.task_universe.as_ref(),
        )?;
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
        let dispatched_items = archon_workflow::v2::call_data::dispatched_items(&execution);
        let record = WorkflowV2CallRecord::new(
            self.runner.v2_store.run_id(),
            execution.call.clone(),
            attempt,
            input_hash,
            result,
            execution.depends_on.clone(),
        )
        .with_source_metadata(
            source_metadata.source_fingerprint.clone(),
            source_metadata.source_task_graph.clone(),
        )
        .with_scaffold_hash(Some(self.scaffold_hash.clone()))
        .with_completion_evidence(completion_evidence)
        .with_evidence_snapshot_hash(evidence_snapshot_hash)
        .with_dispatched_items(dispatched_items)
        .with_agent_sessions(self.take_call_sessions(&call_id));
        if !self
            .publish_dispatched_call(&record, call_generation, self.call_fenced(&execution))
            .await?
        {
            return self
                .redispatch_superseded(&execution, retry.0, retry.1)
                .await;
        }
        self.clear_inflight(&call_id);
        self.mark_tasks_reexecuted(&record);
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
            return Err(WorkflowError::TerminalHostCall(format!(
                "{} ended with {:?}",
                record.call.id, record.status
            )));
        }
        self.result_view_in_generation(&record, view_generation)
    }
}
