// LifecycleDriver: construction and stage dispatch.
//
// One of three inherent `impl LifecycleDriver` blocks split out of
// `workflow_live_v2_lifecycle.rs` to hold the 500-line ceiling.

use super::*;

impl LifecycleDriver {
    pub(crate) fn new(
        host: Arc<WorkflowScriptHost>,
        universe: WorkflowV2TaskUniverse,
        target_repository_root: Option<String>,
        project_artifact_root: Option<String>,
        governed_learning_context: serde_json::Value,
        resume_completed_ids: std::collections::BTreeSet<String>,
        generated_config: &archon_core::config::GeneratedWorkflowConfig,
    ) -> Self {
        let task_universe = serde_json::to_value(&universe).unwrap_or(serde_json::Value::Null);
        let canonical = universe.tasks.len();
        Self {
            host,
            task_universe,
            universe,
            target_repository_root,
            project_artifact_root,
            governed_learning_context,
            max_repair_iterations: usize::from(generated_config.max_repair_iterations.clamp(1, 8)),
            max_investigation_iterations: usize::from(
                generated_config.max_investigation_iterations.clamp(1, 8),
            ),
            max_dependency_waves: canonical.saturating_mul(3).max(1),
            runtime_state: std::sync::Mutex::new(
                workflow_live_v2_lifecycle_terminal_gate::TerminalGateState {
                    completed_ids: resume_completed_ids.clone(),
                    ..Default::default()
                },
            ),
        }
    }

    pub(crate) fn contract(&self) -> LifecycleContract<'_> {
        LifecycleContract {
            task_universe: &self.universe,
            target_repository_root: self.target_repository_root.as_deref(),
        }
    }

    /// Mirror of the JS `__archonCall` payload shape.
    pub(crate) async fn call(
        &self,
        method: &str,
        id: &str,
        source: Option<serde_json::Value>,
        options: serde_json::Value,
    ) -> archon_workflow::WorkflowResult<serde_json::Value> {
        let mut payload = serde_json::json!({ "id": id, "options": options });
        if let Some(source) = source {
            payload["source"] = source;
        }
        let json = self
            .host
            .execute(method.to_string(), payload.to_string())
            .await?;
        let value: serde_json::Value = serde_json::from_str(&json)?;
        Ok(self.contract().normalize_canonical_id_fields(&value))
    }

    pub(crate) async fn reduce(
        &self,
        id: &str,
        source: serde_json::Value,
        tier: &str,
        task: &str,
    ) -> archon_workflow::WorkflowResult<serde_json::Value> {
        let grounded_task = if id.contains("verification") {
            prompts::ground_host_manifest_schema(task)
        } else {
            task.to_string()
        };
        let max_transport_attempts = 2;
        let source = slim_reducer_source(id, &source, false);
        let mut last_transport_failure = None;
        for attempt in 1..=max_transport_attempts {
            let call_id = if attempt == 1 {
                id.to_string()
            } else {
                format!("{id}-transport-retry-{attempt}")
            };
            let attempt_source = if attempt == 1 {
                source.clone()
            } else if uses_verification_slimming(id) {
                slim_reducer_source(id, &source, true)
            } else {
                super::super::super::workflow_live_v2_data::source_pack_value(&source)
            };
            match self
                .call(
                    "reduce",
                    &call_id,
                    Some(attempt_source),
                    serde_json::json!({ "tier": tier, "task": grounded_task }),
                )
                .await
            {
                Ok(result) => {
                    if let Some(error) = transport_failure_summary(&result) {
                        last_transport_failure = Some(error);
                        continue;
                    }
                    return Ok(result);
                }
                Err(error) if is_transport_failure_text(&error.to_string()) => {
                    last_transport_failure = Some(error.to_string());
                }
                Err(error) => return Err(error),
            }
        }
        Ok(transport_failure_result(
            id,
            max_transport_attempts,
            max_transport_attempts,
            last_transport_failure
                .as_deref()
                .unwrap_or("reducer transport failed without a recorded transport error"),
        ))
    }

    pub(crate) async fn parallel(
        &self,
        id: &str,
        items: serde_json::Value,
        options: serde_json::Value,
    ) -> archon_workflow::WorkflowResult<serde_json::Value> {
        self.call("parallel", id, Some(items), options).await
    }

    pub(crate) async fn write_fanout(
        &self,
        id: &str,
        items: serde_json::Value,
        task: &str,
    ) -> archon_workflow::WorkflowResult<serde_json::Value> {
        let items = self.with_declared_task_artifacts(items);
        let source_items = support::array(Some(&items));
        let max_parallelism =
            workflow_live_v2_lifecycle_verify_options::write_wave_parallelism(&source_items);
        self.call(
            "fanout",
            id,
            Some(items),
            serde_json::json!({
                "tier": "coder",
                "itemKind": "implementation",
                "write": "worktree",
                "maxParallelism": max_parallelism,
                "targetFilesFromItem": true,
                "task": task,
            }),
        )
        .await
    }

    /// Declared artifact contract: every write-capable item carries the
    /// artifact requirements its task pack declares, so the implementing
    /// agent is always instructed to produce them.
    pub(crate) fn with_declared_task_artifacts(
        &self,
        items: serde_json::Value,
    ) -> serde_json::Value {
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
                    for declared in &task.deliverable_contracts {
                        let path = declared.artifact_path.trim();
                        let already = requirements
                            .iter()
                            .any(|entry| entry.as_str() == Some(path));
                        if !path.is_empty() && !already {
                            requirements.push(serde_json::Value::String(path.to_string()));
                        }
                    }
                    if let Some(root) = self.project_artifact_root.as_deref() {
                        for declared in &task.deliverable_contracts {
                            let value =
                                serde_json::to_value(declared).unwrap_or(serde_json::Value::Null);
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
}
