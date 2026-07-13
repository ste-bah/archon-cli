// Bounded repair for mechanical failures produced by verification-remediation writes.

impl LifecycleDriver {
    #[allow(clippy::too_many_arguments)]
    async fn run_verification_remediation_wave(
        &self,
        ready_items: &[serde_json::Value],
        remediation_inventory: &serde_json::Value,
        wave_index: usize,
        dependency_iteration: usize,
        remediation_attempt: &usize,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<serde_json::Value> {
        let items = workflow_live_v2_lifecycle_verify_merge::verification_remediation_source_items(
            remediation_inventory,
        );
        let call_id = verification_remediation_wave_id(wave_index, remediation_attempt, None);
        let wave = self
            .write_fanout(
                &call_id,
                serde_json::json!(items),
                prompts::VERIFICATION_REMEDIATION_WAVE_TASK,
            )
            .await?;
        record_verification_remediation_wave(
            evidence, wave_index, dependency_iteration, remediation_attempt,
            "verification-remediation", remediation_inventory, &wave,
        );
        let wave = self
            .repair_verification_remediation_outcomes(
                ready_items, remediation_inventory, wave, wave_index,
                dependency_iteration, remediation_attempt, evidence,
            )
            .await?;
        record_unresolved_verification_remediation(
            remediation_attempt, wave_index, evidence, &wave,
        );
        Ok(wave)
    }

    #[allow(clippy::too_many_arguments)]
    async fn repair_verification_remediation_outcomes(
        &self,
        ready_items: &[serde_json::Value],
        initial_inventory: &serde_json::Value,
        mut wave: serde_json::Value,
        wave_index: usize,
        dependency_iteration: usize,
        remediation_attempt: &usize,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<serde_json::Value> {
        let contract = self.contract();
        let allowed = remediation::remediation_task_id_set(
            &contract,
            &support::array(initial_inventory.get("items")),
        );
        let mut inventory = initial_inventory.clone();
        for repair_attempt in 1..=self.max_repair_iterations {
            let repairable = workflow_live_v2_lifecycle_verify_outcome_repair::repairable_contract_outcomes(&wave);
            if repairable.is_empty() {
                break;
            }
            let followup = self
                .verification_remediation_followup_inventory(
                    ready_items, &inventory, &wave, &repairable, &allowed,
                    wave_index, remediation_attempt, repair_attempt, evidence,
                )
                .await?;
            if !remediation::remediation_inventory_ready(&followup) {
                break;
            }
            wave = self
                .run_verification_remediation_followup(
                    &followup, wave, wave_index, dependency_iteration,
                    remediation_attempt, repair_attempt, evidence,
                )
                .await?;
            inventory = followup;
        }
        Ok(wave)
    }

    #[allow(clippy::too_many_arguments)]
    async fn verification_remediation_followup_inventory(
        &self,
        ready_items: &[serde_json::Value],
        source_inventory: &serde_json::Value,
        wave: &serde_json::Value,
        repairable: &[serde_json::Value],
        allowed: &std::collections::BTreeSet<String>,
        wave_index: usize,
        remediation_attempt: &usize,
        repair_attempt: usize,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<serde_json::Value> {
        let call_id = format!(
            "verification-remediation-outcome-repair-{wave_index}-{remediation_attempt}-{repair_attempt}"
        );
        let source_items = support::array(source_inventory.get("items"));
        let raw = self
            .reduce(
                &call_id,
                serde_json::json!([self.task_universe, ready_items, source_items, wave, repairable]),
                "reducer",
                prompts::REMEDIATION_OUTCOME_REPAIR_TASK,
            )
            .await?;
        support::record_repair_attempt(
            &mut evidence.repair_attempts,
            &call_id,
            "verification_remediation_outcome_repair",
            repairable,
            &raw,
        );
        let source_call_id = verification_remediation_wave_id(
            wave_index,
            remediation_attempt,
            (repair_attempt > 1).then_some(repair_attempt - 1),
        );
        let normalized = remediation::normalize_remediation_inventory_for_sources(
            &self.contract(), &raw, &source_items, ready_items, &source_call_id,
        );
        Ok(remediation::filter_remediation_inventory_by_task_ids(
            &self.contract(), &normalized, allowed,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_verification_remediation_followup(
        &self,
        inventory: &serde_json::Value,
        wave: serde_json::Value,
        wave_index: usize,
        dependency_iteration: usize,
        remediation_attempt: &usize,
        repair_attempt: usize,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<serde_json::Value> {
        let items = workflow_live_v2_lifecycle_verify_merge::verification_remediation_source_items(
            inventory,
        );
        let call_id = verification_remediation_wave_id(
            wave_index, remediation_attempt, Some(repair_attempt),
        );
        let followup = self
            .write_fanout(
                &call_id,
                serde_json::json!(&items),
                prompts::FOLLOWUP_REMEDIATION_WAVE_TASK,
            )
            .await?;
        record_verification_remediation_wave(
            evidence, wave_index, dependency_iteration, remediation_attempt,
            "verification-remediation-retry", inventory, &followup,
        );
        Ok(workflow_live_v2_lifecycle_verify_outcome_repair::merge_repaired_outcomes(
            &wave, followup, &items,
        ))
    }
}

fn verification_remediation_wave_id(
    wave_index: usize,
    remediation_attempt: &usize,
    repair_attempt: Option<usize>,
) -> String {
    let base = format!("remediation-wave-{wave_index}-verification-{remediation_attempt}");
    repair_attempt.map_or(base.clone(), |attempt| format!("{base}-repair-{attempt}"))
}

#[allow(clippy::too_many_arguments)]
fn record_verification_remediation_wave(
    evidence: &mut LifecycleEvidence,
    wave_index: usize,
    dependency_iteration: usize,
    remediation_attempt: &usize,
    kind: &str,
    inventory: &serde_json::Value,
    result: &serde_json::Value,
) {
    evidence.implementation.push(serde_json::json!({
        "kind": kind,
        "implementationWaveIndex": wave_index,
        "dependencyIteration": dependency_iteration,
        "verificationRemediationAttempt": remediation_attempt,
        "verificationRemediationInventory": inventory,
        "result": result,
    }));
}
