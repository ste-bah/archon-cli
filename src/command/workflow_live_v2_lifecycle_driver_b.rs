// LifecycleDriver: final report and terminal handling.
//
// One of three inherent `impl LifecycleDriver` blocks split out of
// `workflow_live_v2_lifecycle.rs` to hold the 500-line ceiling.

impl LifecycleDriver {
    /// Terminal stop. The host raises the terminal marker for every
    /// non-accepted final report, which unwinds the lifecycle exactly as the
    /// JS throw did; an accepted report returns normally.
    pub(super) async fn final_report(
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
        let result = self.call(
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

    pub(super) async fn run(&self) -> archon_workflow::WorkflowResult<()> {
        loop {
            match self.run_once().await {
                Err(error) if is_terminal_gate_reroute(&error) => continue,
                result => return result,
            }
        }
    }

    pub(super) async fn run_once(&self) -> archon_workflow::WorkflowResult<()> {
        let discovery_items = self.discovery_items();
        let discovery = self
            .parallel(
                "initial-readonly-discovery",
                serde_json::Value::Array(discovery_items),
                serde_json::json!({ "tier": "analysis", "task": prompts::DISCOVERY_TASK }),
            )
            .await?;

        let raw_inventory = self
            .reduce(
                "canonical-implementation-inventory",
                serde_json::json!([
                    self.task_universe,
                    discovery,
                    self.governed_learning_context
                ]),
                "reducer",
                prompts::CANONICAL_INVENTORY_TASK,
            )
            .await?;

        let mut evidence = LifecycleEvidence::default();
        let inventory = self
            .repair_inventory(raw_inventory, &discovery, &mut evidence)
            .await?;

        let contract = self.contract();
        let malformed: Vec<serde_json::Value> = support::array(inventory.get("items"))
            .into_iter()
            .filter(|item| !support::valid_inventory_item(&contract, item))
            .collect();
        if !malformed.is_empty() || support::inventory_has_issues(&inventory) {
            return self
                .final_report(
                    "blocked-malformed-inventory",
                    None,
                    "needs_review",
                    serde_json::json!({
                        "taskUniverse": self.task_universe,
                        "inventory": inventory,
                        "malformedInventoryItems": malformed,
                        "repair_attempts": evidence.repair_attempts,
                    }),
                    prompts::BLOCKED_MALFORMED_INVENTORY_TASK,
                )
                .await;
        }
        if support::array(inventory.get("items")).is_empty() {
            return self
                .final_report(
                    "blocked-empty-implementation-inventory",
                    None,
                    "needs_review",
                    serde_json::json!({
                        "taskUniverse": self.task_universe,
                        "discovery": discovery,
                        "inventory": inventory,
                        "repair_attempts": evidence.repair_attempts,
                    }),
                    prompts::BLOCKED_EMPTY_INVENTORY_TASK,
                )
                .await;
        }

        let inventory = loop {
            match self
                .run_dependency_waves(inventory.clone(), &discovery, &mut evidence)
                .await
            {
                Ok(inventory) => break inventory,
                Err(error) if is_terminal_gate_reroute(&error) => continue,
                Err(error) => return Err(error),
            }
        };
        loop {
            match self
                .run_review_and_final_gates(&inventory, &mut evidence)
                .await
            {
                Err(error) if is_terminal_gate_reroute(&error) => continue,
                result => return result,
            }
        }
    }
}
