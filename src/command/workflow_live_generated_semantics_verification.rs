use archon_workflow::WorkflowV2HostCall;

pub(super) fn validate_verification_remediation_lifecycle(
    compact: &str,
    calls: &[WorkflowV2HostCall],
) -> archon_workflow::WorkflowResult<()> {
    require(
        has_verification_failure_triage(compact),
        "generated decomposed PRD workflow must triage failed focused verification before retrying, remediating, or blocking",
    )?;
    require(
        has_verification_remediation_inventory(compact),
        "generated decomposed PRD workflow must create remediation inventory from actionable focused verification failures",
    )?;
    require(
        has_verification_remediation_wave(compact, calls),
        "generated decomposed PRD workflow must run write-capable remediation for actionable focused verification failures",
    )?;
    require(
        has_post_remediation_verification(compact),
        "generated decomposed PRD workflow must re-verify after verification-driven write remediation before unblocking dependents",
    )?;
    require(
        normalizes_verification_sources(compact),
        "generated decomposed PRD workflow must normalize whole verification reducer results before dynamic source scheduling",
    )?;
    require(
        blocked_after_triage_and_remediation(compact),
        "generated decomposed PRD workflow must not run blocked-verification-failed before verification triage and write remediation are exhausted",
    )?;
    Ok(())
}

fn has_verification_failure_triage(compact: &str) -> bool {
    compact.contains(
        "w.reduce(\"verification-failure-triage-\"+currentImplementationWaveIndex+\"-\"+verificationRepairAttempt",
    ) || compact.contains(
        "w.reduce('verification-failure-triage-'+currentImplementationWaveIndex+'-'+verificationRepairAttempt",
    )
}

fn has_verification_remediation_inventory(compact: &str) -> bool {
    compact.contains("actionableVerificationFailures")
        && (compact.contains(
            "w.reduce(\"verification-remediation-inventory-\"+currentImplementationWaveIndex+\"-\"+verificationRemediationAttempt",
        ) || compact.contains(
            "w.reduce('verification-remediation-inventory-'+currentImplementationWaveIndex+'-'+verificationRemediationAttempt",
        ))
}

fn has_verification_remediation_wave(compact: &str, calls: &[WorkflowV2HostCall]) -> bool {
    let _ = calls;
    let source_has_call = compact.contains(
        "w.fanout(\"remediation-wave-\"+currentImplementationWaveIndex+\"-verification-\"+verificationRemediationAttempt,verificationRemediationInventory.items",
    ) || compact.contains(
        "w.fanout('remediation-wave-'+currentImplementationWaveIndex+'-verification-'+verificationRemediationAttempt,verificationRemediationInventory.items",
    );
    source_has_call
}

fn has_post_remediation_verification(compact: &str) -> bool {
    compact.contains(
        "w.reduce(\"post-remediation-verification-plan-\"+currentImplementationWaveIndex+\"-\"+verificationRemediationAttempt",
    ) && compact.contains(
        "w.parallel(\"verification-wave-\"+currentImplementationWaveIndex+\"-post-remediation-\"+verificationRemediationAttempt",
    )
}

fn normalizes_verification_sources(compact: &str) -> bool {
    compact.contains("verificationPlan=normalizeGeneratedInventory(verificationPlan)")
        && compact.contains("verificationRemediationInventory=normalizeRemediationInventory(verificationRemediationInventory)")
        && compact.contains("remediationInventoryReady(verificationRemediationInventory)")
        && compact.contains(
            "postRemediationVerificationPlan=normalizeGeneratedInventory(postRemediationVerificationPlan)",
        )
        && compact.contains(
            "verificationRepairInventory=normalizeGeneratedInventory(verificationRepairPlan)",
        )
        && compact.contains("generatedContractVerificationInventoryReady(verificationRepairInventory)")
        && compact.contains("verificationRepairPlan.items=generatedContractVerificationItems(verificationRepairInventory)")
        && !compact.contains("verificationRepairPlan.retry_items")
}

fn blocked_after_triage_and_remediation(compact: &str) -> bool {
    occurs_before(
        compact,
        "verificationFailureTriage=awaitw.reduce(\"verification-failure-triage-\"",
        "w.finalReport(\"blocked-verification-failed-\"+currentImplementationWaveIndex",
    ) && occurs_before(
        compact,
        "verificationRemediationWave=awaitw.fanout(\"remediation-wave-\"",
        "w.finalReport(\"blocked-verification-failed-\"+currentImplementationWaveIndex",
    )
}

fn occurs_before(haystack: &str, first: &str, second: &str) -> bool {
    let Some(first_index) = haystack.find(first) else {
        return false;
    };
    let Some(second_index) = haystack.find(second) else {
        return false;
    };
    first_index < second_index
}

fn require(ok: bool, message: &str) -> archon_workflow::WorkflowResult<()> {
    if ok {
        Ok(())
    } else {
        Err(archon_workflow::WorkflowError::SpecInvalid(
            message.to_string(),
        ))
    }
}
