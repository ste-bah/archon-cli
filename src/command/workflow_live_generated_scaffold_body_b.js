  const artifactInventory = await w.reduce("artifact-inventory", [taskUniverse, inventory, implementationEvidence, verificationEvidence], {
    tier: "reducer",
    task: "List every required generated dataset, registry entry, validation report, coverage matrix, Pine artifact, backtest artifact, and final evidence artifact. Include only real paths that should exist."
  });
  const savedArtifactInventory = await w.saveArtifact("save-artifact-inventory", artifactInventory, {
    task: "Persist the artifact inventory and evidence references for final acceptance."
  });
  artifactEvidence.push({ kind: "artifact-inventory", artifactInventory, savedArtifactInventory });

  let reviewIteration = 1;
  let review = await w.reduce("adversarial-review-" + reviewIteration, [taskUniverse, implementationEvidence, verificationEvidence, artifactEvidence, governedLearningContext], {
    tier: "reviewer",
    task: "Run read-only adversarial review against the PRD and every TASK acceptance criterion. Use governedLearningContext only to check known prior failure classes. Return accepted/noop only if no gaps remain; otherwise return issue items requiring remediation. Do not modify files."
  });
  reviewEvidence.push({ kind: "review", reviewIteration, result: review });

  function normalizeReviewRemediationInventory(value) {
    const source = value && typeof value === "object" ? value : {};
    const items = generatedContractInventorySourceItems(source).map((item) => normalizeGeneratedItem(item));
    const sourceIssues = generatedContractInventorySourceIssues(source);
    return { ...source, items, unresolved_issues: sourceIssues.concat(items.flatMap((item) => reviewRemediationItemIssues(item))) };
  }

  function reviewNeedsRemediation(value) {
    if (!value || value.status === "accepted" || value.status === "noop") return false;
    return array(value.items).length > 0 || array(value.residual_gaps).length > 0 || array(value.evidence).length > 0 || generatedContractPresent(value.summary);
  }

  function reviewRemediationInput(value) {
    return array(value && value.items).length > 0 ? value.items : value;
  }

  function reviewRemediationItemIssues(item) {
    const issues = [];
    if (!item || !(item.item_id || item.id)) issues.push(generatedContractIssue("review_remediation_shape_repair", item || {}, "item_id", "review remediation item is missing item_id/id"));
    if (!item || canonicalIdsFor(item).length === 0) issues.push(generatedContractIssue("review_remediation_task_reconcile", item || {}, "canonical_task_ids", "review remediation item must use canonical taskUniverse IDs, not synthetic issue IDs"));
    for (const dep of array(item && item.dependency_ids)) if (!canonicalTaskUniverse.has(dep)) issues.push(generatedContractIssue("review_remediation_task_reconcile", item, "dependency_ids", "review remediation dependency is not a canonical task ID"));
    for (const field of ["source_item_id", "failure_status", "failure_evidence", "required_fix", "focused_verification"]) {
      if (!generatedContractPresent(item && item[field])) issues.push(generatedContractIssue("review_remediation_shape_repair", item || {}, field, "review remediation item is missing " + field));
    }
    if (!item || item.target_files === undefined) issues.push(generatedContractIssue("review_remediation_target_discovery", item || {}, "target_files", "review remediation item must include target_files, using [] only for artifact-only remediation"));
    for (const target of generatedContractRawStrings(item || {}, ["target_files"])) {
      const issue = generatedContractTargetFileIssue(target);
      if (issue) issues.push(generatedContractIssue("review_remediation_target_discovery", item, "target_files", issue + ": " + target));
    }
    if (!item || item.artifact_requirements === undefined) issues.push(generatedContractIssue("review_remediation_artifact_discovery", item || {}, "artifact_requirements", "review remediation item is missing artifact requirements"));
    if (generatedContractRawStrings(item || {}, ["target_files"]).length === 0 && !generatedContractPresent(item && item.artifact_requirements)) issues.push(generatedContractIssue("review_remediation_artifact_discovery", item || {}, "artifact_requirements", "artifact-only remediation needs concrete artifact requirements"));
    return issues;
  }

  while (reviewNeedsRemediation(review) && reviewIteration <= 6) {
    let reviewRemediationInventory = await w.reduce("review-remediation-inventory-" + reviewIteration, reviewRemediationInput(review), {
      tier: "reducer",
      task: "Turn adversarial review issues into dependency-safe remediation items. Every item must include item_id, canonical_task_ids using taskUniverse canonical task IDs only, dependency_ids using canonical task IDs only, source_item_id, failure_status, failure_evidence, required_fix, target_files, focused_verification, and artifact_requirements. Use target_files only for repo-owned implementation files; artifact-only remediation must set target_files to [] and put concrete project artifact requirements in artifact_requirements/evidence. Do not invent remediation IDs as canonical_task_ids."
    });
    reviewRemediationInventory = normalizeReviewRemediationInventory(reviewRemediationInventory);
    let reviewRemediationRepairAttempt = 1;
    while (array(reviewRemediationInventory.unresolved_issues).length > 0 && reviewRemediationRepairAttempt <= maxRepairIterations) {
      const reviewRemediationRepair = await w.reduce("review-remediation-inventory-repair-" + reviewIteration + "-" + reviewRemediationRepairAttempt, [taskUniverse, review, reviewRemediationInventory, reviewRemediationInventory.unresolved_issues, implementationEvidence, verificationEvidence, artifactEvidence], {
        tier: "reducer",
        task: "Repair review remediation inventory shape before write fanout. Return full replacement items only. Use canonical taskUniverse IDs in canonical_task_ids/dependency_ids. Include target_files only for repo-owned source edits; use [] for artifact-only remediation with concrete artifact_requirements. Preserve failure context, required_fix, focused_verification, and artifact requirements. Do not invent synthetic canonical task IDs."
      });
      recordRepairAttempt(repairAttempts, "review-remediation-inventory-repair-" + reviewIteration + "-" + reviewRemediationRepairAttempt, "review_remediation_shape_repair", reviewRemediationInventory.unresolved_issues, reviewRemediationRepair);
      reviewRemediationInventory = normalizeReviewRemediationInventory(reviewRemediationRepair);
      reviewRemediationRepairAttempt += 1;
    }
    if (!reviewRemediationInventory.items || reviewRemediationInventory.items.length === 0 || array(reviewRemediationInventory.unresolved_issues).length > 0) {
      return await w.finalReport("blocked-empty-review-remediation-" + reviewIteration, {
        status: "needs_review",
        inputs: { taskUniverse, review, reviewRemediationInventory, reviewEvidence, repair_attempts: repairAttempts },
        task: "Stop because adversarial review found issues but review remediation inventory is still empty or malformed after JS-owned repair."
      });
    }
    const reviewFixes = await w.fanout("review-remediation-wave-" + reviewIteration, reviewRemediationInventory.items, {
      tier: "coder",
      itemKind: "implementation",
      write: "worktree",
      maxParallelism: "configured",
      targetFilesFromItem: true,
      task: "Fix only the assigned adversarial review issue. Return canonical task IDs, status, concrete evidence, commands/tests, artifacts, and residual gaps."
    });
    implementationEvidence.push({ kind: "review-remediation", reviewIteration, reviewRemediationInventory, result: reviewFixes });
    const reviewVerificationPlan = await w.reduce("review-verification-plan-" + reviewIteration, [taskUniverse, reviewFixes, implementationEvidence], {
      tier: "reducer",
      task: "Plan focused verification for review remediation before final review can pass. Return one item per exact command/check whenever possible. Every item must include item_id, canonical_task_ids, dependency_ids using canonical task IDs only, source_item_id or source_call_id when available, focused_verification, expected_evidence, and optional artifact_requirements. Evidence is accepted when at least one intended target passes and no intended target fails; duplicate cargo workspace/lib/bin harness pass sections are valid as one canonical pass."
    });
    if (!reviewVerificationPlan.items || reviewVerificationPlan.items.length === 0) {
      return await w.finalReport("blocked-empty-review-verification-" + reviewIteration, {
        status: "needs_review",
        inputs: { taskUniverse, review, reviewFixes, reviewVerificationPlan, repair_attempts: repairAttempts },
        task: "Stop because review remediation had no focused verification plan."
      });
    }
    reviewVerificationPlan.items = splitFocusedVerificationItems(reviewVerificationPlan.items);
    const reviewVerification = await w.parallel("review-verification-wave-" + reviewIteration, reviewVerificationPlan.items, {
      tier: "coder",
      task: "Run focused verification for review remediation. Return commands with exit_code and output_summary, matched test/check names, pass/fail count, artifacts checked, and residual gaps. Accept duplicate cargo workspace/lib/bin harness pass sections as one canonical pass when at least one intended target passes and no intended target fails. Do not modify files."
    });
    verificationEvidence.push({ kind: "review-verification", reviewIteration, reviewVerificationPlan, result: reviewVerification });
    if (reviewVerification.status !== "accepted" && reviewVerification.status !== "noop") {
      return await w.finalReport("blocked-review-verification-failed-" + reviewIteration, {
        status: "needs_review",
        inputs: { taskUniverse, reviewFixes, reviewVerification, implementationEvidence, verificationEvidence, repair_attempts: repairAttempts },
        task: "Stop because review remediation verification failed."
      });
    }
    reviewIteration += 1;
    review = await w.reduce("adversarial-review-" + reviewIteration, [taskUniverse, implementationEvidence, verificationEvidence, reviewEvidence, artifactEvidence, governedLearningContext], {
      tier: "reviewer",
      task: "Re-run read-only adversarial review. Use governedLearningContext only to prevent repeating known evidenced failure classes. Return accepted/noop only when all PRD/TASK criteria have concrete evidence and artifact paths exist. Do not modify files."
    });
    reviewEvidence.push({ kind: "review", reviewIteration, result: review });
  }

  if (reviewNeedsRemediation(review)) {
    return await w.finalReport("blocked-review-unresolved", {
      status: "needs_review",
      inputs: { taskUniverse, review, implementationEvidence, verificationEvidence, reviewEvidence, artifactEvidence, repair_attempts: repairAttempts },
      task: "Stop because adversarial review issues remain after bounded remediation."
    });
  }
  if (review.status !== "accepted" && review.status !== "noop") {
    return await w.finalReport("blocked-review-not-accepted", {
      status: "needs_review",
      inputs: { taskUniverse, review, implementationEvidence, verificationEvidence, reviewEvidence, artifactEvidence, repair_attempts: repairAttempts },
      task: "Stop because adversarial review did not explicitly accept the evidence."
    });
  }

  let finalEvidenceIteration = 1;
  let finalEvidenceReconciliation = await w.reduce("final-evidence-reconciliation-" + finalEvidenceIteration, [taskUniverse, inventory, implementationEvidence, verificationEvidence, reviewEvidence, artifactEvidence, governedLearningContext], {
    tier: "reducer",
    task: "Reconcile final completion claims, artifact existence, verification evidence, provider/data evidence, and residual gaps before final acceptance. Return issue items only for unsupported completion claims, missing artifacts, or repairable evidence gaps."
  });
  while (finalEvidenceReconciliation.items && finalEvidenceReconciliation.items.length > 0 && finalEvidenceIteration <= maxRepairIterations) {
    const completionClaimRepair = await w.reduce("completion-claim-repair-" + finalEvidenceIteration, finalEvidenceReconciliation.items, {
      tier: "reducer",
      task: "Repair unsupported completion claims into concrete evidence requirements or residual gaps. Return updated evidence references and artifact checks without claiming completion without proof."
    });
    recordRepairAttempt(finalEvidenceRepairAttempts, "completion-claim-repair-" + finalEvidenceIteration, "completion_claim_repair", finalEvidenceReconciliation.items, completionClaimRepair);
    const artifactChecks = array(completionClaimRepair.artifact_checks || completionClaimRepair.items).filter((item) => generatedContractPresent(item && (item.path || item.artifact_path || item.artifactPath || item.artifact_id || item.artifactId)));
    if (artifactChecks.length > 0) {
      const artifactExistenceInvestigation = await w.parallel("artifact-existence-investigation-" + finalEvidenceIteration, artifactChecks, {
        tier: "analysis",
        task: "Investigate artifact existence and path evidence. Return artifact paths checked, existence status, commands run if any, and residual gaps. Do not modify files."
      });
      recordRepairAttempt(finalEvidenceRepairAttempts, "artifact-existence-investigation-" + finalEvidenceIteration, "artifact_existence_investigation", artifactChecks, artifactExistenceInvestigation);
      artifactEvidence.push({ kind: "artifact-existence-investigation", finalEvidenceIteration, artifactChecks, result: artifactExistenceInvestigation });
    }
    finalEvidenceIteration += 1;
    finalEvidenceReconciliation = await w.reduce("final-evidence-reconciliation-" + finalEvidenceIteration, [taskUniverse, inventory, implementationEvidence, verificationEvidence, reviewEvidence, artifactEvidence, governedLearningContext, finalEvidenceRepairAttempts], {
      tier: "reducer",
      task: "Re-run final evidence reconciliation after completion-claim repair and artifact investigation. Return no issue items only when all completion claims have concrete proof."
    });
  }
  if (finalEvidenceReconciliation.items && finalEvidenceReconciliation.items.length > 0) {
    return await w.finalReport("blocked-final-evidence-reconciliation", {
      status: "needs_review",
      inputs: { taskUniverse, finalEvidenceReconciliation, final_evidence_repair_attempts: finalEvidenceRepairAttempts, implementationEvidence, verificationEvidence, reviewEvidence, artifactEvidence },
      task: "Stop because final evidence reconciliation still has unsupported completion claims, missing artifacts, or repairable evidence gaps after bounded JS-owned repair and investigation."
    });
  }

  const requiredArtifacts = await w.requireArtifact("require-final-artifacts", artifactInventory, {
    task: "Verify all referenced artifact paths required by the PRD and TASK files exist after final evidence reconciliation and artifact investigation."
  });
  artifactEvidence.push({ kind: "required-artifacts", requiredArtifacts });

  const finalAudit = await w.reduce("final-zero-gap-audit", [taskUniverse, inventory, implementationEvidence, verificationEvidence, reviewEvidence, artifactEvidence, requiredArtifacts, governedLearningContext, repairAttempts, finalEvidenceRepairAttempts, finalEvidenceReconciliation], {
    tier: "reducer",
    task: "Perform final zero-gap acceptance audit across all canonical tasks, PRD criteria, code changes, tests, provider/data evidence, artifacts, residual gaps, repair attempts, final evidence reconciliation, and governedLearningContext failure classes."
  });
  const finalGate = await w.qualityGate("final-acceptance-gate", [finalAudit, requiredArtifacts, finalEvidenceReconciliation], {
    task: "Accept only if every canonical task has concrete implementation or verified no-op evidence, focused tests, existing artifact paths, and no residual blocking gaps."
  });
  if (finalGate.status !== "accepted" && finalGate.status !== "noop") {
    return await w.finalReport("blocked-final-readiness", [finalGate, finalAudit, requiredArtifacts, finalEvidenceReconciliation], {
      status: "needs_review",
      inputs: { taskUniverse, finalAudit, finalGate, requiredArtifacts, implementationEvidence, verificationEvidence, reviewEvidence, artifactEvidence, repair_attempts: repairAttempts, final_evidence_repair_attempts: finalEvidenceRepairAttempts },
      task: "Stop because final acceptance gate did not accept the evidence. Report exact residual gaps."
    });
  }

  return await w.finalReport("final-acceptance-report", [finalGate, finalAudit, requiredArtifacts, finalEvidenceReconciliation], {
    status: "accepted",
    inputs: { taskUniverse, finalAudit, finalGate, requiredArtifacts, implementationEvidence, verificationEvidence, reviewEvidence, artifactEvidence, repair_attempts: repairAttempts, final_evidence_repair_attempts: finalEvidenceRepairAttempts },
    task: "Produce the final acceptance report with canonical completed task IDs, source files changed, artifact paths, focused test commands/results, provider/data evidence, residual gaps if any, and explicit confirmation that no task was accepted without concrete evidence."
  });
}
