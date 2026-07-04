  function array(value) {
    return generatedContractArray(value);
  }

  function canonicalIdsFor(item) {
    return array(item && item.canonical_task_ids).filter((id) => canonicalTaskUniverse.has(id));
  }

  function dependencyIdsFor(item) {
    return array(item && item.dependency_ids).filter((id) => canonicalTaskUniverse.has(id));
  }

  function invalidDependencyIdsFor(item) {
    return array(item && item.dependency_ids).filter((id) => !canonicalTaskUniverse.has(id));
  }

  function readyItemsFrom(items, completedIds) {
    return array(items).filter((item) => dependencyIdsFor(item).every((id) => completedIds.has(id)));
  }

  function hasConcreteEvidence(outcome) {
    if (!outcome || !outcome.evidence) {
      return generatedContractPresent(outcome.completion_evidence) || generatedContractPresent(outcome.artifact_paths) || generatedContractPresent(outcome.artifacts) || generatedContractPresent(outcome.task_coverage) || generatedContractPresent(outcome.commands_run);
    }
    if (Array.isArray(outcome.evidence)) {
      return outcome.evidence.length > 0;
    }
    if (typeof outcome.evidence === "object") {
      return Object.keys(outcome.evidence).length > 0;
    }
    return Boolean(outcome.evidence);
  }

  function acceptedOrNoopCanonicalTaskIdsFrom(outcomes) {
    const ids = [];
    for (const outcome of array(outcomes)) {
      if (!outcome || (outcome.status !== "accepted" && outcome.status !== "noop")) {
        continue;
      }
      if (!hasConcreteEvidence(outcome)) {
        continue;
      }
      for (const id of canonicalIdsFor(outcome)) {
        ids.push(id);
      }
    }
    return ids;
  }

  function nonAcceptedOutcomes(outcomes) {
    return array(outcomes).filter((outcome) => !outcome || (outcome.status !== "accepted" && outcome.status !== "noop"));
  }

  function workTypeFor(item) {
    return item && typeof item.work_type === "string" ? item.work_type : "";
  }

  function validImplementationItem(item) {
    return Boolean(
      item &&
      workTypeFor(item) === "implementation" &&
      (item.item_id || item.id) &&
      canonicalIdsFor(item).length > 0 &&
      invalidDependencyIdsFor(item).length === 0 &&
      generatedContractPresent(item.target_files) &&
      generatedContractPresent(item.acceptance_criteria) &&
      generatedContractPresent(item.focused_verification) &&
      item.artifact_requirements !== undefined
    );
  }

  function validVerifiedNoopItem(item) {
    return Boolean(
      item &&
      workTypeFor(item) === "verified_noop" &&
      (item.item_id || item.id) &&
      canonicalIdsFor(item).length > 0 &&
      invalidDependencyIdsFor(item).length === 0 &&
      generatedContractPresent(item.acceptance_criteria) &&
      generatedContractPresent(item.noop_proof) &&
      generatedContractPresent(item.noop_proof_refs) &&
      item.artifact_requirements !== undefined
    );
  }

  function validInventoryItem(item) {
    return validImplementationItem(item) || validVerifiedNoopItem(item);
  }

  function itemIsCompleted(item, completedIds) {
    const ids = canonicalIdsFor(item);
    return ids.length > 0 && ids.every((id) => completedIds.has(id));
  }

  function matchingAcceptedIds(sourceItems, outcomes) {
    const allowed = new Set();
    for (const item of array(sourceItems)) {
      for (const id of canonicalIdsFor(item)) {
        allowed.add(id);
      }
    }
    return acceptedOrNoopCanonicalTaskIdsFrom(outcomes).filter((id) => allowed.has(id));
  }

  function repairIssuesSummary(inventory) {
    return array(inventory && inventory.unresolved_issues).map((issue) => {
      return {
        kind: issue.kind,
        field: issue.field,
        item_id: issue.item_id,
        canonical_task_ids: issue.canonical_task_ids,
        message: issue.message
      };
    });
  }

  const discovery = await w.parallel("initial-readonly-discovery", discoveryItems, {
    tier: "analysis",
    task: "Run read-only discovery in parallel. Return structured summaries only: files read, commands run, task coverage, implementation gaps, artifact requirements, and risks. Do not accept implementation tasks from read-only work."
  });

  const rawInventory = await w.reduce("canonical-implementation-inventory", [taskUniverse, discovery, governedLearningContext], {
    tier: "reducer",
    task: "Using taskUniverse, discovery, and governedLearningContext, produce dependency-owned inventory items only for genuinely missing or provably no-op work. Treat learning context as advisory evidence references, not truth. Every item must include work_type. implementation items require item_id, canonical_task_ids, dependency_ids using canonical task IDs only, target_files, acceptance_criteria, focused_verification, and artifact_requirements. target_files must be repo-owned implementation files under targetRepositoryRoot; task docs, context, progress, report, and artifact paths are evidence/artifact refs, not implementation write targets. Split grouped items by dependency readiness: no dependency_id may overlap the same item's canonical_task_ids, and do not group tasks when one claimed task depends on another claimed task. Prerequisite tasks must be represented as implementation or verified_noop items. verified_noop items require item_id, canonical_task_ids, dependency_ids, acceptance_criteria, noop_proof, proof reference entries, and artifact_requirements, using [] only with evidence that no artifact/output is required. Do not emit an empty inventory unless every canonical task has concrete accepted/noop proof."
  });

  let inventory = normalizeGeneratedInventory(rawInventory);
  let repairAttempt = 1;
  while (array(inventory.unresolved_issues).length > 0 && repairAttempt <= Math.max(maxRepairIterations, maxInvestigationIterations)) {
    const shapeIssues = issuesOfKind(inventory, "inventory_shape_repair");
    if (shapeIssues.length > 0 && repairAttempt <= maxRepairIterations) {
      const inventoryShapeRepair = await w.reduce("inventory-shape-repair-" + repairAttempt, [taskUniverse, inventory, shapeIssues, discovery, governedLearningContext], {
        tier: "reducer",
        task: "Repair inventory shape only. Preserve taskUniverse canonical IDs and all existing schedulable fields. Return repaired implementation items with work_type, item_id, canonical_task_ids, dependency_ids, target_files, acceptance_criteria, focused_verification, and artifact_requirements. Return repaired verified_noop items with item_id, canonical_task_ids, dependency_ids, acceptance_criteria, noop_proof, noop_proof_refs/proof_references, and artifact_requirements, using [] only with evidence that no artifact/output is required. Do not replace detailed items with skeletons. Put crosscutting/support notes in repair_summary or evidence_refs, not inventory items."
      });
      recordRepairAttempt(repairAttempts, "inventory-shape-repair-" + repairAttempt, "inventory_shape_repair", shapeIssues, inventoryShapeRepair);
      inventory = normalizeGeneratedInventory(mergeInventoryRepair(inventory, inventoryShapeRepair));
    }

    const universeIssues = issuesOfKind(inventory, "task_universe_reconcile");
    if (universeIssues.length > 0 && repairAttempt <= maxRepairIterations) {
      const taskUniverseReconcile = await w.reduce("task-universe-reconcile-" + repairAttempt, [taskUniverse, inventory, universeIssues, discovery], {
        tier: "reducer",
        task: "Reconcile inventory dependency IDs to canonical task IDs in taskUniverse. Return repaired items or evidence that dependencies are genuinely unresolved."
      });
      recordRepairAttempt(repairAttempts, "task-universe-reconcile-" + repairAttempt, "task_universe_reconcile", universeIssues, taskUniverseReconcile);
      inventory = normalizeGeneratedInventory(mergeInventoryRepair(inventory, taskUniverseReconcile));
    }

    const graphIssues = issuesOfKind(inventory, "dependency_graph_repair");
    if (graphIssues.length > 0 && repairAttempt <= maxRepairIterations) {
      const dependencyGraphRepair = await w.reduce("dependency-graph-repair-" + repairAttempt, [taskUniverse, inventory, graphIssues, discovery, governedLearningContext], {
        tier: "reducer",
        task: "Repair dependency graph defects before scheduling. Split grouped items by dependency readiness while preserving or restoring every schedulable field required by the contract. implementation items must include target_files, acceptance_criteria, focused_verification, and artifact_requirements. verified_noop items must include acceptance_criteria, noop_proof, noop_proof_refs/proof_references, and artifact_requirements. Add verified_noop prerequisite items only with concrete proof refs. Remove impossible internal dependency edges only with evidence, or return concrete unrecoverable graph evidence. No dependency_id may overlap the same item's canonical_task_ids. Put crosscutting verification/support notes in repair_summary or evidence_refs, not inventory items."
      });
      recordRepairAttempt(repairAttempts, "dependency-graph-repair-" + repairAttempt, "dependency_graph_repair", graphIssues, dependencyGraphRepair);
      inventory = normalizeGeneratedInventory(mergeInventoryRepair(inventory, dependencyGraphRepair));
    }

    const targetFileIssues = issuesOfKind(inventory, "target_file_discovery");
    if (targetFileIssues.length > 0 && repairAttempt <= maxInvestigationIterations) {
      const targetFileDiscovery = await w.reduce("target-file-discovery-" + repairAttempt, [taskUniverse, inventory, targetFileIssues, discovery], {
        tier: "analysis",
        task: "Investigate missing or invalid target files from PRD/TASK files and repository evidence. Return repaired implementation items whose target_files are repo-owned implementation files under targetRepositoryRoot, or convert to verified_noop with concrete proof refs, or return concrete evidence that safe repo-owned targets cannot be inferred. Do not use task context/progress/report/artifact files as implementation target_files."
      });
      recordRepairAttempt(repairAttempts, "target-file-discovery-" + repairAttempt, "target_file_discovery", targetFileIssues, targetFileDiscovery);
      inventory = normalizeGeneratedInventory(mergeInventoryRepair(inventory, targetFileDiscovery));
    }

    const verificationIssues = issuesOfKind(inventory, "verification_requirements_discovery");
    if (verificationIssues.length > 0 && repairAttempt <= maxInvestigationIterations) {
      const verificationRequirementsDiscovery = await w.reduce("verification-requirements-discovery-" + repairAttempt, [taskUniverse, inventory, verificationIssues, discovery], {
        tier: "analysis",
        task: "Investigate missing acceptance criteria or focused verification requirements. Return repaired items with focused_verification and acceptance_criteria or concrete evidence that requirements are unavailable."
      });
      recordRepairAttempt(repairAttempts, "verification-requirements-discovery-" + repairAttempt, "verification_requirements_discovery", verificationIssues, verificationRequirementsDiscovery);
      inventory = normalizeGeneratedInventory(mergeInventoryRepair(inventory, verificationRequirementsDiscovery));
    }

    const artifactIssues = issuesOfKind(inventory, "artifact_requirements_discovery");
    if (artifactIssues.length > 0 && repairAttempt <= maxInvestigationIterations) {
      const artifactRequirementsDiscovery = await w.reduce("artifact-requirements-discovery-" + repairAttempt, [taskUniverse, inventory, artifactIssues, discovery], {
        tier: "analysis",
        task: "Investigate artifact requirements from PRD/TASK files. Return repaired items with artifact_requirements, using an empty array only when no artifact is required and that no-op is evidenced."
      });
      recordRepairAttempt(repairAttempts, "artifact-requirements-discovery-" + repairAttempt, "artifact_requirements_discovery", artifactIssues, artifactRequirementsDiscovery);
      inventory = normalizeGeneratedInventory(mergeInventoryRepair(inventory, artifactRequirementsDiscovery));
    }

    const providerIssues = issuesOfKind(inventory, "provider_environment_discovery");
    if (providerIssues.length > 0 && repairAttempt <= maxInvestigationIterations) {
      const providerEnvironmentDiscovery = await w.reduce("provider-environment-discovery-" + repairAttempt, [taskUniverse, inventory, providerIssues, discovery], {
        tier: "analysis",
        task: "Investigate provider/environment evidence without exposing secrets. Return repaired items with redacted env keys checked, commands_run summaries, and provider evidence or concrete external-unavailable evidence."
      });
      recordRepairAttempt(repairAttempts, "provider-environment-discovery-" + repairAttempt, "provider_environment_discovery", providerIssues, providerEnvironmentDiscovery);
      inventory = normalizeGeneratedInventory(mergeInventoryRepair(inventory, providerEnvironmentDiscovery));
    }

    const evidenceIssues = issuesOfKind(inventory, "evidence_repair");
    if (evidenceIssues.length > 0 && repairAttempt <= maxRepairIterations) {
      const evidenceRepair = await w.reduce("evidence-repair-" + repairAttempt, [taskUniverse, inventory, evidenceIssues, discovery, governedLearningContext], {
        tier: "reducer",
        task: "Repair missing verified no-op evidence fields. Return repaired items with noop_proof and noop_proof_refs/proof_references tied to PRD/TASK criteria, or concrete evidence that no safe no-op proof exists."
      });
      recordRepairAttempt(repairAttempts, "evidence-repair-" + repairAttempt, "evidence_repair", evidenceIssues, evidenceRepair);
      inventory = normalizeGeneratedInventory(mergeInventoryRepair(inventory, evidenceRepair));
    }

    repairAttempt += 1;
  }

  const malformedInventoryItems = array(inventory.items).filter((item) => !validInventoryItem(item));
  if (malformedInventoryItems.length > 0 || array(inventory.unresolved_issues).length > 0) {
    return await w.finalReport("blocked-malformed-inventory", {
      status: "needs_review",
      inputs: { taskUniverse, inventory, malformedInventoryItems, repair_attempts: repairAttempts },
      task: "Stop after JS-owned repair/investigation attempts because inventory is still malformed or contains unresolved/non-canonical dependencies. Include repair_attempts with call IDs, issue kinds, canonical task IDs, files read, commands run, artifacts checked, redacted env keys checked, evidence refs, and exact reasons the workflow cannot safely continue."
    });
  }

  let remainingItems = array(inventory.items);
  const completedIds = new Set();
  let dependencyIteration = 1;
  let implementationWaveIndex = 1;

  if (remainingItems.length === 0) {
    return await w.finalReport("blocked-empty-implementation-inventory", {
      status: "needs_review",
      inputs: { taskUniverse, discovery, inventory, repair_attempts: repairAttempts },
      task: "Stop because the inventory is empty. Accept this only if every canonical task has concrete accepted/noop evidence; otherwise report the missing inventory evidence."
    });
  }

  while (remainingItems.length > 0 && dependencyIteration <= maxDependencyWaves) {
    const readyItems = readyItemsFrom(remainingItems, completedIds);
    if (readyItems.length === 0) {
      let deadlockRepairAttempt = 1;
      let deadlockRepaired = false;
      while (deadlockRepairAttempt <= maxRepairIterations) {
        const deadlockGraphIssues = generatedContractInventoryGraphIssues(remainingItems, Array.from(completedIds));
        const dependencyGraphRepair = await w.reduce("dependency-graph-repair-deadlock-" + dependencyIteration + "-" + deadlockRepairAttempt, [taskUniverse, inventory, remainingItems, deadlockGraphIssues, Array.from(completedIds), discovery, governedLearningContext], {
          tier: "reducer",
          task: "Repair a dependency deadlock before terminal stop. Split grouped items by dependency readiness, add verified_noop prerequisite items with concrete proof refs, or return concrete unrecoverable graph evidence with files read, commands/artifacts checked, and canonical task IDs."
        });
        recordRepairAttempt(repairAttempts, "dependency-graph-repair-deadlock-" + dependencyIteration + "-" + deadlockRepairAttempt, "dependency_graph_repair", deadlockGraphIssues, dependencyGraphRepair);
        inventory = normalizeGeneratedInventory(mergeInventoryRepair(inventory, dependencyGraphRepair));
        remainingItems = array(inventory.items).filter((item) => !itemIsCompleted(item, completedIds));
        if (readyItemsFrom(remainingItems, completedIds).length > 0) {
          deadlockRepaired = true;
          break;
        }
        deadlockRepairAttempt += 1;
      }
      if (deadlockRepaired) {
        continue;
      }
      return await w.finalReport("blocked-dependency-deadlock-" + dependencyIteration, {
        status: "blocked",
        inputs: { taskUniverse, remainingItems, completed_ids: Array.from(completedIds), implementationEvidence, verificationEvidence, repair_attempts: repairAttempts },
        task: "Stop because no remaining implementation items are dependency-ready after bounded JS-owned dependency graph repair. Report exhausted graph repair evidence and next valid restart/remediation choices."
      });
    }

    const readyNoopItems = readyItems.filter((item) => workTypeFor(item) === "verified_noop");
    const readyImplementationItems = readyItems.filter((item) => workTypeFor(item) === "implementation");
    const acceptedThisWave = new Set();

    if (readyNoopItems.length > 0) {
      const noopProof = await w.parallel("noop-proof-verification-" + dependencyIteration, readyNoopItems, {
        tier: "analysis",
        itemKind: "noop_proof",
        maxParallelism: "configured",
        task: "Verify the assigned dependency-ready no-op proof against PRD/TASK acceptance criteria. Do not modify files. Return item_id, canonical_task_ids, status accepted/noop only with concrete proof refs, artifacts checked, commands if any, and residual gaps."
      });
      verificationEvidence.push({ kind: "verified-noop", dependencyIteration, readyNoopItems, result: noopProof });
      for (const id of matchingAcceptedNoopIds(readyNoopItems, noopProof.outcomes || noopProof.items)) {
        acceptedThisWave.add(id);
      }
      let failedNoopProof = nonAcceptedOutcomes(noopProof.outcomes || noopProof.items);
      if (failedNoopProof.length > 0) {
        let noopRepairAttempt = 1;
        let noopRetryItems = readyNoopItems;
        while (failedNoopProof.length > 0 && noopRepairAttempt <= maxRepairIterations) {
          const noopEvidenceRepair = await w.reduce("noop-evidence-repair-" + dependencyIteration + "-" + noopRepairAttempt, [taskUniverse, noopRetryItems, noopProof, failedNoopProof], {
            tier: "reducer",
            task: "Repair failed dependency-ready no-op proof into re-verifiable no-op items or concrete proof gaps. Preserve canonical task IDs and return items only when further verification can be safely attempted."
          });
          recordRepairAttempt(repairAttempts, "noop-evidence-repair-" + dependencyIteration + "-" + noopRepairAttempt, "evidence_repair", failedNoopProof, noopEvidenceRepair);
          noopRetryItems = array(mergeInventoryRepair({ items: noopRetryItems }, noopEvidenceRepair).items);
          if (noopRetryItems.length === 0) {
            break;
          }
          const noopReverification = await w.parallel("noop-proof-reverification-" + dependencyIteration + "-" + noopRepairAttempt, noopRetryItems, {
            tier: "analysis",
            itemKind: "noop_proof",
            maxParallelism: "configured",
            task: "Re-verify repaired dependency-ready no-op proof against PRD/TASK criteria. Do not modify files. Return item_id, canonical_task_ids, accepted/noop only with concrete proof refs, artifacts checked, commands if any, and residual gaps."
          });
          verificationEvidence.push({ kind: "verified-noop-retry", dependencyIteration, noopRepairAttempt, noopRetryItems, result: noopReverification });
          for (const id of matchingAcceptedNoopIds(noopRetryItems, noopReverification.outcomes || noopReverification.items)) {
            acceptedThisWave.add(id);
          }
          failedNoopProof = nonAcceptedOutcomes(noopReverification.outcomes || noopReverification.items);
          noopRepairAttempt += 1;
        }
      }
      if (failedNoopProof.length > 0) {
        return await w.finalReport("blocked-noop-proof-failed-" + dependencyIteration, {
          status: "needs_review",
          inputs: { taskUniverse, readyNoopItems, noopProof, failedNoopProof, verificationEvidence, repair_attempts: repairAttempts },
          task: "Stop because dependency-ready no-op proof did not produce accepted/noop evidence after bounded JS-owned evidence repair and re-verification. Report exact proof gaps."
        });
      }
    }
    let implementationCandidateIds = [];
    let wave = { status: "noop", outcomes: [] };
    const currentImplementationWaveIndex = implementationWaveIndex;
    if (readyImplementationItems.length > 0) {
      wave = await w.fanout("implementation-wave-" + currentImplementationWaveIndex, readyImplementationItems, {
        tier: "coder",
        itemKind: "implementation",
        write: "worktree",
        maxParallelism: "configured",
        targetFilesFromItem: true,
        task: "Implement only the assigned dependency-ready item. Return one structured outcome per item with item_id, canonical_task_ids, snake_case status, evidence, changed files, commands/tests, artifacts, and residual gaps. Use accepted or noop only with concrete task-linked proof."
      });
      implementationEvidence.push({ kind: "implementation", implementationWaveIndex: currentImplementationWaveIndex, dependencyIteration, readyImplementationItems, result: wave });
      implementationCandidateIds = matchingAcceptedIds(readyImplementationItems, wave.outcomes);

      const failedImplementationOutcomes = nonAcceptedOutcomes(wave.outcomes);
      if (failedImplementationOutcomes.length > 0) {
        let remediationInventory = await w.reduce("remediation-inventory-" + currentImplementationWaveIndex, [taskUniverse, readyImplementationItems, wave, failedImplementationOutcomes, implementationEvidence], {
          tier: "reducer",
          task: "Create remediation items only for non-accepted/non-noop implementation outcomes from the current wave. Reuse target_files from the original readyImplementationItems/source item; do not target workflow.js, branch JSON, task docs, or artifacts unless the source item explicitly owned them. Each remediation item must include item_id, source_item_id, canonical_task_ids, target_files, failure_status, failure_evidence, required_fix, verification_requirements, dependency_ids, focused verification, and artifact requirements."
        });
        remediationInventory = normalizeRemediationInventoryForSources(remediationInventory, readyImplementationItems, [], "implementation-wave-" + currentImplementationWaveIndex);
        let remediationInventoryRepairAttempt = 1;
        while (!remediationInventoryReady(remediationInventory) && remediationInventoryRepairAttempt <= maxRepairIterations) {
          const remediationInventoryRepair = await w.reduce("remediation-empty-inventory-repair-" + currentImplementationWaveIndex + "-" + remediationInventoryRepairAttempt, [taskUniverse, readyImplementationItems, failedImplementationOutcomes, remediationInventory], {
            tier: "reducer",
            task: "Repair an empty or malformed remediation inventory into actionable remediation items for the non-accepted implementation outcomes, preserving canonical task IDs, target files, verification, and artifact requirements, or return concrete evidence that no safe remediation can be inferred."
          });
          recordRepairAttempt(repairAttempts, "remediation-empty-inventory-repair-" + currentImplementationWaveIndex + "-" + remediationInventoryRepairAttempt, "remediation_inventory_repair", failedImplementationOutcomes, remediationInventoryRepair);
          remediationInventory = normalizeRemediationInventoryForSources(remediationInventoryRepair, readyImplementationItems, remediationInventory.items, "implementation-wave-" + currentImplementationWaveIndex);
          remediationInventoryRepairAttempt += 1;
        }
        if (!remediationInventoryReady(remediationInventory)) {
          return await w.finalReport("blocked-malformed-remediation-" + currentImplementationWaveIndex, {
            status: "needs_review",
            inputs: { taskUniverse, readyImplementationItems, wave, failedImplementationOutcomes, remediationInventory, repair_attempts: repairAttempts },
            task: "Stop because implementation produced non-accepted outcomes but bounded JS-owned remediation inventory repair remained empty or malformed. Report exact unresolved outcomes and contract issues."
          });
        }
        let remediationWave = await w.fanout("remediation-wave-" + currentImplementationWaveIndex, remediationInventory.items, {
          tier: "coder",
          itemKind: "implementation",
          write: "worktree",
          maxParallelism: "configured",
          targetFilesFromItem: true,
          task: "Remediate only the assigned unresolved item. Return item_id, canonical_task_ids, snake_case status, concrete evidence, changed files, commands/tests, artifacts, and residual gaps."
        });
        implementationEvidence.push({ kind: "remediation", implementationWaveIndex: currentImplementationWaveIndex, dependencyIteration, remediationInventory, result: remediationWave });
        implementationCandidateIds = implementationCandidateIds.concat(matchingAcceptedIds(remediationInventory.items, remediationWave.outcomes));
        let unresolvedAfterRemediation = nonAcceptedOutcomes(remediationWave.outcomes);
        const remediationTaskIds = remediationTaskIdSet(remediationInventory.items);
        let remediationAttempt = 1;
        while (unresolvedAfterRemediation.length > 0 && remediationAttempt <= maxRepairIterations) {
          let followupRemediationInventory = await w.reduce("remediation-outcome-repair-" + currentImplementationWaveIndex + "-" + remediationAttempt, [taskUniverse, readyImplementationItems, remediationInventory.items, remediationWave, unresolvedAfterRemediation], {
            tier: "reducer",
            task: "Repair unresolved remediation outcomes into a follow-up remediation inventory, preserving original source-owned target files, canonical task IDs, verification, and artifact requirements. Do not infer target files from workflow scripts, branch result JSON, or artifact paths."
          });
          recordRepairAttempt(repairAttempts, "remediation-outcome-repair-" + currentImplementationWaveIndex + "-" + remediationAttempt, "remediation_inventory_repair", unresolvedAfterRemediation, followupRemediationInventory);
          followupRemediationInventory = filterRemediationInventoryByTaskIds(normalizeRemediationInventoryForSources(followupRemediationInventory, remediationInventory.items, readyImplementationItems, "remediation-wave-" + currentImplementationWaveIndex), remediationTaskIds);
          if (!remediationInventoryReady(followupRemediationInventory)) {
            break;
          }
          const followupRemediationWave = await w.fanout("remediation-wave-" + currentImplementationWaveIndex + "-" + remediationAttempt, followupRemediationInventory.items, {
            tier: "coder",
            itemKind: "implementation",
            write: "worktree",
            maxParallelism: "configured",
            targetFilesFromItem: true,
            task: "Run follow-up remediation only for the assigned unresolved item. Return item_id, canonical_task_ids, snake_case status, concrete evidence, changed files, commands/tests, artifacts, and residual gaps."
          });
          implementationEvidence.push({ kind: "remediation-retry", implementationWaveIndex: currentImplementationWaveIndex, dependencyIteration, remediationAttempt, remediationInventory: followupRemediationInventory, result: followupRemediationWave });
          implementationCandidateIds = implementationCandidateIds.concat(matchingAcceptedIds(followupRemediationInventory.items, followupRemediationWave.outcomes));
          remediationInventory = followupRemediationInventory;
          remediationWave = followupRemediationWave;
          unresolvedAfterRemediation = nonAcceptedOutcomes(followupRemediationWave.outcomes);
          remediationAttempt += 1;
        }
        if (unresolvedAfterRemediation.length > 0) {
          return await w.finalReport("blocked-remediation-unresolved-" + currentImplementationWaveIndex, {
            status: "needs_review",
            inputs: { taskUniverse, readyImplementationItems, wave, remediationInventory, remediationWave, unresolvedAfterRemediation, implementationEvidence, repair_attempts: repairAttempts },
            task: "Stop because bounded JS-owned remediation attempts left unresolved implementation outcomes. Report exact remaining gaps and next valid restart/remediation choices."
          });
        }
      }
    }

    const implementationCandidateIdsUnique = Array.from(new Set(implementationCandidateIds)).filter((id) => !completedIds.has(id));
    if (implementationCandidateIdsUnique.length > 0) {
      let verificationPlan = await w.reduce("verification-plan-" + currentImplementationWaveIndex, [taskUniverse, readyImplementationItems, implementationCandidateIdsUnique, implementationEvidence], {
        tier: "reducer",
        task: "Plan focused verification for newly implemented or remediated canonical task IDs before dependents can unblock. Return one verification item per exact command/check whenever possible. Each item must include item_id, canonical_task_ids, focused_verification, expected_evidence, optional artifact_requirements, and source_item_id. Evidence is accepted when at least one intended target passes and no intended target fails; duplicate cargo workspace/lib/bin harness pass sections are valid as one canonical pass."
      });
      let verificationPlanRepairAttempt = 1;
      while ((!verificationPlan.items || verificationPlan.items.length === 0) && verificationPlanRepairAttempt <= maxRepairIterations) {
        const verificationPlanRepair = await w.reduce("verification-plan-repair-" + currentImplementationWaveIndex + "-" + verificationPlanRepairAttempt, [taskUniverse, readyImplementationItems, implementationCandidateIdsUnique, implementationEvidence, verificationPlan], {
          tier: "reducer",
          task: "Repair an empty focused verification plan into concrete verification items, preferably one item per exact command/check, with item_id, canonical_task_ids, focused_verification, expected_evidence, optional artifact_requirements, and source_item_id, or return concrete evidence that verification cannot be inferred safely."
        });
        recordRepairAttempt(repairAttempts, "verification-plan-repair-" + currentImplementationWaveIndex + "-" + verificationPlanRepairAttempt, "verification_plan_repair", implementationCandidateIdsUnique, verificationPlanRepair);
        verificationPlan = verificationPlanRepair;
        verificationPlanRepairAttempt += 1;
      }
      if (verificationPlan.items) {
        verificationPlan.items = splitFocusedVerificationItems(verificationPlan.items);
      }
      if (!verificationPlan.items || verificationPlan.items.length === 0) {
        return await w.finalReport("blocked-empty-verification-" + currentImplementationWaveIndex, {
          status: "needs_review",
          inputs: { taskUniverse, readyImplementationItems, implementationCandidateIdsUnique, verificationPlan, implementationEvidence, repair_attempts: repairAttempts },
          task: "Stop because bounded JS-owned repair could not produce a focused verification plan for newly implemented work."
        });
      }
      let verification = await w.parallel("verification-wave-" + currentImplementationWaveIndex, verificationPlan.items, {
        tier: "coder",
        itemKind: "focused_verification",
        task: "Run focused verification only. Return structured status, item_id, canonical_task_ids, focused_verification executed, commands run with exit_code and output_summary, matched test/check names, pass/fail count, artifacts checked, source_item_id, and residual gaps. Accept duplicate cargo workspace/lib/bin harness pass sections as one canonical pass when at least one intended target passes and no intended target fails. Do not modify files."
      });
      verificationEvidence.push({ kind: "verification", implementationWaveIndex: currentImplementationWaveIndex, dependencyIteration, verificationPlan, result: verification });
      let verificationRepairAttempt = 1;
      while (verification.status !== "accepted" && verification.status !== "noop" && verificationRepairAttempt <= maxRepairIterations) {
        const verificationRepairPlan = await w.reduce("verification-repair-plan-" + currentImplementationWaveIndex + "-" + verificationRepairAttempt, [taskUniverse, verificationPlan.items, verification, implementationEvidence], {
          tier: "reducer",
          task: "Repair failed focused verification into a concrete retry plan, preferably one item per exact command/check, with item_id, canonical_task_ids, focused_verification, expected_evidence, optional artifact_requirements, and source_item_id. Do not require exactly one cargo pass section; require at least one intended target pass and no intended target failure, or return concrete evidence that the implementation cannot be verified safely."
        });
        recordRepairAttempt(repairAttempts, "verification-repair-plan-" + currentImplementationWaveIndex + "-" + verificationRepairAttempt, "verification_repair", verification.outcomes || verification.items || [verification], verificationRepairPlan);
        if (!verificationRepairPlan.items || verificationRepairPlan.items.length === 0) {
          break;
        }
        verificationRepairPlan.items = splitFocusedVerificationItems(verificationRepairPlan.items);
        verification = await w.parallel("verification-wave-" + currentImplementationWaveIndex + "-" + verificationRepairAttempt, verificationRepairPlan.items, {
          tier: "coder",
          itemKind: "focused_verification",
          task: "Run repaired focused verification only. Return structured status, item_id, canonical_task_ids, focused_verification executed, commands run with exit_code and output_summary, matched test/check names, pass/fail count, artifacts checked, source_item_id, and residual gaps. Accept duplicate cargo workspace/lib/bin harness pass sections as one canonical pass when at least one intended target passes and no intended target fails. Do not modify files."
        });
        verificationEvidence.push({ kind: "verification-retry", implementationWaveIndex: currentImplementationWaveIndex, dependencyIteration, verificationRepairAttempt, verificationPlan: verificationRepairPlan, result: verification });
        verificationRepairAttempt += 1;
      }
      if (verification.status !== "accepted" && verification.status !== "noop") {
        return await w.finalReport("blocked-verification-failed-" + currentImplementationWaveIndex, {
          status: "needs_review",
          inputs: { taskUniverse, readyImplementationItems, implementationCandidateIdsUnique, implementationEvidence, verificationEvidence, verification, repair_attempts: repairAttempts },
          task: "Stop because bounded JS-owned verification repair did not produce accepted/noop evidence. Report exact commands/results and unresolved gaps."
        });
      }
      for (const id of acceptedOrNoopCanonicalTaskIdsFrom(verification.outcomes || verification.items)) {
        if (implementationCandidateIdsUnique.includes(id)) {
          acceptedThisWave.add(id);
        }
      }
    }
    let newlyCompletedIds = Array.from(acceptedThisWave).filter((id) => !completedIds.has(id));
    if (newlyCompletedIds.length === 0) {
      let completionEvidenceRepair = await w.reduce("wave-completion-evidence-repair-" + dependencyIteration, [taskUniverse, readyItems, readyNoopItems, readyImplementationItems, implementationEvidence, verificationEvidence], {
        tier: "reducer",
        task: "Repair a dependency-ready wave that produced no newly completed canonical task IDs. Return concrete completion evidence to re-check, or exact evidence-backed gaps that prevent safe unblocking."
      });
      recordRepairAttempt(repairAttempts, "wave-completion-evidence-repair-" + dependencyIteration, "completion_evidence_repair", readyItems, completionEvidenceRepair);
      completionEvidenceRepair = normalizeGeneratedInventory(completionEvidenceRepair);
      newlyCompletedIds = matchingAcceptedCompletionIds(readyItems, completionEvidenceRepair.items || completionEvidenceRepair.outcomes).filter((id) => !completedIds.has(id));
      if (newlyCompletedIds.length === 0) {
        return await w.finalReport("blocked-no-completion-" + dependencyIteration, {
          status: "needs_review",
          inputs: { taskUniverse, readyItems, readyNoopItems, readyImplementationItems, wave, completionEvidenceRepair, implementationEvidence, verificationEvidence, repair_attempts: repairAttempts },
          task: "Stop because the dependency-ready wave produced no new verified accepted/noop canonical task IDs after JS-owned completion evidence repair."
        });
      }
    }
    for (const id of newlyCompletedIds) {
      completedIds.add(id);
    }
    remainingItems = remainingItems.filter((item) => !itemIsCompleted(item, completedIds));
    if (readyImplementationItems.length > 0) {
      implementationWaveIndex += 1;
    }
    dependencyIteration += 1;
  }

  if (remainingItems.length > 0) {
    return await w.finalReport("blocked-loop-exhaustion-" + dependencyIteration, {
      status: "blocked",
      inputs: { taskUniverse, remainingItems, completed_ids: Array.from(completedIds), implementationEvidence, verificationEvidence, repair_attempts: repairAttempts },
      task: "Stop because the bounded dependency loop ended with unresolved implementation items. Report unresolved task IDs and dependency evidence."
    });
  }
