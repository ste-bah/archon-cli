impl WorkflowV2ScriptRunner {
    /// Run the decomposed-PRD lifecycle natively. `harness_source` is the
    /// recorded scaffold (hash identity for reuse/metadata); it is NOT
    /// executed.
    pub(in super::super) async fn run_decomposed_lifecycle(
        self,
        harness_source: &str,
        governed_learning_context: serde_json::Value,
    ) -> archon_workflow::WorkflowResult<WorkflowV2ScriptSummary> {
        let harness_source = harness_source.to_string();
        tokio::task::spawn_blocking(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|err| {
                    WorkflowError::SpecInvalid(format!(
                        "decomposed lifecycle local async runtime failed: {err}"
                    ))
                })?;
            runtime.block_on(self.run_decomposed_lifecycle_on_current_thread(
                &harness_source,
                governed_learning_context,
            ))
        })
        .await
        .map_err(|err| {
            WorkflowError::SpecInvalid(format!("decomposed lifecycle task failed: {err}"))
        })?
    }

    async fn run_decomposed_lifecycle_on_current_thread(
        self,
        harness_source: &str,
        governed_learning_context: serde_json::Value,
    ) -> archon_workflow::WorkflowResult<WorkflowV2ScriptSummary> {
        let Some(task_universe) = self.task_universe.clone() else {
            return Err(WorkflowError::SpecInvalid(
                "decomposed lifecycle requires an authoritative task universe".to_string(),
            ));
        };
        let target_repository_root = self.runtime.target_repository_root.clone();
        let project_artifact_root = archon_workflow::project_artifact_context_from_v2_root(
            self.v2_store.root(),
        )
        .project_root;
        let resume_completed_ids = self.resume_completed_ids.clone();
        let generated_config = self.runtime.generated_config.clone();
        let host = Arc::new(WorkflowScriptHost {
            scaffold_hash: workflow_scaffold_hash(harness_source),
            runner: self,
            accumulator: Arc::new(Mutex::new(WorkflowScriptAccumulator::default())),
        });
        let driver = LifecycleDriver::new(
            host.clone(),
            task_universe,
            target_repository_root,
            project_artifact_root,
            governed_learning_context,
            resume_completed_ids,
            &generated_config,
        );
        match driver.run().await {
            Ok(()) => {
                let summary = host.summary().await;
                host.emit_terminal_status(summary.status);
                Ok(summary)
            }
            Err(err) => {
                if matches!(
                    err,
                    WorkflowError::ControlPaused(_) | WorkflowError::ControlCancelled(_)
                ) {
                    return Err(err);
                }
                let error = err.to_string();
                if error.contains(TERMINAL_HOST_CALL_MARKER) {
                    let summary = host.summary().await;
                    host.emit_terminal_status(summary.status);
                    return Ok(summary);
                }
                let summary = host.mark_script_failure(&error).await;
                Ok(summary)
            }
        }
    }
}

impl LifecycleDriver {
    /// Declared artifact contract: every write-capable item carries the
    /// artifact requirements its task pack declares, so the implementing
    /// agent is always instructed to produce them.
    fn with_declared_task_artifacts(&self, items: serde_json::Value) -> serde_json::Value {
        let contract = self.contract();
        let enriched: Vec<serde_json::Value> = support::array(Some(&items))
            .into_iter()
            .map(|item| {
                let ids = contract.canonical_ids_for(&item);
                let mut requirements = support::array(item.get("artifact_requirements"));
                let mut verifier_commands =
                    support::strings_of(item.get("artifact_verification_commands"));
                for task in &self.universe.tasks {
                    if !ids.contains(&task.canonical_task_id) {
                        continue;
                    }
                    for declared in &task.artifact_requirements {
                        let already = requirements
                            .iter()
                            .any(|entry| entry.as_str() == Some(declared.as_str()));
                        if !already {
                            requirements.push(serde_json::Value::String(declared.clone()));
                        }
                    }
                    if let Some(root) = self.project_artifact_root.as_deref() {
                        for declared in &task.deliverable_contracts {
                            let value = serde_json::to_value(declared)
                                .unwrap_or(serde_json::Value::Null);
                            if let Some(command) =
                                workflow_live_v2_deliverable_contract::typed_verification_command(
                                    root, &value,
                                )
                                && !verifier_commands.contains(&command)
                            {
                                verifier_commands.push(command);
                            }
                        }
                    }
                }
                let mut object = item.as_object().cloned().unwrap_or_default();
                object.insert(
                    "artifact_requirements".to_string(),
                    serde_json::Value::Array(requirements),
                );
                object.insert(
                    "artifact_verification_commands".to_string(),
                    serde_json::json!(verifier_commands),
                );
                serde_json::Value::Object(object)
            })
            .collect();
        serde_json::Value::Array(enriched)
    }

