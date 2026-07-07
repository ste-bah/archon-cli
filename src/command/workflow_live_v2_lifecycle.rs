// Rust decomposed-PRD lifecycle driver — the faithful port of the generated
// scaffold JS (body_a/body_b + verification/ownership splices). It drives the
// same `WorkflowScriptHost::execute` entry point the QuickJS bridge used, so
// result reuse, source metadata, run control, events, and persistence behave
// identically; only the interpreter changed.

use super::super::workflow_live_generated_lifecycle_remediation as remediation;
use super::super::workflow_live_generated_lifecycle_support as support;
use super::super::workflow_live_generated_lifecycle_support::LifecycleContract;
use self::workflow_live_v2_lifecycle_prompts as prompts;

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
            governed_learning_context,
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

struct LifecycleDriver {
    host: Arc<WorkflowScriptHost>,
    universe: WorkflowV2TaskUniverse,
    task_universe: serde_json::Value,
    target_repository_root: Option<String>,
    governed_learning_context: serde_json::Value,
    max_repair_iterations: usize,
    max_investigation_iterations: usize,
    max_dependency_waves: usize,
}

/// Mutable evidence bundles — the JS lifecycle's top-level arrays.
#[derive(Default)]
struct LifecycleEvidence {
    implementation: Vec<serde_json::Value>,
    verification: Vec<serde_json::Value>,
    review: Vec<serde_json::Value>,
    artifact: Vec<serde_json::Value>,
    repair_attempts: Vec<serde_json::Value>,
    final_evidence_repair_attempts: Vec<serde_json::Value>,
}

impl LifecycleDriver {
    fn new(
        host: Arc<WorkflowScriptHost>,
        universe: WorkflowV2TaskUniverse,
        target_repository_root: Option<String>,
        governed_learning_context: serde_json::Value,
        generated_config: &archon_core::config::GeneratedWorkflowConfig,
    ) -> Self {
        let task_universe = serde_json::to_value(&universe).unwrap_or(serde_json::Value::Null);
        let canonical = universe.tasks.len();
        Self {
            host,
            task_universe,
            universe,
            target_repository_root,
            governed_learning_context,
            max_repair_iterations: usize::from(generated_config.max_repair_iterations.clamp(1, 8)),
            max_investigation_iterations: usize::from(
                generated_config.max_investigation_iterations.clamp(1, 8),
            ),
            max_dependency_waves: canonical.saturating_mul(3).max(1),
        }
    }

