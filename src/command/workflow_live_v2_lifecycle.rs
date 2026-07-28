// Rust decomposed-PRD lifecycle driver — the faithful port of the generated
// scaffold JS (body_a/body_b + verification/ownership splices). It drives the same
// `WorkflowScriptHost::execute` entry point the QuickJS bridge used, so
// result reuse, source metadata, run control, events, and persistence behave identically.

use super::super::workflow_live_generated_lifecycle_remediation as remediation;
use super::super::workflow_live_generated_lifecycle_support as support;
use super::super::workflow_live_generated_lifecycle_support::LifecycleContract;
use self::workflow_live_v2_lifecycle_prompts as prompts;

const TERMINAL_GATE_REROUTE_MARKER: &str = "workflow terminal gate reroute:";

include!("workflow_live_v2_lifecycle_reporting.rs");

struct LifecycleDriver {
    host: Arc<WorkflowScriptHost>,
    universe: WorkflowV2TaskUniverse,
    task_universe: serde_json::Value,
    target_repository_root: Option<String>,
    project_artifact_root: Option<String>,
    governed_learning_context: serde_json::Value,
    max_repair_iterations: usize,
    max_investigation_iterations: usize,
    max_dependency_waves: usize,
    runtime_state:
        std::sync::Mutex<workflow_live_v2_lifecycle_terminal_gate::TerminalGateState>,
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
                super::workflow_live_v2_data::source_pack_value(&source)
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
            last_transport_failure.as_deref().unwrap_or(
                "reducer transport failed without a recorded transport error",
            ),
        ))
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

    async fn run(&self) -> archon_workflow::WorkflowResult<()> {
        loop {
            match self.run_once().await {
                Err(error) if is_terminal_gate_reroute(&error) => continue,
                result => return result,
            }
        }
    }

    async fn run_once(&self) -> archon_workflow::WorkflowResult<()> {
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

fn normalize_null_report_collections(value: &mut serde_json::Value) {
    const COLLECTION_FIELDS: &[&str] = &[
        "accepted_tasks",
        "actionable",
        "artifact_requirements",
        "artifacts",
        "blocked_tasks",
        "canonical_task_ids",
        "commands_run",
        "completed_ids",
        "dependency_ids",
        "evidence",
        "failed_tasks",
        "files_changed",
        "files_read",
        "focused_verification",
        "items",
        "missing_tasks",
        "noop_tasks",
        "outcomes",
        "remediation_actions",
        "repair_attempts",
        "residual_gaps",
        "retry_items",
        "review_blockers",
        "review_findings",
        "target_files",
        "task_coverage",
        "tests_run",
        "unresolved_issues",
    ];
    match value {
        serde_json::Value::Object(object) => {
            for (key, child) in object {
                if child.is_null() && COLLECTION_FIELDS.contains(&key.as_str()) {
                    *child = serde_json::Value::Array(Vec::new());
                } else {
                    normalize_null_report_collections(child);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                normalize_null_report_collections(child);
            }
        }
        _ => {}
    }
}

fn terminal_marker_requires_report_fallback(status: Option<WorkflowV2Status>) -> bool {
    status == Some(WorkflowV2Status::Failed)
}

fn is_terminal_gate_reroute(error: &WorkflowError) -> bool {
    error.to_string().contains(TERMINAL_GATE_REROUTE_MARKER)
}
