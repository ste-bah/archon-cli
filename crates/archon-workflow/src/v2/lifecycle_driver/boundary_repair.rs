use super::*;

impl LifecycleDriver {
    /// D78: persist a monitor-visible typed record whenever the host rejects
    /// an LLM repair for semantic-preservation violations. Rejections
    /// otherwise live only in in-memory repair-attempt evidence until the
    /// terminal report, leaving external observers unable to distinguish a
    /// rejected repair's raw envelope from adopted state.
    pub(crate) async fn record_preservation_rejection(
        &self,
        repair_id: &str,
        violations: &[String],
    ) -> crate::WorkflowResult<()> {
        self.call(
            "checkpoint",
            &format!("{repair_id}-semantic-preservation-rejected"),
            Some(serde_json::json!({
                "repair_call_id": repair_id,
                "adopted": false,
                "violations": violations,
            })),
            serde_json::json!({
                "task": "Record host-side semantic-preservation rejection evidence."
            }),
        )
        .await
        .map(|_| ())
    }

    /// One corrective re-ask after a preservation rejection.
    ///
    /// A rejected repair costs a full round: the original triage stays
    /// authoritative, its routes get retried, and the next round asks the same
    /// reducer the same question. Observed twice in one run — 20 violations
    /// across 6 items, the same four fields dropped both times — while the
    /// repair prompt already stated the preservation contract verbatim. The
    /// reducer never sees the rejection, so it cannot learn from it.
    ///
    /// This shows it the exact violations. Adopted only if the corrected
    /// attempt preserves identity AND accounts for more outcomes than the
    /// pre-repair triage — the same bar the first attempt had to clear, so a
    /// second failure costs one call and changes nothing.
    pub(crate) async fn preservation_corrected_repair(
        &self,
        repair_id: &str,
        violations: &[String],
        unaccounted: &[serde_json::Value],
        triage: &serde_json::Value,
        failed_outcomes: &[serde_json::Value],
        evidence: &mut LifecycleEvidence,
    ) -> crate::WorkflowResult<Option<serde_json::Value>> {
        let retry_id = format!("{repair_id}-preservation-retry");
        let corrected = self
            .reduce(
                &retry_id,
                serde_json::json!([violations, unaccounted, triage]),
                "reducer",
                prompts::VERIFICATION_TRIAGE_PRESERVATION_RETRY_TASK,
            )
            .await?;
        let corrected = lifecycle_policy::verify_routing::harvest_nested_triage_routes(&corrected);
        support::record_repair_attempt(
            &mut evidence.repair_attempts,
            &retry_id,
            "verification_triage_preservation_retry",
            &semantic_preservation::violation_issues(violations),
            &corrected,
        );
        let preservation = semantic_preservation::check_items(
            &semantic_preservation::canonical_route_entries(triage),
            &semantic_preservation::canonical_route_entries(&corrected),
        );
        if !preservation.passed() {
            self.record_preservation_rejection(&retry_id, &preservation.violations)
                .await?;
            return Ok(None);
        }
        let before =
            lifecycle_policy::verify_routing::unaccounted_failed_outcomes(triage, failed_outcomes);
        let after = lifecycle_policy::verify_routing::unaccounted_failed_outcomes(
            &corrected,
            failed_outcomes,
        );
        Ok((after.len() < before.len()).then_some(corrected))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn enforce_outcome_repair_accounting(
        &self,
        call_id: &str,
        raw: serde_json::Value,
        failed_outcomes: &[serde_json::Value],
        source_items: &[serde_json::Value],
        fallback_items: &[serde_json::Value],
        source_call_id: &str,
        allowed_task_ids: &std::collections::BTreeSet<String>,
        evidence: &mut LifecycleEvidence,
    ) -> crate::WorkflowResult<serde_json::Value> {
        let normalize = |value: &serde_json::Value| {
            let harvested = lifecycle_policy::boundary_repair::harvest_outcome_repair_items(value);
            remediation::filter_remediation_inventory_by_task_ids(
                &self.contract(),
                &remediation::normalize_remediation_inventory_for_sources(
                    &self.contract(),
                    &harvested,
                    source_items,
                    fallback_items,
                    source_call_id,
                ),
                allowed_task_ids,
            )
        };
        let inventory = normalize(&raw);
        let quality =
            lifecycle_policy::boundary_repair::outcome_repair_quality(&inventory, failed_outcomes);
        if remediation::remediation_inventory_ready(&inventory)
            && quality
                == (lifecycle_policy::boundary_repair::OutcomeRepairQuality {
                    unaccounted: 0,
                    unresolved_issues: 0,
                    empty_inventory: 0,
                })
        {
            return Ok(inventory);
        }

        let repair_id = format!("{call_id}-shape-repair-1");
        let repair = self
            .reduce(
                &repair_id,
                serde_json::json!([failed_outcomes, &inventory, &raw]),
                "reducer",
                prompts::REMEDIATION_OUTCOME_SHAPE_REPAIR_TASK,
            )
            .await?;
        let repaired_inventory = normalize(&repair);
        support::record_repair_attempt(
            &mut evidence.repair_attempts,
            &repair_id,
            "remediation_outcome_shape_repair",
            failed_outcomes,
            &repaired_inventory,
        );
        let repaired_quality = lifecycle_policy::boundary_repair::outcome_repair_quality(
            &repaired_inventory,
            failed_outcomes,
        );
        // D74: structural improvement alone is not adoption — the repair must
        // also preserve the semantic identity of the items it reshaped.
        let preservation = semantic_preservation::check_items(
            &support::array(inventory.get("items")),
            &support::array(repaired_inventory.get("items")),
        );
        if !preservation.passed() {
            support::record_repair_attempt(
                &mut evidence.repair_attempts,
                &repair_id,
                "semantic_preservation_rejected",
                &semantic_preservation::violation_issues(&preservation.violations),
                &repaired_inventory,
            );
            self.record_preservation_rejection(&repair_id, &preservation.violations)
                .await?;
            return Ok(inventory);
        }
        if repaired_quality < quality {
            Ok(repaired_inventory)
        } else {
            Ok(inventory)
        }
    }

    pub(crate) async fn enforce_final_reconciliation_shape(
        &self,
        call_id: &str,
        reconciliation: serde_json::Value,
        evidence: &mut LifecycleEvidence,
    ) -> crate::WorkflowResult<serde_json::Value> {
        let reconciliation =
            lifecycle_policy::boundary_repair::harvest_reconciliation_items(&reconciliation);
        let quality = lifecycle_policy::boundary_repair::reconciliation_quality(&reconciliation);
        if quality
            == (lifecycle_policy::boundary_repair::ReconciliationQuality {
                missing_collection: 0,
                malformed_items: 0,
            })
        {
            return Ok(reconciliation);
        }

        let repair_id = format!("{call_id}-shape-repair-1");
        let repaired = self
            .reduce(
                &repair_id,
                serde_json::json!([self.task_universe, &reconciliation]),
                "reducer",
                prompts::FINAL_EVIDENCE_RECONCILIATION_SHAPE_REPAIR_TASK,
            )
            .await?;
        let repaired = lifecycle_policy::boundary_repair::harvest_reconciliation_items(&repaired);
        support::record_repair_attempt(
            &mut evidence.final_evidence_repair_attempts,
            &repair_id,
            "final_evidence_reconciliation_shape_repair",
            &lifecycle_policy::boundary_repair::collection_items(&reconciliation),
            &repaired,
        );
        let repaired_quality = lifecycle_policy::boundary_repair::reconciliation_quality(&repaired);
        // D74: reconciliation issues must survive the shape repair with their
        // identity and classification intact — dropping or reclassifying an
        // issue is how a false green would sneak past the final gates.
        let preservation = semantic_preservation::check_items(
            &lifecycle_policy::boundary_repair::collection_items(&reconciliation),
            &lifecycle_policy::boundary_repair::collection_items(&repaired),
        );
        if !preservation.passed() {
            support::record_repair_attempt(
                &mut evidence.final_evidence_repair_attempts,
                &repair_id,
                "semantic_preservation_rejected",
                &semantic_preservation::violation_issues(&preservation.violations),
                &repaired,
            );
            self.record_preservation_rejection(&repair_id, &preservation.violations)
                .await?;
            return Ok(reconciliation);
        }
        if repaired_quality.defect_count() < quality.defect_count() {
            Ok(repaired)
        } else {
            Ok(reconciliation)
        }
    }
}
