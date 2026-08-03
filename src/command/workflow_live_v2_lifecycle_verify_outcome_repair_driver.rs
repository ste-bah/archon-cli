// Bounded repair for mechanical failures produced by verification-remediation writes.

use super::*;

impl LifecycleDriver {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn run_verification_remediation_wave(
        &self,
        ready_items: &[serde_json::Value],
        remediation_inventory: &serde_json::Value,
        wave_index: usize,
        dependency_iteration: usize,
        remediation_attempt: &usize,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<serde_json::Value> {
        let items = lifecycle_policy::verify_merge::verification_remediation_source_items(
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
            evidence,
            wave_index,
            dependency_iteration,
            remediation_attempt,
            "verification-remediation",
            remediation_inventory,
            &wave,
        );
        let wave = self
            .repair_verification_remediation_outcomes(
                ready_items,
                remediation_inventory,
                wave,
                wave_index,
                dependency_iteration,
                remediation_attempt,
                evidence,
            )
            .await?;
        record_unresolved_verification_remediation(
            remediation_attempt,
            wave_index,
            evidence,
            &wave,
        );
        Ok(wave)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn repair_verification_remediation_outcomes(
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
        let mut noop_disagreement_streak = 0usize;
        for repair_attempt in 1..=self.max_repair_iterations {
            let repairable =
                lifecycle_policy::verify_outcome_repair::repairable_contract_outcomes(&wave);
            if repairable.is_empty() {
                break;
            }
            let followup_inventory = self
                .verification_remediation_followup_inventory(
                    ready_items,
                    &inventory,
                    &wave,
                    &repairable,
                    &allowed,
                    wave_index,
                    remediation_attempt,
                    repair_attempt,
                    evidence,
                )
                .await?;
            if !remediation::remediation_inventory_ready(&followup_inventory) {
                break;
            }
            let before = wave.clone();
            let (next_wave, followup_wave) = self
                .run_verification_remediation_followup(
                    &followup_inventory,
                    wave,
                    wave_index,
                    dependency_iteration,
                    remediation_attempt,
                    repair_attempt,
                    evidence,
                )
                .await?;
            noop_disagreement_streak =
                lifecycle_policy::verify_outcome_repair::next_noop_disagreement_streak(
                    noop_disagreement_streak,
                    &before,
                    &next_wave,
                    &followup_wave,
                );
            wave = next_wave;
            if noop_disagreement_streak >= 2 {
                wave = lifecycle_policy::verify_outcome_repair::mark_noop_disagreement(&wave);
                break;
            }
            inventory = followup_inventory;
        }
        Ok(wave)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn verification_remediation_followup_inventory(
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
                serde_json::json!([
                    self.task_universe,
                    ready_items,
                    source_items,
                    wave,
                    repairable
                ]),
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
        self.enforce_outcome_repair_accounting(
            &call_id,
            raw,
            repairable,
            &source_items,
            ready_items,
            &source_call_id,
            allowed,
            evidence,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn run_verification_remediation_followup(
        &self,
        inventory: &serde_json::Value,
        wave: serde_json::Value,
        wave_index: usize,
        dependency_iteration: usize,
        remediation_attempt: &usize,
        repair_attempt: usize,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<(serde_json::Value, serde_json::Value)> {
        let items =
            lifecycle_policy::verify_merge::verification_remediation_source_items(inventory);
        let call_id =
            verification_remediation_wave_id(wave_index, remediation_attempt, Some(repair_attempt));
        let followup = self
            .write_fanout(
                &call_id,
                serde_json::json!(&items),
                prompts::FOLLOWUP_REMEDIATION_WAVE_TASK,
            )
            .await?;
        record_verification_remediation_wave(
            evidence,
            wave_index,
            dependency_iteration,
            remediation_attempt,
            "verification-remediation-retry",
            inventory,
            &followup,
        );
        let merged = lifecycle_policy::verify_outcome_repair::merge_repaired_outcomes(
            &wave,
            followup.clone(),
            &items,
        );
        Ok((merged, followup))
    }
}

pub(super) fn verification_remediation_wave_id(
    wave_index: usize,
    remediation_attempt: &usize,
    repair_attempt: Option<usize>,
) -> String {
    let base = format!("remediation-wave-{wave_index}-verification-{remediation_attempt}");
    repair_attempt.map_or(base.clone(), |attempt| format!("{base}-repair-{attempt}"))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn record_verification_remediation_wave(
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
