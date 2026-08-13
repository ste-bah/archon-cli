// Verification-failure triage and its single bounded re-triage path.

use super::*;
use crate::v2::lifecycle_policy::triage_outcomes::triage_failed_outcomes;

impl LifecycleDriver {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn run_verification_remediation(
        &self,
        ready_items: &[serde_json::Value],
        plan_items: &[serde_json::Value],
        wave_index: usize,
        dependency_iteration: usize,
        repair_attempt: usize,
        remediation_attempt: &mut usize,
        verification: &mut serde_json::Value,
        evidence: &mut LifecycleEvidence,
    ) -> crate::WorkflowResult<bool> {
        let triage_id = format!("verification-failure-triage-{wave_index}-{repair_attempt}");
        let failed_outcomes = triage_failed_outcomes(verification);
        let triage = self
            .verification_failure_triage(
                &triage_id,
                ready_items,
                plan_items,
                &failed_outcomes,
                evidence,
            )
            .await?;
        let (triage, triage_id, retry_producer) = self
            .bounded_verification_retriage(
                triage,
                &triage_id,
                plan_items,
                &failed_outcomes,
                wave_index,
                repair_attempt,
                verification,
                evidence,
            )
            .await?;
        let routes = lifecycle_policy::verify_routing::triage_routes(&triage);
        let route_plan = lifecycle_policy::verify_routing::triage_route_plan(&routes);
        let retried = if route_plan.run_retries {
            self.run_producer_retry(
                &triage,
                retry_producer,
                plan_items,
                &failed_outcomes,
                wave_index,
                dependency_iteration,
                repair_attempt,
                verification,
                evidence,
            )
            .await?
        } else {
            false
        };
        if route_plan.terminal_blocked && !retried {
            return Ok(false);
        }
        let superseded = if route_plan.try_supersede {
            lifecycle_policy::verify_supersede::try_supersede_verification(
                &self.contract(),
                verification,
                &triage,
                &triage_id,
            )
        } else {
            None
        };
        let superseded = if let Some(supersede) = superseded {
            *verification = supersede.verification;
            evidence.verification.push(supersede.record);
            true
        } else {
            false
        };
        // A retry re-runs a READ-ONLY verifier. That resolves a verification
        // -shape problem, but when the finding is about the artifact's content
        // — "revise each provider row to address exact-native capability,
        // native fetch, snapshot support..." — re-reading an unchanged file
        // returns the same verdict every time. Observed live: three rounds of
        // plan → shape-repair → re-verify against a byte-identical 53KB report,
        // each consuming a round, until the budget died. The triage had filed
        // the finding as a retry item, so the writer was never asked.
        //
        // One retry round is a fair test of "the verifier just needs to look
        // again". After that the evidence says otherwise, so the failures go to
        // write remediation instead of round four of the same read.
        let escalated: Vec<serde_json::Value> = if routes.implementation_failures.is_empty()
            && !routes.retry_items.is_empty()
            && repair_attempt > 1
        {
            routes.retry_items.clone()
        } else {
            Vec::new()
        };
        if !route_plan.run_write_remediation && escalated.is_empty() {
            return Ok(retried || superseded);
        }
        let failures = if escalated.is_empty() {
            routes.implementation_failures.clone()
        } else {
            escalated
        };
        let remediated = self
            .run_write_verification_remediation(
                ready_items,
                plan_items,
                &failures,
                wave_index,
                dependency_iteration,
                remediation_attempt,
                verification,
                evidence,
                &triage,
            )
            .await?;
        Ok(retried || superseded || remediated)
    }

