pub(super) fn apply_ownership_expansion_lifecycle(source: String) -> String {
    let needle = r#"        if (unresolvedAfterRemediation.length > 0) {
          return await w.finalReport("blocked-remediation-unresolved-" + currentImplementationWaveIndex, {
            status: "needs_review",
            inputs: { taskUniverse, readyImplementationItems, wave, remediationInventory, remediationWave, unresolvedAfterRemediation, implementationEvidence, repair_attempts: repairAttempts },
            task: "Stop because bounded JS-owned remediation attempts left unresolved implementation outcomes. Report exact remaining gaps and next valid restart/remediation choices."
          });
        }
"#;
    let Some(start) = source.find(needle) else {
        return source;
    };
    let mut rewritten = String::with_capacity(source.len() + OWNERSHIP_EXPANSION_JS.len());
    rewritten.push_str(&source[..start]);
    rewritten.push_str(OWNERSHIP_EXPANSION_JS);
    rewritten.push_str(&source[start + needle.len()..]);
    rewritten
}

const OWNERSHIP_EXPANSION_JS: &str = r#"        if (unresolvedAfterRemediation.length > 0) {
          const ownershipExpansionOutcomes = unresolvedAfterRemediation.filter((outcome) => {
            const data = (outcome && outcome.result && outcome.result.data) || (outcome && outcome.data) || {};
            return data.ownership_expansion_required === true || generatedContractArray(data.proposed_ownership_expansions).length > 0;
          });
          if (ownershipExpansionOutcomes.length > 0) {
            let ownershipExpansionInventory = await w.reduce("ownership-expansion-inventory-" + currentImplementationWaveIndex + "-" + remediationAttempt, [taskUniverse, readyImplementationItems, remediationInventory.items, remediationWave, ownershipExpansionOutcomes, implementationEvidence], {
              tier: "reducer",
              task: "Review unresolved write-branch evidence that proposes explicit repo path ownership expansion. Return follow-up remediation items only when each added path is an exact repo-owned file needed for the same canonical task and supported by branch evidence. Preserve existing target_files, append only justified explicit files, keep dependency_ids canonical, include focused_verification and artifact_requirements, and do not broaden by directory, language, framework, glob, or project artifact path."
            });
            recordRepairAttempt(repairAttempts, "ownership-expansion-inventory-" + currentImplementationWaveIndex + "-" + remediationAttempt, "ownership_expansion_inventory", ownershipExpansionOutcomes, ownershipExpansionInventory);
            ownershipExpansionInventory = normalizeRemediationInventory(ownershipExpansionInventory);
            if (remediationInventoryReady(ownershipExpansionInventory)) {
              const ownershipExpansionWave = await w.fanout("remediation-wave-" + currentImplementationWaveIndex + "-ownership-" + remediationAttempt, ownershipExpansionInventory.items, {
                tier: "coder",
                itemKind: "implementation",
                write: "worktree",
                maxParallelism: "configured",
                targetFilesFromItem: true,
                task: "Run follow-up remediation only for the assigned unresolved item after JS-owned explicit ownership expansion. Return item_id, canonical_task_ids, snake_case status, concrete evidence, changed files, commands/tests, artifacts, and residual gaps."
              });
              implementationEvidence.push({ kind: "ownership-expansion-remediation", implementationWaveIndex: currentImplementationWaveIndex, dependencyIteration, remediationAttempt, remediationInventory: ownershipExpansionInventory, result: ownershipExpansionWave });
              implementationCandidateIds = implementationCandidateIds.concat(matchingAcceptedIds(ownershipExpansionInventory.items, ownershipExpansionWave.outcomes));
              remediationInventory = ownershipExpansionInventory;
              remediationWave = ownershipExpansionWave;
              unresolvedAfterRemediation = nonAcceptedOutcomes(ownershipExpansionWave.outcomes);
            }
          }
        }
        if (unresolvedAfterRemediation.length > 0) {
          return await w.finalReport("blocked-remediation-unresolved-" + currentImplementationWaveIndex, {
            status: "needs_review",
            inputs: { taskUniverse, readyImplementationItems, wave, remediationInventory, remediationWave, unresolvedAfterRemediation, implementationEvidence, repair_attempts: repairAttempts },
            task: "Stop because bounded JS-owned remediation attempts left unresolved implementation outcomes. Report exact remaining gaps and next valid restart/remediation choices."
          });
        }
"#;
