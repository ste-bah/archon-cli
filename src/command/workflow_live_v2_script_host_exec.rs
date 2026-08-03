// WorkflowScriptHost: call reuse and execution.
// One of three inherent `impl WorkflowScriptHost` blocks split out of
// `workflow_live_v2_script_host.rs` to hold the 500-line ceiling.

/// Canonical task ids a stored record speaks for.
///
/// Three sources, unioned, because no single one is populated for every call
/// kind: wave records carry `completed_ids`/`completion_evidence`, while a v3
/// `implement-task-*`/`remediate-task-*` record carries no task-id evidence at
/// all and names its task only in the call id.

use super::*;

pub(super) fn record_task_ids(
    record: &WorkflowV2CallRecord,
    universe: Option<&WorkflowV2TaskUniverse>,
) -> std::collections::BTreeSet<String> {
    let mut tasks = record
        .completed_ids
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    for evidence in &record.completion_evidence {
        let task_id = evidence.task_id.trim();
        if !task_id.is_empty() {
            tasks.insert(task_id.to_string());
        }
    }
    if let Some(universe) = universe {
        let call_id = record.call.id.to_ascii_lowercase();
        for task in &universe.tasks {
            if call_id_names_task(&call_id, &task.canonical_task_id.to_ascii_lowercase()) {
                tasks.insert(task.canonical_task_id.clone());
            }
        }
    }
    tasks
}

/// Whether a lowercased call id embeds a lowercased canonical task id as a
/// whole token. A bare `contains` would let a shorter id (`TASK-01`) match the
/// call id of a longer one (`TASK-010`) and taint an unrelated task, so the
/// match must not be followed by another alphanumeric character.
pub(super) fn call_id_names_task(call_id_lower: &str, task_id_lower: &str) -> bool {
    if task_id_lower.is_empty() {
        return false;
    }
    call_id_lower
        .match_indices(task_id_lower)
        .any(|(start, _)| {
            call_id_lower[start + task_id_lower.len()..]
                .chars()
                .next()
                .is_none_or(|next| !next.is_ascii_alphanumeric())
        })
}

impl WorkflowScriptHost {
    /// Record that a call just RE-EXECUTED, so every task it speaks for — and
    /// everything downstream of those tasks in the authoritative task universe —
    /// can no longer be served from cache by a reuse path that cannot key on the
    /// input hash. Nothing else fires an invalidation mid-run: the store's
    /// `invalidate_*` routines are reachable only from `workflow restart`.
    pub(super) fn mark_tasks_reexecuted(&self, record: &WorkflowV2CallRecord) {
        let Some(universe) = self.runner.task_universe.as_ref() else {
            // No task universe means no dependency graph — and also no
            // `resume_completed_ids`, so the hash-free reuse paths are inert.
            return;
        };
        let touched = record_task_ids(record, Some(universe));
        if touched.is_empty() {
            return;
        }
        let closure = touched
            .iter()
            .flat_map(|task_id| universe.downstream_task_closure(task_id))
            .collect::<Vec<_>>();
        if let Ok(mut dirty) = self.runner.reexecuted_task_closure.lock() {
            dirty.extend(closure);
        }
    }

    /// Whether reusing `record` WITHOUT an input-hash match would replay a
    /// result whose inputs have already moved under it in this run.
    pub(super) fn hash_free_reuse_stale(&self, record: &WorkflowV2CallRecord) -> bool {
        let Ok(dirty) = self.runner.reexecuted_task_closure.lock() else {
            // A poisoned lock means we cannot prove freshness; fail closed onto
            // the content-keyed paths rather than replay blind.
            return true;
        };
        if dirty.is_empty() {
            return false;
        }
        record_task_ids(record, self.runner.task_universe.as_ref())
            .iter()
            .any(|task_id| dirty.contains(task_id))
    }

