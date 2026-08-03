// LifecycleDriver: discovery and inventory items.
//
// One of three inherent `impl LifecycleDriver` blocks split out of
// `workflow_live_v2_lifecycle.rs` to hold the 500-line ceiling.

use super::*;

impl LifecycleDriver {
    pub(super) fn discovery_items(&self) -> Vec<serde_json::Value> {
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
    pub(crate) async fn repair_inventory(
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
                        serde_json::json!([self.task_universe, inventory, issues, discovery]),
                    ),
                    "target_file_discovery"
                    | "verification_requirements_discovery"
                    | "artifact_requirements_discovery"
                    | "provider_environment_discovery" => (
                        "analysis",
                        serde_json::json!([self.task_universe, inventory, issues, discovery]),
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
                inventory = contract.normalize_inventory(&support::merge_inventory_repair(
                    &contract, &inventory, &repair,
                ));
            }
            attempt += 1;
        }
        Ok(inventory)
    }
}
