use super::*;
impl LifecycleDriver {
    /// Dispatch the ready implementation items to a write-capable wave to close
    /// the completion gaps the evidence repair could only describe.
    ///
    /// Returns the canonical task ids this pass genuinely completed — empty
    /// when there is nothing to write for, or when the write wave could not
    /// close them either, which is the point at which blocking is honest.
    pub(super) async fn remediate_wave_completion_gaps(
        &self,
        ready_implementation_items: &[serde_json::Value],
        completed_ids: &std::collections::BTreeSet<String>,
        dependency_iteration: usize,
        evidence: &mut LifecycleEvidence,
    ) -> crate::WorkflowResult<Vec<String>> {
        // Dispatch only what is outstanding. `completed_ids` used to filter the
        // RETURN value alone, so a wave with three of four tasks done still
        // sent all four to a writer — agents re-opening finished work whose
        // results were then discarded as already-completed.
        let outstanding: Vec<serde_json::Value> = ready_implementation_items
            .iter()
            .filter(|item| {
                let ids = support::array(item.get("canonical_task_ids"));
                ids.is_empty()
                    || ids
                        .iter()
                        .any(|id| id.as_str().is_some_and(|id| !completed_ids.contains(id)))
            })
            .cloned()
            .collect();
        if outstanding.is_empty() {
            return Ok(Vec::new());
        }
        let contract = self.contract();
        let call_id = format!("wave-completion-remediation-{dependency_iteration}");
        let wave = self
            .write_fanout(
                &call_id,
                serde_json::json!(&outstanding),
                prompts::WAVE_COMPLETION_REMEDIATION_TASK,
            )
            .await?;
        support::record_repair_attempt(
            &mut evidence.repair_attempts,
            &call_id,
            "completion_gap_remediation",
            &outstanding,
            &wave,
        );
        evidence.implementation.push(wave.clone());
        let outcomes = crate::v2::outcome_envelope::outcomes_of(&wave);
        Ok(
            support::matching_accepted_completion_ids(&contract, &outstanding, &outcomes)
                .into_iter()
                .filter(|id| !completed_ids.contains(id))
                .collect(),
        )
    }
}

/// JS `generatedContractInventoryGraphIssues(remainingItems, completedIds)` —
/// deadlock diagnostics passed to the repair reducer.
pub(crate) fn deadlock_graph_issues(
    contract: &LifecycleContract<'_>,
    remaining_items: &[serde_json::Value],
    completed_ids: &std::collections::BTreeSet<String>,
) -> Vec<serde_json::Value> {
    let mut issues = Vec::new();
    for item in remaining_items {
        let blocked_on: Vec<String> = contract
            .dependency_ids_for(item)
            .into_iter()
            .filter(|id| !completed_ids.contains(id))
            .collect();
        if blocked_on.is_empty() {
            continue;
        }
        issues.push(serde_json::json!({
            "kind": "dependency_graph_repair",
            "field": "dependency_ids",
            "message": format!("item is blocked on incomplete dependencies: {}", blocked_on.join(", ")),
            "item_id": item.get("item_id").or_else(|| item.get("id")),
            "canonical_task_ids": support::array(item.get("canonical_task_ids")),
        }));
    }
    issues
}
