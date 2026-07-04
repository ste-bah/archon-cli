use archon_workflow::{WorkflowError, WorkflowV2HostCall, WorkflowV2HostMethod};

use super::workflow_live_generated_semantics_support::*;
use super::workflow_live_generated_semantics_verification::validate_verification_remediation_lifecycle;
use super::workflow_live_task_universe::WorkflowV2TaskUniverse;

#[path = "workflow_live_generated_semantics_bindings.rs"]
mod workflow_live_generated_semantics_bindings;
#[path = "workflow_live_generated_semantics_ownership.rs"]
mod workflow_live_generated_semantics_ownership;

use workflow_live_generated_semantics_bindings::validate_const_host_call_bindings;
use workflow_live_generated_semantics_ownership::validate_ownership_expansion_lifecycle;

pub(super) fn validate_generated_workflow_semantics(
    task: &str,
    task_universe: Option<&WorkflowV2TaskUniverse>,
    source: &str,
    calls: &[WorkflowV2HostCall],
) -> archon_workflow::WorkflowResult<()> {
    reject(
        compact_js(source).contains("humanGate(")
            || calls
                .iter()
                .any(|call| call.method == WorkflowV2HostMethod::HumanGate),
        "generated workflow.js must not use w.humanGate; stop with explicit blocked/needs_review status instead",
    )?;
    if !requires_decomposed_prd_dependency_scaffold(task, source, calls) {
        return Ok(());
    }
    let task_universe = task_universe.ok_or_else(|| {
        WorkflowError::SpecInvalid(
            "generated decomposed PRD workflow requires authoritative TASK universe before scaffold validation"
                .to_string(),
        )
    })?;
    validate_dependency_wave_scaffold(source, calls, task_universe)
}

fn requires_decomposed_prd_dependency_scaffold(
    task: &str,
    source: &str,
    calls: &[WorkflowV2HostCall],
) -> bool {
    let text = format!("{task}\n{source}").to_ascii_lowercase();
    let looks_decomposed_prd = text.contains("decomposed prd")
        || text.contains("task-")
        || text.contains("dependency_ids")
        || text.contains("depends_on");
    let implementation_intent = text.contains("implement")
        || text.contains("implementation")
        || calls.iter().any(implementation_intent_call);
    looks_decomposed_prd && implementation_intent
}

