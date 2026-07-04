use super::super::workflow_live_generated_semantics_support::{occurs_before, require};

pub(super) fn validate_ownership_expansion_lifecycle(
    compact: &str,
) -> archon_workflow::WorkflowResult<()> {
    require(
        has_ownership_expansion_filter(compact),
        "generated decomposed PRD workflow must detect ownership_expansion_required branch evidence before unresolved remediation blocks",
    )?;
    require(
        has_ownership_expansion_inventory(compact),
        "generated decomposed PRD workflow must use JS-owned ownership-expansion-inventory before retrying expanded remediation",
    )?;
    require(
        has_ownership_expansion_wave(compact),
        "generated decomposed PRD workflow must run worktree-isolated remediation after JS-owned ownership expansion",
    )?;
    require(
        ownership_expansion_before_block(compact),
        "generated decomposed PRD workflow must try ownership expansion before blocked-remediation-unresolved",
    )
}

fn has_ownership_expansion_filter(compact: &str) -> bool {
    compact.contains("ownershipExpansionOutcomes=unresolvedAfterRemediation.filter")
        && compact.contains("ownership_expansion_required")
        && compact.contains("proposed_ownership_expansions")
}

fn has_ownership_expansion_inventory(compact: &str) -> bool {
    compact.contains(
        "w.reduce(\"ownership-expansion-inventory-\"+currentImplementationWaveIndex+\"-\"+remediationAttempt",
    ) || compact.contains(
        "w.reduce('ownership-expansion-inventory-'+currentImplementationWaveIndex+'-'+remediationAttempt",
    )
}

fn has_ownership_expansion_wave(compact: &str) -> bool {
    compact.contains(
        "w.fanout(\"remediation-wave-\"+currentImplementationWaveIndex+\"-ownership-\"+remediationAttempt,ownershipExpansionInventory.items",
    ) || compact.contains(
        "w.fanout('remediation-wave-'+currentImplementationWaveIndex+'-ownership-'+remediationAttempt,ownershipExpansionInventory.items",
    )
}

fn ownership_expansion_before_block(compact: &str) -> bool {
    occurs_before(
        compact,
        "w.reduce(\"ownership-expansion-inventory-\"+currentImplementationWaveIndex",
        "w.finalReport(\"blocked-remediation-unresolved-\"+currentImplementationWaveIndex",
    )
}