    /// Terminal stop. The host raises the terminal marker for every
    /// non-accepted final report, which unwinds the lifecycle exactly as the
    /// JS throw did; an accepted report returns normally.
    async fn final_report(
        &self,
        id: &str,
        source: Option<serde_json::Value>,
        status: &str,
        mut inputs: serde_json::Value,
        task: &str,
    ) -> archon_workflow::WorkflowResult<()> {
        if id.starts_with("blocked-") {
            // Claims remain fail-closed. Scheduling remains fail-open because
            // attempted work still faces implementation, verification, triage,
            // review, and final evidence gates before it can earn acceptance.
            let decision = {
                let mut state = self
                    .runtime_state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                workflow_live_v2_lifecycle_terminal_gate::decide(
                    &self.contract(),
                    id,
                    &inputs,
                    &mut state,
                )
            };
            if let workflow_live_v2_lifecycle_terminal_gate::TerminalGateDecision::Reroute(
                event,
            ) = decision
            {
                let reroute_count = event
                    .get("reroute_count")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(1);
                self.call(
                    "checkpoint",
                    &format!("terminal-gate-reroute-{id}-{reroute_count}"),
                    Some(event),
                    serde_json::json!({
                        "task": "Record host-side terminal-gate reroute evidence."
                    }),
                )
                .await?;
                return Err(WorkflowError::StageFailed(format!(
                    "{TERMINAL_GATE_REROUTE_MARKER} {id}"
                )));
            }
            let terminal_gate_events = self
                .runtime_state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .events
                .clone();
            if let Some(object) = inputs.as_object_mut()
                && !terminal_gate_events.is_empty()
            {
                object.insert(
                    "terminalGateEvidence".to_string(),
                    serde_json::Value::Array(terminal_gate_events),
                );
            }
        }
        normalize_null_report_collections(&mut inputs);
        let result = self
            .call(
                "finalReport",
                id,
                source,
                serde_json::json!({ "status": status, "inputs": inputs, "task": task }),
            )
            .await;
        let report_error = match result {
            Ok(_) => return Ok(()),
            Err(error) if error.to_string().contains(TERMINAL_HOST_CALL_MARKER) => {
                let recorded_status = self
                    .host
                    .runner
                    .v2_store
                    .load_call_record(id)?
                    .map(|record| record.status);
                if !terminal_marker_requires_report_fallback(recorded_status) {
                    return Err(error);
                }
                error
            }
            Err(error) => error,
        };
        let (completed_ids, terminal_gate_events) = {
            let state = self
                .runtime_state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (
                state.completed_ids.iter().cloned().collect::<Vec<_>>(),
                state.events.clone(),
            )
        };
        let required_ids = self
            .universe
            .tasks
            .iter()
            .map(|task| task.canonical_task_id.clone())
            .collect::<Vec<_>>();
        let missing_ids = required_ids
            .iter()
            .filter(|id| !completed_ids.contains(id))
            .cloned()
            .collect::<Vec<_>>();
        let fallback_id = format!("{id}-host-fallback");
        let fallback_result = serde_json::json!({
            "status": "needs_review",
            "summary": format!("host fallback report after '{id}' failed to build: {report_error}"),
            "evidence": [],
            "artifacts": [],
            "commands_run": [],
            "files_read": [],
            "files_changed": [],
            "task_coverage": [],
            "residual_gaps": [{
                "id": "final_report_construction_failed",
                "description": report_error.to_string(),
                "severity": "blocking"
            }],
            "data": {
                "original_report_id": id,
                "requested_status": status,
                "completed_task_ids": completed_ids,
                "missing_task_ids": missing_ids,
                "terminal_gate_events": terminal_gate_events,
            }
        });
        self.call(
            "finalReport",
            &fallback_id,
            None,
            serde_json::json!({
                "status": "needs_review",
                "inputs": [fallback_result],
                "task": "Emit the minimal host-built terminal report after report construction failed."
            }),
        )
        .await
        .map(|_| ())
    }
}
