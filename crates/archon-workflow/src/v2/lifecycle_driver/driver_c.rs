// LifecycleDriver: discovery and inventory items.
//
// One of three inherent `impl LifecycleDriver` blocks split out of
// `lifecycle_driver.rs` to hold the 500-line ceiling.

use super::*;

impl LifecycleDriver {
    pub(crate) fn discovery_items(&self) -> Vec<serde_json::Value> {
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
    pub async fn repair_inventory(
        &self,
        raw_inventory: serde_json::Value,
        discovery: &serde_json::Value,
        evidence: &mut LifecycleEvidence,
    ) -> crate::WorkflowResult<serde_json::Value> {
        let contract = self.contract();
        // Fill declared deliverables in from the universe BEFORE validation, so
        // the repair loop is never asked to reconstruct paths the host already
        // parsed. Six rounds failed to do that in one run and blocked it.
        let seeded = crate::v2::inventory_artifact_seeding::seed_artifact_requirements(
            &self.universe,
            &raw_inventory,
        );
        let mut inventory = contract.normalize_inventory(&seeded);
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
                // Kept for the corrective re-ask, which must see exactly what
                // the first attempt saw.
                let retry_source = source.clone();
                let repair = self.reduce(&call_id, source, tier, task).await?;
                support::record_repair_attempt(
                    &mut evidence.repair_attempts,
                    &call_id,
                    kind,
                    &issues,
                    &repair,
                );
                inventory = self
                    .adopt_inventory_repair(
                        &contract,
                        kind,
                        &call_id,
                        &retry_source,
                        task,
                        inventory,
                        repair,
                        issues.len(),
                        evidence,
                    )
                    .await?;
            }
            attempt += 1;
        }
        Ok(inventory)
    }

    /// Adopt a repair only if it resolves at least one issue of its kind, and
    /// tell the reducer when it did not.
    ///
    /// Every repair used to be adopted unconditionally and the next attempt
    /// re-asked in the same words, so a confident non-fix cost a full round and
    /// taught nobody anything. Observed live: `artifact-requirements-discovery`
    /// ran six times, each returning `accepted` with "repaired artifact
    /// requirements for seven items", and the issue count stayed at seven every
    /// round until the cap blocked the run. The same stage ended the same way in
    /// an earlier run on a different agent, so this is the loop's shape, not one
    /// agent's mistake.
    ///
    /// A repair that reduces nothing is not a repair. It is discarded, the
    /// pre-repair inventory stands, and one corrective re-ask names what
    /// survived — the same contract `enforce_triage_accounting` applies to
    /// shape repairs.
    #[allow(clippy::too_many_arguments)]
    async fn adopt_inventory_repair(
        &self,
        contract: &LifecycleContract<'_>,
        kind: &str,
        call_id: &str,
        retry_source: &serde_json::Value,
        task: &str,
        inventory: serde_json::Value,
        repair: serde_json::Value,
        before: usize,
        evidence: &mut LifecycleEvidence,
    ) -> crate::WorkflowResult<serde_json::Value> {
        let merged = contract.normalize_inventory(&support::merge_inventory_repair(
            contract, &inventory, &repair,
        ));
        if support::issues_of_kind(&merged, kind).len() < before {
            return Ok(merged);
        }
        let retry_id = format!("{call_id}-unresolved-retry");
        let remaining = support::issues_of_kind(&inventory, kind);
        let corrective = self
            .reduce(
                &retry_id,
                serde_json::json!([retry_source, remaining, INVENTORY_REPAIR_UNRESOLVED_NOTICE]),
                "reducer",
                task,
            )
            .await?;
        support::record_repair_attempt(
            &mut evidence.repair_attempts,
            &retry_id,
            kind,
            &remaining,
            &corrective,
        );
        let corrected = contract.normalize_inventory(&support::merge_inventory_repair(
            contract,
            &inventory,
            &corrective,
        ));
        if support::issues_of_kind(&corrected, kind).len() < before {
            Ok(corrected)
        } else {
            // Neither attempt moved it. Keep the pre-repair inventory so the
            // terminal report names the ORIGINAL issues rather than whatever a
            // non-fix rewrote them into.
            Ok(inventory)
        }
    }
}

/// Told to the reducer only after its own repair resolved nothing.
const INVENTORY_REPAIR_UNRESOLVED_NOTICE: &str = "Your previous repair for this issue kind resolved NONE of the listed issues — the host re-validated the merged inventory and found the same count. Do not restate the same answer. For each issue below, satisfy the stated rule literally: if a field must contain concrete values, supply concrete values; emptying it, moving its contents elsewhere, or asserting it is now compliant does not resolve the issue. If an issue genuinely cannot be resolved from the supplied evidence, return that item unchanged and say which evidence is missing.";