    fn contract(&self) -> LifecycleContract<'_> {
        LifecycleContract {
            task_universe: &self.universe,
            target_repository_root: self.target_repository_root.as_deref(),
        }
    }

    /// Mirror of the JS `__archonCall` payload shape.
    async fn call(
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
        serde_json::from_str(&json).map_err(Into::into)
    }

    async fn reduce(
        &self,
        id: &str,
        source: serde_json::Value,
        tier: &str,
        task: &str,
    ) -> archon_workflow::WorkflowResult<serde_json::Value> {
        self.call(
            "reduce",
            id,
            Some(source),
            serde_json::json!({ "tier": tier, "task": task }),
        )
        .await
    }

    async fn parallel(
        &self,
        id: &str,
        items: serde_json::Value,
        options: serde_json::Value,
    ) -> archon_workflow::WorkflowResult<serde_json::Value> {
        self.call("parallel", id, Some(items), options).await
    }

    async fn write_fanout(
        &self,
        id: &str,
        items: serde_json::Value,
        task: &str,
    ) -> archon_workflow::WorkflowResult<serde_json::Value> {
        let items = self.with_declared_task_artifacts(items);
        self.call(
            "fanout",
            id,
            Some(items),
            serde_json::json!({
                "tier": "coder",
                "itemKind": "implementation",
                "write": "worktree",
                "maxParallelism": "configured",
                "targetFilesFromItem": true,
                "task": task,
            }),
        )
        .await
    }

    /// Declared artifact contract: every write-capable item carries the
    /// artifact requirements its task pack declares, so the implementing
    /// agent is always instructed to produce them (the wf-afae6bee fix).
    fn with_declared_task_artifacts(&self, items: serde_json::Value) -> serde_json::Value {
        let contract = self.contract();
        let enriched: Vec<serde_json::Value> = support::array(Some(&items))
            .into_iter()
            .map(|item| {
                let ids = contract.canonical_ids_for(&item);
                let mut requirements = support::array(item.get("artifact_requirements"));
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
                }
                let mut object = item.as_object().cloned().unwrap_or_default();
                object.insert(
                    "artifact_requirements".to_string(),
                    serde_json::Value::Array(requirements),
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
        inputs: serde_json::Value,
        task: &str,
    ) -> archon_workflow::WorkflowResult<()> {
        self.call(
            "finalReport",
            id,
            source,
            serde_json::json!({ "status": status, "inputs": inputs, "task": task }),
        )
        .await
        .map(|_| ())
    }

    async fn run(&self) -> archon_workflow::WorkflowResult<()> {
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

        let inventory = self
            .run_dependency_waves(inventory, &discovery, &mut evidence)
            .await?;
        self.run_review_and_final_gates(&inventory, &mut evidence)
            .await
    }

    fn discovery_items(&self) -> Vec<serde_json::Value> {
        let paths = serde_json::json!(self.universe.source_roots);
        vec![
            serde_json::json!({
                "id": "prd-task-review",
                "task": prompts::DISCOVERY_ITEM_PRD_TASK_REVIEW,
                "paths": paths,
            }),
            serde_json::json!({
                "id": "repository-implementation-audit",
                "task": prompts::DISCOVERY_ITEM_REPOSITORY_AUDIT,
                "paths": paths,
            }),
            serde_json::json!({
                "id": "acceptance-evidence-audit",
                "task": prompts::DISCOVERY_ITEM_ACCEPTANCE_AUDIT,
                "paths": paths,
            }),
        ]
    }

    /// body_a.js inventory repair loop: one pass per attempt over the issue
    /// kinds, each reduce gated by its own iteration cap.
    async fn repair_inventory(
        &self,
        raw_inventory: serde_json::Value,
        discovery: &serde_json::Value,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<serde_json::Value> {
        let contract = self.contract();
        let mut inventory = contract.normalize_inventory(&raw_inventory);
        let mut attempt = 1usize;
        let cap = self
            .max_repair_iterations
            .max(self.max_investigation_iterations);
        while !support::array(inventory.get("unresolved_issues")).is_empty() && attempt <= cap {
            let passes: [(&str, &str, &str, usize); 8] = [
                (
                    "inventory_shape_repair",
                    "inventory-shape-repair",
                    prompts::INVENTORY_SHAPE_REPAIR_TASK,
                    self.max_repair_iterations,
                ),
                (
                    "task_universe_reconcile",
                    "task-universe-reconcile",
                    prompts::TASK_UNIVERSE_RECONCILE_TASK,
                    self.max_repair_iterations,
                ),
                (
                    "dependency_graph_repair",
                    "dependency-graph-repair",
                    prompts::DEPENDENCY_GRAPH_REPAIR_TASK,
                    self.max_repair_iterations,
                ),
                (
                    "target_file_discovery",
                    "target-file-discovery",
                    prompts::TARGET_FILE_DISCOVERY_TASK,
                    self.max_investigation_iterations,
                ),
                (
                    "verification_requirements_discovery",
                    "verification-requirements-discovery",
                    prompts::VERIFICATION_REQUIREMENTS_DISCOVERY_TASK,
                    self.max_investigation_iterations,
                ),
                (
                    "artifact_requirements_discovery",
                    "artifact-requirements-discovery",
                    prompts::ARTIFACT_REQUIREMENTS_DISCOVERY_TASK,
                    self.max_investigation_iterations,
                ),
                (
                    "provider_environment_discovery",
                    "provider-environment-discovery",
                    prompts::PROVIDER_ENVIRONMENT_DISCOVERY_TASK,
                    self.max_investigation_iterations,
                ),
                (
                    "evidence_repair",
                    "evidence-repair",
                    prompts::EVIDENCE_REPAIR_TASK,
                    self.max_repair_iterations,
                ),
            ];
            for (kind, id_prefix, task, kind_cap) in passes {
                let issues = support::issues_of_kind(&inventory, kind);
                if issues.is_empty() || attempt > kind_cap {
                    continue;
                }
                let call_id = format!("{id_prefix}-{attempt}");
                // Reducers receive the same source bundles the JS passed;
                // analysis-tier passes drop the learning context like the JS.
                let (tier, source) = match kind {
                    "task_universe_reconcile" => (
                        "reducer",
                        serde_json::json!([
                            self.task_universe,
                            inventory,
                            issues,
                            discovery
                        ]),
                    ),
                    "target_file_discovery"
                    | "verification_requirements_discovery"
                    | "artifact_requirements_discovery"
                    | "provider_environment_discovery" => (
                        "analysis",
                        serde_json::json!([
                            self.task_universe,
                            inventory,
                            issues,
                            discovery
                        ]),
                    ),
                    _ => (
                        "reducer",
                        serde_json::json!([
                            self.task_universe,
                            inventory,
                            issues,
                            discovery,
                            self.governed_learning_context
                        ]),
                    ),
                };
                let repair = self.reduce(&call_id, source, tier, task).await?;
                support::record_repair_attempt(
                    &mut evidence.repair_attempts,
                    &call_id,
                    kind,
                    &issues,
                    &repair,
                );
                inventory = contract
                    .normalize_inventory(&support::merge_inventory_repair(
                        &contract, &inventory, &repair,
                    ));
            }
            attempt += 1;
        }
        Ok(inventory)
    }
}
