pub(super) fn apply_verification_remediation_lifecycle(source: String) -> String {
    let start = r#"      let verificationPlan = await w.reduce("verification-plan-" + currentImplementationWaveIndex"#;
    let end = r#"    let newlyCompletedIds = Array.from(acceptedThisWave).filter((id) => !completedIds.has(id));"#;
    let Some(start_idx) = source.find(start) else {
        return source;
    };
    let Some(relative_end_idx) = source[start_idx..].find(end) else {
        return source;
    };
    let end_idx = start_idx + relative_end_idx;
    let mut rewritten = String::with_capacity(source.len() + VERIFICATION_LIFECYCLE_JS.len());
    rewritten.push_str(&source[..start_idx]);
    rewritten.push_str(VERIFICATION_LIFECYCLE_JS);
    rewritten.push_str(&source[end_idx..]);
    rewritten
}

const VERIFICATION_LIFECYCLE_JS: &str = r#"      let verificationPlan = await w.reduce("verification-plan-" + currentImplementationWaveIndex, [taskUniverse, readyImplementationItems, implementationCandidateIdsUnique, implementationEvidence], {
        tier: "reducer",
        task: "Plan focused verification for newly implemented or remediated canonical task IDs before dependents can unblock. Return one verification item per exact command/check whenever possible. Each item must include item_id, canonical_task_ids, focused_verification, expected_evidence, optional artifact_requirements, and source_item_id. Check project artifacts relative to projectArtifactRoot when present, and repository source files relative to targetRepositoryRoot. When provider/API credentials matter, include provider_env_requirements with exact env key names and require redacted provider_env_proof; never include values. Evidence is accepted when at least one intended target passes and no intended target fails; duplicate cargo workspace/lib/bin harness pass sections are valid as one canonical pass."
      });
      verificationPlan = normalizeGeneratedInventory(verificationPlan);
      let verificationPlanRepairAttempt = 1;
      while (!generatedContractVerificationInventoryReady(verificationPlan) && verificationPlanRepairAttempt <= maxRepairIterations) {
        const verificationPlanRepair = await w.reduce("verification-plan-repair-" + currentImplementationWaveIndex + "-" + verificationPlanRepairAttempt, [taskUniverse, readyImplementationItems, implementationCandidateIdsUnique, implementationEvidence, verificationPlan], {
          tier: "reducer",
          task: "Repair an empty or malformed focused verification plan into concrete verification items, preferably one item per exact command/check, with item_id, canonical_task_ids, focused_verification, expected_evidence, optional artifact_requirements, and source_item_id. Check project artifacts relative to projectArtifactRoot when present. Include provider_env_requirements and redacted provider_env_proof requirements for provider-sensitive checks, or return concrete evidence that verification cannot be inferred safely."
        });
        recordRepairAttempt(repairAttempts, "verification-plan-repair-" + currentImplementationWaveIndex + "-" + verificationPlanRepairAttempt, "verification_plan_repair", implementationCandidateIdsUnique, verificationPlanRepair);
        verificationPlan = normalizeGeneratedInventory(verificationPlanRepair);
        verificationPlanRepairAttempt += 1;
      }
      if (generatedContractVerificationInventoryReady(verificationPlan)) {
        verificationPlan.items = generatedContractVerificationItems(verificationPlan);
      }
      if (!generatedContractVerificationInventoryReady(verificationPlan) || !verificationPlan.items || verificationPlan.items.length === 0) {
        return await w.finalReport("blocked-empty-verification-" + currentImplementationWaveIndex, {
          status: "needs_review",
          inputs: { taskUniverse, readyImplementationItems, implementationCandidateIdsUnique, verificationPlan, implementationEvidence, repair_attempts: repairAttempts },
          task: "Stop because bounded JS-owned repair could not produce a focused verification plan for newly implemented work."
        });
      }
      let verification = await w.parallel("verification-wave-" + currentImplementationWaveIndex, verificationPlan.items, {
        tier: "coder",
        itemKind: "focused_verification",
        task: "Run focused verification only. Return structured status, item_id, canonical_task_ids, focused_verification executed, commands run with exit_code and output_summary, matched test/check names, pass/fail count, artifacts checked, source_item_id, verification_failure_class when failed, and residual gaps. Accept duplicate cargo workspace/lib/bin harness pass sections as one canonical pass when at least one intended target passes and no intended target fails. Do not modify files."
      });
      verificationEvidence.push({ kind: "verification", implementationWaveIndex: currentImplementationWaveIndex, dependencyIteration, verificationPlan, result: verification });

      let verificationRepairAttempt = 1;
      let verificationRemediationAttempt = 1;
      while (verification.status !== "accepted" && verification.status !== "noop" && verificationRepairAttempt <= maxRepairIterations) {
        const verificationOutcomes = verification.outcomes || verification.items || [verification];
        const actionableVerificationFailures = verificationOutcomes.filter((outcome) => {
          const data = (outcome && outcome.result && outcome.result.data) || (outcome && outcome.data) || {};
          return data.verification_remediation_required === true || data.verification_failure_class === "actionable_implementation_failure" || data.verification_failure_next_action === "write_remediation";
        });
        if (actionableVerificationFailures.length > 0) {
          const verificationFailureTriage = await w.reduce("verification-failure-triage-" + currentImplementationWaveIndex + "-" + verificationRepairAttempt, [taskUniverse, readyImplementationItems, verificationPlan.items, actionableVerificationFailures, implementationEvidence, verificationEvidence], {
            tier: "reducer",
            task: "Classify failed focused verification outcomes. Return actionable implementation failures as items requiring write remediation, retryable verification-shape issues as retry items, and terminal external/runtime/safety blockers with concrete evidence. Do not mark tasks complete."
          });
          recordRepairAttempt(repairAttempts, "verification-failure-triage-" + currentImplementationWaveIndex + "-" + verificationRepairAttempt, "verification_failure_triage", actionableVerificationFailures, verificationFailureTriage);
          let verificationRemediationInventory = await w.reduce("verification-remediation-inventory-" + currentImplementationWaveIndex + "-" + verificationRemediationAttempt, [taskUniverse, readyImplementationItems, verificationPlan.items, verificationFailureTriage, actionableVerificationFailures, implementationEvidence, verificationEvidence], {
            tier: "reducer",
            task: "Create write-capable remediation items only for actionable implementation failures proven by focused verification. Every item must include item_id, source_item_id, canonical_task_ids, dependency_ids using canonical task IDs only, target_files from the original implementation item ownership, failure_status, failure_evidence, required_fix, focused_verification, and artifact_requirements."
          });
          recordRepairAttempt(repairAttempts, "verification-remediation-inventory-" + currentImplementationWaveIndex + "-" + verificationRemediationAttempt, "verification_remediation_inventory", actionableVerificationFailures, verificationRemediationInventory);
          verificationRemediationInventory = normalizeRemediationInventory(verificationRemediationInventory);
          if (!remediationInventoryReady(verificationRemediationInventory)) {
            break;
          }
          const verificationRemediationWave = await w.fanout("remediation-wave-" + currentImplementationWaveIndex + "-verification-" + verificationRemediationAttempt, verificationRemediationInventory.items, {
            tier: "coder",
            itemKind: "implementation",
            write: "worktree",
            maxParallelism: "configured",
            targetFilesFromItem: true,
            task: "Fix only the assigned focused-verification failure. Use the original implementation item's ownership. Return item_id, canonical_task_ids, snake_case status, concrete evidence, changed files, commands/tests, artifacts, and residual gaps."
          });
          implementationEvidence.push({ kind: "verification-remediation", implementationWaveIndex: currentImplementationWaveIndex, dependencyIteration, verificationRemediationAttempt, verificationRemediationInventory, result: verificationRemediationWave });
          implementationCandidateIds = implementationCandidateIds.concat(matchingAcceptedIds(verificationRemediationInventory.items, verificationRemediationWave.outcomes));
          const unresolvedVerificationRemediation = nonAcceptedOutcomes(verificationRemediationWave.outcomes);
          if (unresolvedVerificationRemediation.length > 0) {
            recordRepairAttempt(repairAttempts, "remediation-wave-" + currentImplementationWaveIndex + "-verification-" + verificationRemediationAttempt, "verification_remediation_unresolved", unresolvedVerificationRemediation, verificationRemediationWave);
            break;
          }
          let postRemediationVerificationPlan = await w.reduce("post-remediation-verification-plan-" + currentImplementationWaveIndex + "-" + verificationRemediationAttempt, [taskUniverse, readyImplementationItems, verificationRemediationInventory.items, verificationRemediationWave, implementationEvidence, verificationEvidence], {
            tier: "reducer",
            task: "Plan focused verification after write remediation. Return one item per exact command/check with item_id, canonical_task_ids, focused_verification, expected_evidence, optional artifact_requirements, and source_item_id. Include provider_env_requirements for provider-sensitive checks and require redacted provider_env_proof only."
          });
          recordRepairAttempt(repairAttempts, "post-remediation-verification-plan-" + currentImplementationWaveIndex + "-" + verificationRemediationAttempt, "post_remediation_verification_plan", verificationRemediationInventory.items, postRemediationVerificationPlan);
          postRemediationVerificationPlan = normalizeGeneratedInventory(postRemediationVerificationPlan);
          let postRemediationVerificationPlanRepairAttempt = 1;
          while (!generatedContractVerificationInventoryReady(postRemediationVerificationPlan) && postRemediationVerificationPlanRepairAttempt <= maxRepairIterations) {
            const postRemediationVerificationPlanRepair = await w.reduce("post-remediation-verification-plan-repair-" + currentImplementationWaveIndex + "-" + verificationRemediationAttempt + "-" + postRemediationVerificationPlanRepairAttempt, [taskUniverse, verificationRemediationInventory.items, verificationRemediationWave, postRemediationVerificationPlan], {
              tier: "reducer",
              task: "Repair malformed post-remediation verification output into canonical focused verification items with item_id, canonical_task_ids, focused_verification, expected_evidence or artifact_requirements, and source_item_id."
            });
            recordRepairAttempt(repairAttempts, "post-remediation-verification-plan-repair-" + currentImplementationWaveIndex + "-" + verificationRemediationAttempt + "-" + postRemediationVerificationPlanRepairAttempt, "post_remediation_verification_plan_repair", postRemediationVerificationPlan.unresolved_issues || [], postRemediationVerificationPlanRepair);
            postRemediationVerificationPlan = normalizeGeneratedInventory(postRemediationVerificationPlanRepair);
            postRemediationVerificationPlanRepairAttempt += 1;
          }
          if (!generatedContractVerificationInventoryReady(postRemediationVerificationPlan) || !postRemediationVerificationPlan.items || postRemediationVerificationPlan.items.length === 0) {
            break;
          }
          postRemediationVerificationPlan.items = generatedContractVerificationItems(postRemediationVerificationPlan);
          verification = await w.parallel("verification-wave-" + currentImplementationWaveIndex + "-post-remediation-" + verificationRemediationAttempt, postRemediationVerificationPlan.items, {
            tier: "coder",
            itemKind: "focused_verification",
            task: "Run focused post-remediation verification only. Return structured status, item_id, canonical_task_ids, commands run with exit_code and output_summary, matched checks, pass/fail count, artifacts checked, source_item_id, verification_failure_class when failed, and residual gaps. Do not modify files."
          });
          verificationEvidence.push({ kind: "post-remediation-verification", implementationWaveIndex: currentImplementationWaveIndex, dependencyIteration, verificationRemediationAttempt, verificationPlan: postRemediationVerificationPlan, result: verification });
          verificationRemediationAttempt += 1;
          verificationRepairAttempt += 1;
          continue;
        }

        const verificationRepairPlan = await w.reduce("verification-repair-plan-" + currentImplementationWaveIndex + "-" + verificationRepairAttempt, [taskUniverse, verificationPlan.items, verification, implementationEvidence], {
          tier: "reducer",
          task: "Repair failed focused verification shape into a concrete retry plan only when the issue is missing/malformed verification evidence, target selection, or retryable command shape. Do not use verification repair for product-code or artifact-contract failures; those require write remediation."
        });
        recordRepairAttempt(repairAttempts, "verification-repair-plan-" + currentImplementationWaveIndex + "-" + verificationRepairAttempt, "verification_repair", verification.outcomes || verification.items || [verification], verificationRepairPlan);
        let verificationRepairInventory = normalizeGeneratedInventory(verificationRepairPlan);
        const verificationRepairAllowedTaskIds = generatedContractUnique(generatedContractArray(verificationPlan.items).flatMap((item) => generatedContractArray(item.canonical_task_ids)));
        verificationRepairInventory = generatedContractConstrainInventoryTasks(verificationRepairInventory, verificationRepairAllowedTaskIds);
        let verificationRepairShapeAttempt = 1;
        while (!generatedContractVerificationInventoryReady(verificationRepairInventory) && verificationRepairInventory.unresolved_issues && verificationRepairInventory.unresolved_issues.length > 0 && verificationRepairShapeAttempt <= maxRepairIterations) {
          const verificationRepairShapeRepair = await w.reduce("verification-repair-shape-repair-" + currentImplementationWaveIndex + "-" + verificationRepairAttempt + "-" + verificationRepairShapeAttempt, [taskUniverse, verificationRepairInventory, verificationRepairInventory.unresolved_issues, verification, implementationEvidence], {
            tier: "reducer",
            task: "Repair malformed focused verification retry output into canonical items. Return items with item_id, canonical_task_ids, focused_verification, expected_evidence or artifact_requirements, source_item_id, and provider_env_requirements when provider credentials are required. Project artifact checks must name projectArtifactRoot-relative paths when possible."
          });
          recordRepairAttempt(repairAttempts, "verification-repair-shape-repair-" + currentImplementationWaveIndex + "-" + verificationRepairAttempt + "-" + verificationRepairShapeAttempt, "verification_repair_shape_repair", verificationRepairInventory.unresolved_issues, verificationRepairShapeRepair);
          verificationRepairInventory = normalizeGeneratedInventory(verificationRepairShapeRepair);
          verificationRepairInventory = generatedContractConstrainInventoryTasks(verificationRepairInventory, verificationRepairAllowedTaskIds);
          verificationRepairShapeAttempt += 1;
        }
        if (!generatedContractVerificationInventoryReady(verificationRepairInventory) || !verificationRepairInventory.items || verificationRepairInventory.items.length === 0) {
          break;
        }
        verificationRepairPlan.items = generatedContractVerificationItems(verificationRepairInventory);
        verification = await w.parallel("verification-wave-" + currentImplementationWaveIndex + "-" + verificationRepairAttempt, verificationRepairPlan.items, {
          tier: "coder",
          itemKind: "focused_verification",
          task: "Run repaired focused verification only. Return structured status, item_id, canonical_task_ids, focused_verification executed, commands run with exit_code and output_summary, matched test/check names, pass/fail count, artifacts checked, source_item_id, verification_failure_class when failed, and residual gaps. Do not modify files."
        });
        verificationEvidence.push({ kind: "verification-retry", implementationWaveIndex: currentImplementationWaveIndex, dependencyIteration, verificationRepairAttempt, verificationPlan: verificationRepairPlan, result: verification });
        verificationRepairAttempt += 1;
      }
      if (verification.status !== "accepted" && verification.status !== "noop") {
        return await w.finalReport("blocked-verification-failed-" + currentImplementationWaveIndex, {
          status: "needs_review",
          inputs: { taskUniverse, readyImplementationItems, implementationCandidateIdsUnique, implementationEvidence, verificationEvidence, verification, repair_attempts: repairAttempts },
          task: "Stop because bounded JS-owned verification triage, verification repair, and write remediation did not produce accepted/noop evidence. Report exact commands/results, remediation attempts, and unresolved gaps."
        });
      }
      for (const id of acceptedOrNoopCanonicalTaskIdsFrom(verification.outcomes || verification.items)) {
        if (implementationCandidateIdsUnique.includes(id)) {
          acceptedThisWave.add(id);
        }
      }
    }

"#;