    pub(crate) async fn verification_failure_triage(
        &self,
        triage_id: &str,
        ready_items: &[serde_json::Value],
        plan_items: &[serde_json::Value],
        actionable: &[serde_json::Value],
        evidence: &mut LifecycleEvidence,
    ) -> crate::WorkflowResult<serde_json::Value> {
        let triage = self
            .reduce(
                triage_id,
                serde_json::json!([
                    self.task_universe,
                    ready_items,
                    plan_items,
                    actionable,
                    evidence.implementation,
                    evidence.verification
                ]),
                "reducer",
                prompts::VERIFICATION_FAILURE_TRIAGE_TASK,
            )
            .await?;
        let triage = lifecycle_policy::verify_overreach::reroute_unplanned_raw_task_identity(
            triage, plan_items,
        );
        let triage = self
            .enforce_triage_accounting(triage_id, actionable, triage, evidence)
            .await?;
        support::record_repair_attempt(
            &mut evidence.repair_attempts,
            triage_id,
            "verification_failure_triage",
            actionable,
            &triage,
        );
        Ok(triage)
    }

    /// Triage boundary contract: harvest routes reducers nested under known
    /// containers, then require every non-accepted outcome to be accounted
    /// for in a canonical route array. Unaccounted outcomes get one bounded
    /// shape-repair re-ask; the repair is adopted only if it accounts for
    /// more of them.
    pub async fn enforce_triage_accounting(
        &self,
        triage_id: &str,
        failed_outcomes: &[serde_json::Value],
        triage: serde_json::Value,
        evidence: &mut LifecycleEvidence,
    ) -> crate::WorkflowResult<serde_json::Value> {
        let triage = lifecycle_policy::verify_routing::harvest_nested_triage_routes(&triage);
        let unaccounted =
            lifecycle_policy::verify_routing::unaccounted_failed_outcomes(&triage, failed_outcomes);
        if unaccounted.is_empty() {
            return Ok(triage);
        }
        let repair_id = format!("{triage_id}-shape-repair-1");
        let repaired = self
            .reduce(
                &repair_id,
                serde_json::json!([&unaccounted, &triage]),
                "reducer",
                prompts::VERIFICATION_TRIAGE_SHAPE_REPAIR_TASK,
            )
            .await?;
        let repaired = lifecycle_policy::verify_routing::harvest_nested_triage_routes(&repaired);
        support::record_repair_attempt(
            &mut evidence.repair_attempts,
            &repair_id,
            "verification_triage_shape_repair",
            &unaccounted,
            &repaired,
        );
        let still_unaccounted = lifecycle_policy::verify_routing::unaccounted_failed_outcomes(
            &repaired,
            failed_outcomes,
        );
        // D74: a shape repair that accounts for more outcomes is still rejected
        // when it rewrote or dropped the semantic identity of already-routed
        // entries; the original triage stays authoritative.
        let preservation = semantic_preservation::check_items(
            &semantic_preservation::canonical_route_entries(&triage),
            &semantic_preservation::canonical_route_entries(&repaired),
        );
        if !preservation.passed() {
            support::record_repair_attempt(
                &mut evidence.repair_attempts,
                &repair_id,
                "semantic_preservation_rejected",
                &semantic_preservation::violation_issues(&preservation.violations),
                &repaired,
            );
            self.record_preservation_rejection(&repair_id, &preservation.violations)
                .await?;
            // The reducer never sees its own rejection, so without this it
            // repeats the identical field loss next round. One corrective
            // re-ask, adopted only on the same terms the first attempt faced.
            let corrected = self
                .preservation_corrected_repair(
                    &repair_id,
                    &preservation.violations,
                    &unaccounted,
                    &triage,
                    failed_outcomes,
                    evidence,
                )
                .await?;
            return Ok(corrected.unwrap_or(triage));
        }
        if still_unaccounted.len() < unaccounted.len() {
            Ok(repaired)
        } else {
            Ok(triage)
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn bounded_verification_retriage(
        &self,
        triage: serde_json::Value,
        triage_id: &str,
        plan_items: &[serde_json::Value],
        actionable: &[serde_json::Value],
        wave_index: usize,
        repair_attempt: usize,
        verification: &serde_json::Value,
        evidence: &mut LifecycleEvidence,
    ) -> crate::WorkflowResult<(
        serde_json::Value,
        String,
        lifecycle_policy::verify_routing::RetryProducer,
    )> {
        if !lifecycle_policy::verify_retriage::needs_bounded_retriage(
            &self.contract(),
            verification,
            &triage,
        ) {
            return Ok((
                triage,
                triage_id.to_string(),
                lifecycle_policy::verify_routing::RetryProducer::Triage,
            ));
        }
        let id = format!("verification-failure-retriage-{wave_index}-{repair_attempt}");
        let feedback = lifecycle_policy::verify_retriage::retriage_feedback(verification, &triage);
        let retriage = self
            .reduce(
                &id,
                serde_json::json!([self.task_universe, plan_items, actionable, feedback]),
                "reducer",
                prompts::VERIFICATION_FAILURE_RETRIAGE_TASK,
            )
            .await?;
        let retriage = self
            .enforce_triage_accounting(&id, actionable, retriage, evidence)
            .await?;
        support::record_repair_attempt(
            &mut evidence.repair_attempts,
            &id,
            "verification_failure_retriage",
            actionable,
            &retriage,
        );
        Ok((
            retriage,
            id,
            lifecycle_policy::verify_routing::RetryProducer::Retriage,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn run_producer_retry(
        &self,
        producer_output: &serde_json::Value,
        producer: lifecycle_policy::verify_routing::RetryProducer,
        plan_items: &[serde_json::Value],
        source_outcomes: &[serde_json::Value],
        wave_index: usize,
        dependency_iteration: usize,
        repair_attempt: usize,
        verification: &mut serde_json::Value,
        evidence: &mut LifecycleEvidence,
    ) -> crate::WorkflowResult<bool> {
        let contract = self.contract();
        let Some(retry_items) = producer_retry_items(
            &contract,
            producer_output,
            producer,
            plan_items,
            source_outcomes,
        ) else {
            return Ok(false);
        };
        let retry_items = lifecycle_policy::verify_options::prepare_verification_items(
            retry_items,
            self.project_artifact_root.as_deref(),
            &evidence.implementation,
            &self.task_universe,
        );
        let retry_result = self
            .parallel(
                &format!(
                    "verification-wave-{wave_index}-{}-retry-{repair_attempt}",
                    producer.label()
                ),
                serde_json::json!(&retry_items),
                lifecycle_policy::verify_options::verification_options(
                    &retry_items,
                    prompts::RETRY_VERIFICATION_WAVE_TASK,
                    true,
                ),
            )
            .await?;
        *verification = lifecycle_policy::verify_merge::merge_retry_outcomes(
            verification,
            retry_result,
            &retry_items,
        );
        record_triage_retry(
            evidence,
            producer,
            wave_index,
            dependency_iteration,
            repair_attempt,
            retry_items,
            verification,
        );
        Ok(true)
    }
}

pub(crate) fn record_triage_retry(
    evidence: &mut LifecycleEvidence,
    producer: lifecycle_policy::verify_routing::RetryProducer,
    wave_index: usize,
    dependency_iteration: usize,
    repair_attempt: usize,
    retry_items: Vec<serde_json::Value>,
    verification: &serde_json::Value,
) {
    let producer_call_id = match producer {
        lifecycle_policy::verify_routing::RetryProducer::Triage => {
            format!("verification-failure-triage-{wave_index}-{repair_attempt}")
        }
        lifecycle_policy::verify_routing::RetryProducer::Retriage => {
            format!("verification-failure-retriage-{wave_index}-{repair_attempt}")
        }
        lifecycle_policy::verify_routing::RetryProducer::RepairPlan => {
            format!("verification-repair-plan-{wave_index}-{repair_attempt}")
        }
    };
    evidence.verification.push(serde_json::json!({
        "kind": "verification-triage-retry",
        "implementationWaveIndex": wave_index,
        "dependencyIteration": dependency_iteration,
        "verificationRepairAttempt": repair_attempt,
        "retryProducer": producer.label(),
        "producerCallId": producer_call_id,
        "verificationPlan": { "items": retry_items },
        "result": verification,
    }));
}