fn validate_dependency_wave_scaffold(
    source: &str,
    calls: &[WorkflowV2HostCall],
    task_universe: &WorkflowV2TaskUniverse,
) -> archon_workflow::WorkflowResult<()> {
    let compact = compact_js(source);
    reject(
        compact.contains("humanGate("),
        "generated decomposed PRD workflows must not use w.humanGate; they must stop with blocked/needs_review reports and explicit resume/restart commands",
    )?;
    validate_const_host_call_bindings(source)?;
    require(
        declares_variable(&compact, "taskUniverse"),
        "generated decomposed PRD workflow must inline `taskUniverse` from authoritative TASK evidence",
    )?;
    let embedded_task_universe = extract_embedded_task_universe(source)?;
    require(
        embedded_task_universe == *task_universe,
        "generated decomposed PRD workflow must embed the exact authoritative taskUniverse JSON",
    )?;
    require(
        compact.contains(
            "canonicalTaskUniverse=newSet((taskUniverse.tasks||[]).map((task)=>task.canonical_task_id).filter(Boolean))",
        ),
        "generated decomposed PRD workflow must derive `canonicalTaskUniverse` from taskUniverse.tasks canonical_task_id values",
    )?;
    require_helper_contract(&compact)?;
    require(
        declares_variable(&compact, "implementationEvidence")
            && declares_variable(&compact, "verificationEvidence")
            && declares_variable(&compact, "reviewEvidence")
            && declares_variable(&compact, "artifactEvidence"),
        "generated decomposed PRD workflow must keep JS-owned implementation, verification, review, and artifact evidence accumulators",
    )?;
    require(
        compact
            .contains("w.reduce(\"canonical-implementation-inventory\",[taskUniverse,discovery]")
            || compact.contains("w.reduce(\"canonical-implementation-inventory\",[taskUniverse,discovery,governedLearningContext]")
            || compact
                .contains("w.reduce('canonical-implementation-inventory',[taskUniverse,discovery]")
            || compact.contains("w.reduce('canonical-implementation-inventory',[taskUniverse,discovery,governedLearningContext]"),
        "generated decomposed PRD workflow must pass taskUniverse into the implementation inventory reducer",
    )?;
    require_repair_investigation_contract(&compact)?;
    require(
        declares_variable(&compact, "remainingItems"),
        "generated decomposed PRD workflow must declare `remainingItems` for dependency-owned implementation state",
    )?;
    require(
        compact.contains("completedIds=newSet(") || compact.contains("completedIds=newSet"),
        "generated decomposed PRD workflow must declare `completedIds` as the dependency completion set",
    )?;
    require(
        declares_variable(&compact, "dependencyIteration")
            && declares_variable(&compact, "implementationWaveIndex"),
        "generated decomposed PRD workflow must declare separate `dependencyIteration` and `implementationWaveIndex` counters",
    )?;
    require(
        compact.contains("while(")
            && compact.contains("remainingItems.length>0")
            && compact.contains("dependencyIteration<="),
        "generated decomposed PRD workflow must use a bounded `while` loop over unresolved implementation items",
    )?;
    require(
        (compact.contains("readyItems=readyItemsFrom(remainingItems,completedIds)")
            && compact.contains("functionreadyItemsFrom(items,completedIds)")
            && compact.contains("dependencyIdsFor(item).every((id)=>completedIds.has(id))"))
            || (compact.contains("readyItems=remainingItems.filter(")
                && compact.contains("dependency_ids")
                && compact.contains("completedIds.has(")),
        "generated decomposed PRD workflow must compute `readyItems` from `remainingItems` using dependency_ids and completedIds",
    )?;
    require(
        compact.contains("w.reduce(\"dependency-graph-repair-\"+repairAttempt")
            || compact.contains("w.reduce('dependency-graph-repair-'+repairAttempt"),
        "generated decomposed PRD workflow must repair invalid dependency graphs before scheduling",
    )?;
    require(
        compact.contains("w.reduce(\"dependency-graph-repair-deadlock-\"+dependencyIteration")
            || compact.contains("w.reduce('dependency-graph-repair-deadlock-'+dependencyIteration"),
        "generated decomposed PRD workflow must attempt JS-owned dependency graph repair before dependency-deadlock stop",
    )?;
    reject(
        (compact.contains("readyItems.length===0")
            && (compact.contains("finalReport(\"blocked-dependency-deadlock")
                || compact.contains("finalReport('blocked-dependency-deadlock")))
            && !(occurs_before(
                &compact,
                "w.reduce(\"dependency-graph-repair-deadlock-\"+dependencyIteration",
                "finalReport(\"blocked-dependency-deadlock",
            ) || occurs_before(
                &compact,
                "w.reduce('dependency-graph-repair-deadlock-'+dependencyIteration",
                "finalReport('blocked-dependency-deadlock",
            )),
        "generated decomposed PRD workflow must not stop on dependency deadlock before bounded dependency-graph repair",
    )?;
    require(
        compact.contains("readyItems=readyItemsFrom(remainingItems,completedIds)")
            && compact.contains("dependency_ids")
            && compact.contains("completedIds.has("),
        "generated decomposed PRD workflow must use the shared readyItemsFrom dependency helper for scheduling",
    )?;
    require(
        compact.contains("workTypeFor(item)===\"verified_noop\"")
            && compact.contains("workTypeFor(item)===\"implementation\"")
            && compact.contains("readyNoopItems=readyItems.filter(")
            && compact.contains("readyImplementationItems=readyItems.filter("),
        "generated decomposed PRD workflow must split dependency-ready inventory into verified_noop and implementation work inside the bounded loop",
    )?;
    require(
        compact.contains("validVerifiedNoopItem(item)")
            && compact.contains("noop_proof")
            && compact.contains("noop_proof_refs")
            && compact.contains("validInventoryItem(item)"),
        "generated decomposed PRD workflow must validate verified_noop inventory separately from implementation inventory",
    )?;
    require(
        compact
            .contains("w.parallel(\"noop-proof-verification-\"+dependencyIteration,readyNoopItems")
            || compact.contains(
                "w.parallel('noop-proof-verification-'+dependencyIteration,readyNoopItems",
            ),
        "generated decomposed PRD workflow must verify dependency-ready no-op proof inside the dependency loop",
    )?;
    require(
        (compact.contains(
            "w.reduce(\"noop-evidence-repair-\"+dependencyIteration+\"-\"+noopRepairAttempt",
        ) || compact.contains(
            "w.reduce('noop-evidence-repair-'+dependencyIteration+'-'+noopRepairAttempt",
        )) && (compact.contains(
            "w.parallel(\"noop-proof-reverification-\"+dependencyIteration+\"-\"+noopRepairAttempt",
        ) || compact.contains(
            "w.parallel('noop-proof-reverification-'+dependencyIteration+'-'+noopRepairAttempt",
        )),
        "generated decomposed PRD workflow must repair and re-verify failed no-op proof before blocking",
    )?;
    require(
        contains_dynamic_implementation_wave_call(&compact),
        "generated decomposed PRD workflow must call `w.fanout(\"implementation-wave-\" + currentImplementationWaveIndex, readyImplementationItems, ...)`",
    )?;
    reject(
        calls
            .iter()
            .any(|call| call.method == WorkflowV2HostMethod::Implementation),
        "generated decomposed PRD workflow must not use direct one-shot w.implementation for multi-task PRD work; use dependency-ready implementation fanout waves",
    )?;
    require(
        calls.iter().any(|call| {
            implementation_write_fanout(call)
                && call_source_matches(call, "readyImplementationItems")
                && call
                    .options
                    .extra
                    .get("dynamic_id_prefix")
                    .and_then(serde_json::Value::as_str)
                    == Some("implementation-wave-")
        }),
        "generated decomposed PRD workflow implementation fanout must use dynamic id prefix `implementation-wave-`, source `readyImplementationItems`, and worktree-isolated write mode",
    )?;
    reject(
        compact.contains("(item.canonical_task_ids||[]).every"),
        "generated decomposed PRD workflow must not treat missing canonical_task_ids as completed via empty every(); require non-empty canonical IDs",
    )?;
    require(
        compact.contains("constacceptedThisWave=newSet()")
            && compact
                .contains("matchingAcceptedNoopIds(readyNoopItems,noopProof.outcomes||noopProof.items)")
            && compact.contains("implementationCandidateIds=matchingAcceptedIds(readyImplementationItems,wave.outcomes)")
            && compact.contains("implementationCandidateIds=implementationCandidateIds.concat(matchingAcceptedIds(remediationInventory.items,remediationWave.outcomes))")
            && compact.contains("letnewlyCompletedIds=Array.from(acceptedThisWave).filter((id)=>!completedIds.has(id))")
            && contains_completed_id_add_loop(&compact)
            && compact.contains("remainingItems=remainingItems.filter((item)=>!itemIsCompleted(item,completedIds))"),
        "generated decomposed PRD workflow must collect completed IDs only from verified no-op proof or implementation/remediation candidates that later pass focused verification, then shrink remainingItems only from completedIds",
    )?;
    require(
        compact.contains("w.reduce(\"verification-plan-\"+currentImplementationWaveIndex,[taskUniverse,readyImplementationItems,implementationCandidateIdsUnique,implementationEvidence]")
            || compact.contains("w.reduce('verification-plan-'+currentImplementationWaveIndex,[taskUniverse,readyImplementationItems,implementationCandidateIdsUnique,implementationEvidence]"),
        "generated decomposed PRD workflow must plan focused verification for each implementation wave before unblocking dependents",
    )?;
    require(
        compact.contains(
            "w.reduce(\"verification-plan-repair-\"+currentImplementationWaveIndex+\"-\"+verificationPlanRepairAttempt",
        ) || compact.contains(
            "w.reduce('verification-plan-repair-'+currentImplementationWaveIndex+'-'+verificationPlanRepairAttempt",
        ),
        "generated decomposed PRD workflow must repair empty focused verification plans before blocking",
    )?;
    require(
        compact.contains("w.parallel(\"verification-wave-\"+currentImplementationWaveIndex,verificationPlan.items")
            || compact.contains("w.parallel('verification-wave-'+currentImplementationWaveIndex,verificationPlan.items"),
        "generated decomposed PRD workflow must run focused verification for each implementation wave before unblocking dependents",
    )?;
    require(
        compact.contains(
            "w.reduce(\"verification-repair-plan-\"+currentImplementationWaveIndex+\"-\"+verificationRepairAttempt",
        ) || compact.contains(
            "w.reduce('verification-repair-plan-'+currentImplementationWaveIndex+'-'+verificationRepairAttempt",
        ),
        "generated decomposed PRD workflow must repair failed focused verification before blocking",
    )?;
    require(
        occurs_before(
            &compact,
            "letverification=awaitw.parallel(\"verification-wave-\"+currentImplementationWaveIndex",
            completed_id_add_loop_pattern(&compact),
        ) || occurs_before(
            &compact,
            "letverification=awaitw.parallel('verification-wave-'+currentImplementationWaveIndex",
            completed_id_add_loop_pattern(&compact),
        ),
        "generated decomposed PRD workflow must verify newly completed work before adding IDs to completedIds",
    )?;
    reject(
        occurs_before(
            &compact,
            "for(constidofnewlyCompletedIds){completedIds.add(id)",
            "letverification=awaitw.parallel(\"verification-wave-\"+currentImplementationWaveIndex",
        ) || occurs_before(
            &compact,
            "for(constidofnewlyCompletedIds){completedIds.add(id)",
            "letverification=awaitw.parallel('verification-wave-'+currentImplementationWaveIndex",
        ),
        "generated decomposed PRD workflow must not add implementation IDs to completedIds before focused verification",
    )?;
    require(
        compact.contains("canonical_task_ids") && compact.contains("evidence"),
        "generated decomposed PRD workflow must require branch outcomes with canonical_task_ids and evidence",
    )?;
    validate_verification_remediation_lifecycle(&compact, calls)?;
    require(
        compact.contains("w.reduce(\"wave-completion-evidence-repair-\"+dependencyIteration")
            || compact.contains("w.reduce('wave-completion-evidence-repair-'+dependencyIteration"),
        "generated decomposed PRD workflow must run JS-owned completion evidence repair before blocked-no-completion",
    )?;
    require(
        compact.contains("completionEvidenceRepair=normalizeGeneratedInventory(completionEvidenceRepair)")
            && compact.contains("newlyCompletedIds=matchingAcceptedCompletionIds(readyItems,completionEvidenceRepair.items||completionEvidenceRepair.outcomes)"),
        "generated decomposed PRD workflow must re-check accepted completion evidence repair before blocked-no-completion",
    )?;
    require(
        contains_remediation_inventory_from_nonaccepted_wave_outcomes(&compact, calls),
        "generated decomposed PRD workflow must route non-accepted implementation wave outcomes into remediation inventory",
    )?;
    require(
        compact.contains("functionnormalizeRemediationInventory(value)")
            && compact.contains("functionnormalizeRemediationInventoryForSources(value,sourceItems,fallbackItems,sourceCallId)")
            && compact.contains("remediationInventory=normalizeRemediationInventoryForSources(remediationInventory,readyImplementationItems,[],\"implementation-wave-\"+currentImplementationWaveIndex)")
            && compact.contains("while(!remediationInventoryReady(remediationInventory)")
            && compact.contains("remediationInventory=normalizeRemediationInventoryForSources(remediationInventoryRepair,readyImplementationItems,remediationInventory.items,\"implementation-wave-\"+currentImplementationWaveIndex)")
            && compact.contains("constremediationTaskIds=remediationTaskIdSet(remediationInventory.items)")
            && compact.contains("followupRemediationInventory=filterRemediationInventoryByTaskIds(normalizeRemediationInventoryForSources(followupRemediationInventory,remediationInventory.items,readyImplementationItems,\"remediation-wave-\"+currentImplementationWaveIndex),remediationTaskIds)")
            && compact.contains("if(!remediationInventoryReady(followupRemediationInventory)){break;}"),
        "generated decomposed PRD workflow must normalize, preserve original ownership, preflight, and constrain follow-up remediation inventories before fanout",
    )?;
    validate_ownership_expansion_lifecycle(&compact)?;
    require(
        compact.contains("w.reduce(\"remediation-empty-inventory-repair-\"+currentImplementationWaveIndex+\"-\"+remediationInventoryRepairAttempt")
            || compact.contains("w.reduce('remediation-empty-inventory-repair-'+currentImplementationWaveIndex+'-'+remediationInventoryRepairAttempt"),
        "generated decomposed PRD workflow must repair empty or malformed remediation inventory before blocking",
    )?;
    require(
        compact.contains(
            "w.reduce(\"remediation-outcome-repair-\"+currentImplementationWaveIndex+\"-\"+remediationAttempt",
        ) || compact
            .contains("w.reduce('remediation-outcome-repair-'+currentImplementationWaveIndex+'-'+remediationAttempt"),
        "generated decomposed PRD workflow must repair unresolved remediation outcomes before blocking",
    )?;
    require(
        compact.contains("source_item_id")
            && compact.contains("failure_status")
            && compact.contains("failure_evidence")
            && compact.contains("required_fix")
            && compact.contains("verification_requirements"),
        "generated decomposed PRD workflow remediation inventory must carry source_item_id, failure_status, failure_evidence, required_fix, and verification_requirements",
    )?;
    require(
        contains_dynamic_remediation_wave_call(&compact),
        "generated decomposed PRD workflow must call `w.fanout(\"remediation-wave-\" + currentImplementationWaveIndex, remediationInventory.items, ...)` for actionable remediation",
    )?;
    require(
        calls.iter().any(|call| {
            implementation_write_fanout(call)
                && call_source_matches(call, "remediationInventory.items")
                && call
                    .options
                    .extra
                    .get("dynamic_id_prefix")
                    .and_then(serde_json::Value::as_str)
                    == Some("remediation-wave-")
        }),
        "generated decomposed PRD workflow remediation fanout must use dynamic id prefix `remediation-wave-`, source `remediation.items`, and worktree-isolated write mode",
    )?;
    require(
        compact.contains("readyItems.length===0")
            && (compact.contains("finalReport(\"blocked-dependency-deadlock")
                || compact.contains("finalReport('blocked-dependency-deadlock")),
        "generated decomposed PRD workflow must stop terminally on dependency deadlock",
    )?;
    require(
        compact.contains("remainingItems.length>0")
            && (compact.contains("finalReport(\"blocked-loop-exhaustion")
                || compact.contains("finalReport('blocked-loop-exhaustion")),
        "generated decomposed PRD workflow must stop terminally on bounded-loop exhaustion with unresolved items",
    )?;
    require(
        (compact.contains("finalReport(\"blocked-review-unresolved")
            || compact.contains("finalReport('blocked-review-unresolved"))
            && (compact.contains("finalReport(\"blocked-review-not-accepted")
                || compact.contains("finalReport('blocked-review-not-accepted")),
        "generated decomposed PRD workflow must stop terminally when adversarial review remains unresolved or not accepted",
    )?;
    reject(
        compact.contains("qualityGate(\"adversarial-review-")
            || compact.contains("qualityGate('adversarial-review-"),
        "generated decomposed PRD workflow must run adversarial review through real read-only worker/reducer calls, not a local qualityGate",
    )?;
    require(
        compact.contains("w.reduce(\"adversarial-review-\"+reviewIteration,[taskUniverse,implementationEvidence,verificationEvidence,artifactEvidence]")
            || compact.contains("w.reduce(\"adversarial-review-\"+reviewIteration,[taskUniverse,implementationEvidence,verificationEvidence,artifactEvidence,governedLearningContext]")
            || compact.contains("w.reduce('adversarial-review-'+reviewIteration,[taskUniverse,implementationEvidence,verificationEvidence,artifactEvidence]")
            || compact.contains("w.reduce('adversarial-review-'+reviewIteration,[taskUniverse,implementationEvidence,verificationEvidence,artifactEvidence,governedLearningContext]"),
        "generated decomposed PRD workflow must run the first adversarial review as real read-only reduce work",
    )?;
    require(
        compact.contains("w.reduce(\"final-evidence-reconciliation-\"+finalEvidenceIteration")
            || compact.contains("w.reduce('final-evidence-reconciliation-'+finalEvidenceIteration"),
        "generated decomposed PRD workflow must run JS-owned final evidence reconciliation before final acceptance",
    )?;
    require(
        compact.contains("w.reduce(\"completion-claim-repair-\"+finalEvidenceIteration")
            || compact.contains("w.reduce('completion-claim-repair-'+finalEvidenceIteration"),
        "generated decomposed PRD workflow must repair unsupported completion claims before final acceptance",
    )?;
    require(
        compact.contains("w.parallel(\"artifact-existence-investigation-\"+finalEvidenceIteration")
            || compact
                .contains("w.parallel('artifact-existence-investigation-'+finalEvidenceIteration"),
        "generated decomposed PRD workflow must investigate referenced artifact existence before final acceptance",
    )?;
    require(
        compact.contains("w.reduce(\"final-zero-gap-audit\",[taskUniverse,inventory,implementationEvidence,verificationEvidence,reviewEvidence,artifactEvidence]")
            || compact.contains("w.reduce(\"final-zero-gap-audit\",[taskUniverse,inventory,implementationEvidence,verificationEvidence,reviewEvidence,artifactEvidence,governedLearningContext]")
            || compact.contains("w.reduce(\"final-zero-gap-audit\",[taskUniverse,inventory,implementationEvidence,verificationEvidence,reviewEvidence,artifactEvidence,governedLearningContext,repairAttempts")
            || compact.contains("w.reduce(\"final-zero-gap-audit\",[taskUniverse,inventory,implementationEvidence,verificationEvidence,reviewEvidence,artifactEvidence,requiredArtifacts")
            || compact.contains("w.reduce('final-zero-gap-audit',[taskUniverse,inventory,implementationEvidence,verificationEvidence,reviewEvidence,artifactEvidence]")
            || compact.contains("w.reduce('final-zero-gap-audit',[taskUniverse,inventory,implementationEvidence,verificationEvidence,reviewEvidence,artifactEvidence,governedLearningContext]"),
        "generated decomposed PRD workflow final audit must receive all implementation, verification, review, and artifact evidence",
    )?;
    require(
        compact.contains("w.requireArtifact(\"require-final-artifacts\",artifactInventory")
            || compact.contains("w.requireArtifact('require-final-artifacts',artifactInventory"),
        "generated decomposed PRD workflow must require final artifacts before acceptance",
    )?;
    require(
        occurs_before(
            &compact,
            "w.reduce(\"final-evidence-reconciliation-\"+finalEvidenceIteration",
            "w.requireArtifact(\"require-final-artifacts\",artifactInventory",
        ) || occurs_before(
            &compact,
            "w.reduce('final-evidence-reconciliation-'+finalEvidenceIteration",
            "w.requireArtifact('require-final-artifacts',artifactInventory",
        ),
        "generated decomposed PRD workflow must require final artifacts only after final evidence reconciliation",
    )?;
    require(
        compact.contains("w.qualityGate(\"final-acceptance-gate\",[finalAudit,requiredArtifacts,finalEvidenceReconciliation]")
            || compact.contains("w.qualityGate('final-acceptance-gate',[finalAudit,requiredArtifacts,finalEvidenceReconciliation]"),
        "generated decomposed PRD workflow final acceptance gate must receive explicit typed source results",
    )?;
    require(
        compact.contains("w.finalReport(\"final-acceptance-report\",[finalGate,finalAudit,requiredArtifacts,finalEvidenceReconciliation]")
            || compact.contains("w.finalReport('final-acceptance-report',[finalGate,finalAudit,requiredArtifacts,finalEvidenceReconciliation]"),
        "generated decomposed PRD workflow final report must receive explicit typed source results",
    )?;
    reject(
        compact.contains("readyItems=remainingItems;")
            || compact.contains("readyItems=implementationItems")
            || compact.contains("readyImplementationItems=remainingItems;")
            || compact.contains("readyImplementationItems=readyItems;")
            || compact.contains("fanout(\"implementation-wave-\"+currentImplementationWaveIndex,implementationItems")
            || compact.contains("fanout('implementation-wave-'+currentImplementationWaveIndex,implementationItems")
            || compact.contains("fanout(\"implementation-wave-\"+currentImplementationWaveIndex,remainingItems")
            || compact.contains("fanout('implementation-wave-'+currentImplementationWaveIndex,remainingItems"),
        "generated decomposed PRD workflow must not schedule all remaining implementation items as one unordered wave",
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "workflow_live_generated_semantics_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "workflow_live_generated_semantics_tests_review.rs"]
mod review_tests;

#[cfg(test)]
#[path = "workflow_live_generated_semantics_tests_noop.rs"]
mod noop_tests;

#[cfg(test)]
#[path = "workflow_live_generated_semantics_tests_remediation.rs"]
mod remediation_tests;