    /// Find an accepted stored record to reuse for a call whose task is already
    /// completed but whose ordinal-suffixed id shifted on re-run. Matches by
    /// task + kind (verify vs implement/remediate), preferring the latest
    /// accepted attempt. Only scans when the call actually belongs to a
    /// completed task, so non-completed calls pay no cost.
    pub(super) fn reusable_completed_task_record(
        &self,
        execution: &WorkflowV2CallExecution,
    ) -> archon_workflow::WorkflowResult<Option<WorkflowV2CallRecord>> {
        let completed = &self.runner.resume_completed_ids;
        if completed.is_empty() {
            return Ok(None);
        }
        // Only v3 implement/verify calls reuse this way, and only against
        // records of the SAME v3 family — never a stale decomposed record.
        let Some(want_family) = v3_call_family(&execution.call.id) else {
            return Ok(None);
        };
        // Which completed task does this call belong to? Match by the canonical
        // task token embedded in the call id.
        let call_id_lower = execution.call.id.to_ascii_lowercase();
        let Some(task_token) = completed
            .iter()
            .map(|task| task.to_ascii_lowercase())
            .find(|token| call_id_lower.contains(token.as_str()))
        else {
            return Ok(None);
        };
        // Match candidate records to the same task by CALL ID token, NOT by
        // completion evidence: implement/remediate records carry no task-id
        // evidence (only verify records do), so an evidence check would reject
        // the accepted remediate record that actually satisfies the task. The
        // task is already confirmed complete (it's in `completed`), the family
        // is fixed, and the record must be accepted+valid — that is sufficient.
        let mut best: Option<WorkflowV2CallRecord> = None;
        for record in self.runner.v2_store.load_call_records()? {
            if v3_call_family(&record.call.id) != Some(want_family) {
                continue;
            }
            if !record.call.id.to_ascii_lowercase().contains(&task_token) {
                continue;
            }
            if !(is_reusable_status(record.status)
                && record.invalidated_by.is_none()
                && record.result.validate().is_ok())
            {
                continue;
            }
            // This path CANNOT key on the input hash: it exists precisely
            // because the call arrives under a new ordinal-suffixed id, and the
            // call id is part of the hashed input, so the hashes can never match
            // by construction. Bound it instead — a task whose upstream work has
            // been redone in this run must not be served from a record produced
            // before that redo.
            if self.hash_free_reuse_stale(&record) {
                continue;
            }
            if best
                .as_ref()
                .is_none_or(|current| record.attempt >= current.attempt)
            {
                best = Some(record);
            }
        }
        Ok(best)
    }

    pub(crate) async fn execute(
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
        poll_v2_run_control(
            &self.runner.workflow_store,
            &self.runner.run_id,
            &execution.call.id,
        )?;
        if let Some(record) = self.runner.v2_store.load_call_record(&execution.call.id)? {
            // Restart/resume from a task: a call whose tasks are ALL already
            // recorded complete must be reused directly — including its
            // verification — without re-checking scaffold/input hashes. A
            // re-authored script or a fresh verifier input would otherwise
            // fail the hash match and force every prior task to re-validate
            // from the top, which is exactly what `restart task <id>` must not
            // do. Only accepted/noop, non-invalidated, still-valid records for
            // tasks in the completed set qualify.
            //
            // The hash is deliberately still not consulted — a re-authored
            // script legitimately changes the input of a call whose task is
            // already done — but the waiver is BOUNDED to work this run has not
            // touched: once an upstream task has re-executed here, a record for
            // anything downstream of it is stale and must fall through to the
            // content-keyed paths below. Restarting at task 080 still skips
            // 010-079; it no longer replays a record for 090 after 080 (which
            // 090 depends on) produced different output.
            if record_tasks_all_completed(&record, &self.runner.resume_completed_ids)
                && !self.hash_free_reuse_stale(&record)
                && is_reusable_status(record.status)
                && record.invalidated_by.is_none()
                && record.result.validate().is_ok()
                && reusable_record_has_required_completion_evidence(&record)
            {
                self.mark_reused(&record).await?;
                return result_view_json(&record.result);
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
                && reusable_record_has_required_completion_evidence(&record)
            {
                self.runner
                    .client
                    .tui_tx
                    .send_async(TuiEvent::TextDelta(format!(
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
                poll_v2_run_control(
                    &self.runner.workflow_store,
                    &self.runner.run_id,
                    &execution.call.id,
                )?;
                self.mark_reused(&record).await?;
                return result_view_json(&record.result);
            }
        }

        // Ordinal-drift resilience: v3 call ids embed a global ordinal that
        // shifts across re-runs when reused tasks skip their remediation loops,
        // so a completed task's call arrives under a NEW id with no record at
        // `execution.call.id`. When the call belongs to a task already in the
        // completed set, reuse that task's accepted record of the same kind
        // (implement vs verify) regardless of the ordinal — this is what makes
        // `restart`/continue actually skip 010–079 instead of re-validating.
        if let Some(record) = self.reusable_completed_task_record(&execution)? {
            self.mark_reused(&record).await?;
            return result_view_json(&record.result);
        }

        self.runner
            .client
            .tui_tx
            .send_async(TuiEvent::TextDelta(format!(
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
                    WorkflowError::ControlPaused(_)
                        | WorkflowError::ControlCancelled(_)
                        | WorkflowError::NotificationDelivery(_)
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
        // This call did real work, so anything downstream of the tasks it
        // speaks for can no longer be reused without a content match.
        self.mark_tasks_reexecuted(&record);
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

}
