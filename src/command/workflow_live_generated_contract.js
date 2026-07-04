  function generatedContractArray(value) {
    return Array.isArray(value) ? value : [];
  }

  function generatedContractStrings(value) {
    if (Array.isArray(value)) {
      return value.flatMap((entry) => generatedContractStrings(entry));
    }
    if (typeof value === "string") {
      return value.split(",").map((entry) => entry.trim()).filter(Boolean);
    }
    if (value && typeof value === "object") {
      const selected = value.path || value.id || value.summary || value.command;
      return typeof selected === "string" && selected.trim() ? [selected.trim()] : [];
    }
    if (typeof value === "number" || typeof value === "boolean") {
      return [String(value)];
    }
    return [];
  }

  function generatedContractUnique(values) {
    return Array.from(new Set(generatedContractArray(values).map((value) => String(value).trim()).filter(Boolean))).sort();
  }

  function generatedContractRawValues(item, keys) {
    const values = [];
    for (const key of keys) {
      const value = item && item[key];
      if (Array.isArray(value)) {
        values.push(...value);
      } else if (value !== undefined && value !== null) {
        values.push(value);
      }
    }
    return values;
  }

  function generatedContractRawStrings(item, keys) {
    return generatedContractUnique(generatedContractRawValues(item, keys).flatMap((value) => generatedContractStrings(value)));
  }

  function generatedContractResolveTaskId(value) {
    if (!value) {
      return null;
    }
    const trimmed = String(value).trim();
    if (!trimmed) {
      return null;
    }
    if (canonicalTaskUniverse.has(trimmed)) {
      return trimmed;
    }
    const match = generatedContractArray(taskUniverse.tasks || []).find((task) => {
      return task && generatedContractArray(task.aliases).includes(trimmed);
    });
    return match ? match.canonical_task_id : null;
  }

  function normalizeGeneratedItem(item) {
    const source = item && typeof item === "object" ? item : {};
    const out = { ...source };
    const itemId = source.item_id || source.itemId || source.id || source.task_id || source.taskId || source.work_unit_id || source.workUnitId || source.source_item_id || source.sourceItemId || source.failed_item_id || source.failedItemId || source.source_failed_item_id || source.sourceFailedItemId;
    if (itemId) {
      out.item_id = String(itemId).trim();
      out.id = out.item_id;
    }
    const workType = source.work_type || source.workType || source.item_type || source.itemType || source.kind;
    if (workType) out.work_type = String(workType).trim();
    const explicitTaskIds = generatedContractUnique(generatedContractRawStrings(source, ["canonical_task_ids", "canonicalTaskIds", "canonical_task_id", "canonicalTaskId", "task_ids", "taskIds", "task_id", "taskId"]).map((id) => generatedContractResolveTaskId(id) || id)); out.canonical_task_ids = explicitTaskIds.length > 0 ? explicitTaskIds : generatedContractEmbeddedTaskIds(source);
    out.dependency_ids = generatedContractUnique(generatedContractRawStrings(source, ["dependency_ids", "dependencyIds", "dependencies", "depends_on", "dependsOn", "canonical_dependency_ids", "canonicalDependencyIds"]).map((id) => generatedContractResolveTaskId(id) || "__unresolved__:" + id));
    const copyArray = (target, keys) => {
      const values = generatedContractRawValues(source, keys);
      if (values.length > 0) {
        out[target] = values;
      }
    };
    const copyValue = (target, keys) => {
      for (const key of keys) {
        if (source[key] !== undefined && source[key] !== null) {
          out[target] = source[key];
          return;
        }
      }
    };
    copyArray("target_files", ["target_files", "targetFiles", "files", "changed_files", "changedFiles", "owned_source_files", "ownedSourceFiles", "owned_test_files", "ownedTestFiles", "owned_manifest_files", "ownedManifestFiles", "owned_lockfiles", "ownedLockfiles", "owned_build_config_files", "ownedBuildConfigFiles", "owned_docs_config_files", "ownedDocsConfigFiles", "owned_generated_outputs", "ownedGeneratedOutputs"]);
    copyArray("acceptance_criteria", ["acceptance_criteria", "acceptanceCriteria", "criteria", "acceptance"]);
    copyArray("focused_verification", ["focused_verification", "focusedVerification", "focused_tests", "focusedTests", "verification", "verification_requirements", "verificationRequirements", "verification_shape", "verificationShape", "command", "check", "test_command", "testCommand", "commands", "commands_run", "commandsRun", "manual_fixture_steps", "manualFixtureSteps"]);
    if (source.required_evidence || source.requiredEvidence || source.expected_completion_evidence || source.expectedCompletionEvidence) {
      const requiredEvidence = source.required_evidence || source.requiredEvidence || source.expected_completion_evidence || source.expectedCompletionEvidence;
      const requiredFocused = generatedContractRawValues(requiredEvidence, ["focused_tests", "focusedTests", "direct_checks", "directChecks", "commands", "commands_run", "commandsRun", "required_summary_points", "requiredSummaryPoints"]);
      if (requiredFocused.length > 0) {
        out.focused_verification = generatedContractArray(out.focused_verification).concat(requiredFocused);
      }
    }
    copyArray("expected_evidence", ["expected_evidence", "expectedEvidence", "expected_acceptance", "expectedAcceptance", "required_evidence", "requiredEvidence", "evidence_to_capture", "evidenceToCapture", "expected_result", "expectedResult"]);
    copyArray("artifact_requirements", ["artifact_requirements", "artifactRequirements", "artifacts", "required_artifacts", "requiredArtifacts", "expected_artifacts", "expectedArtifacts", "artifact_checks", "artifactChecks", "project_artifact_requirements", "projectArtifactRequirements"]);
    if (source.required_evidence || source.requiredEvidence || source.expected_completion_evidence || source.expectedCompletionEvidence) {
      const requiredEvidence = source.required_evidence || source.requiredEvidence || source.expected_completion_evidence || source.expectedCompletionEvidence;
      const requiredArtifacts = generatedContractRawValues(requiredEvidence, ["artifact_paths", "artifactPaths", "artifacts", "expected_artifacts", "expectedArtifacts", "artifact_checks", "artifactChecks"]);
      if (requiredArtifacts.length > 0) {
        out.artifact_requirements = generatedContractArray(out.artifact_requirements).concat(requiredArtifacts);
      }
    }
    generatedContractApplyRetryAliases(source, out);
    copyArray("noop_proof_refs", ["noop_proof_refs", "noopProofRefs", "proof_references", "proofReferences", "proof_refs", "proofRefs"]);
    copyValue("noop_proof", ["noop_proof", "noopProof", "proof", "noop_evidence", "noopEvidence"]);
    copyArray("provider_evidence", ["provider_evidence", "providerEvidence", "provider_env_evidence", "providerEnvEvidence", "environment_evidence", "environmentEvidence"]);
    generatedContractApplyProviderEnvAliases(source, out);
    copyValue("evidence", ["evidence", "proof", "proof_references", "proofReferences"]);
    if (!out.source_item_id) {
      out.source_item_id = source.source_item_id || source.sourceItemId || source.source_issue_id || source.sourceIssueId || source.issue_id || source.issueId || source.gap_id || source.gapId || out.item_id;
    }
    if (!out.failure_status) {
      out.failure_status = source.failure_status || source.failureStatus || source.status || source.blocker_status || source.blockerStatus || (source.acceptance_blocker || source.blocker || source.required_evidence || source.requiredEvidence ? "needs_review" : undefined);
    }
    if (!out.failure_evidence) {
      const failureEvidence = generatedContractRawValues(source, ["failure_evidence", "failureEvidence", "failure_kind", "failureKind", "verification_failure_class", "verificationFailureClass", "evidence", "acceptance_blocker", "acceptanceBlocker", "blocker", "residual_gaps", "residualGaps", "gaps"]);
      if (failureEvidence.length > 0) {
        out.failure_evidence = failureEvidence;
      }
    }
    if (!out.required_fix) {
      out.required_fix = source.required_fix || source.requiredFix || source.fix || source.remediation || source.title || source.summary || source.acceptance_blocker || source.acceptanceBlocker || source.blocker;
    }
    if (!out.verification_requirements) {
      const verificationRequirements = generatedContractRawValues(source, ["verification_requirements", "verificationRequirements", "verification", "verification_shape", "verificationShape", "focused_verification", "focusedVerification", "focused_tests", "focusedTests"]);
      if (verificationRequirements.length > 0) {
        out.verification_requirements = verificationRequirements;
      }
    }
    return out;
  }

  function generatedContractPresent(value) {
    if (typeof value === "string") {
      return value.trim().length > 0;
    }
    if (Array.isArray(value)) {
      return value.length > 0;
    }
    if (value && typeof value === "object") {
      return Object.keys(value).length > 0;
    }
    return value === true || typeof value === "number";
  }
  function splitFocusedVerificationItems(items) {
    const split = [];
    for (const rawItem of generatedContractArray(items)) {
      const item = normalizeGeneratedItem(rawItem);
      if (item.expected_completion_evidence || item.expectedCompletionEvidence || item.expected_result_shape || item.expectedResultShape || item.retry_command_shape || item.retryCommandShape || item.split_verification_items === false || item.repair_type === "verification_evidence_shape") {
        split.push(item);
        continue;
      }
      const checks = generatedContractStrings(item.focused_verification);
      if (checks.length <= 1) {
        split.push(item);
        continue;
      }
      const baseId = item.item_id || item.id || "verification";
      checks.forEach((check, index) => {
        split.push({
          ...item,
          item_id: `${baseId}-check-${index + 1}`,
          id: `${baseId}-check-${index + 1}`,
          focused_verification: [check],
          source_item_id: item.source_item_id || baseId,
          split_from_item_id: baseId
        });
      });
    }
    return split;
  }

  function generatedContractNormalizePath(value) {
    const pathText = String(value || "").trim().replace(/\\/g, "/");
    if (!pathText) {
      return "";
    }
    const absolute = pathText.startsWith("/") || /^[A-Za-z]:\//.test(pathText);
    const parts = [];
    for (const part of pathText.split("/")) {
      if (!part || part === ".") {
        continue;
      }
      if (part === "..") {
        if (parts.length > 0 && parts[parts.length - 1] !== "..") {
          parts.pop();
        } else if (!absolute) {
          parts.push("..");
        }
        continue;
      }
      parts.push(part);
    }
    if (absolute) {
      return parts.length === 0 ? "/" : "/" + parts.join("/");
    }
    return parts.length === 0 ? "." : parts.join("/");
  }

  function generatedContractTargetFileIssue(target) {
    const pathText = String(target || "").trim();
    if (!pathText) {
      return "implementation item has an empty target file";
    }
    if (!targetRepositoryRoot) {
      return null;
    }
    const root = generatedContractNormalizePath(targetRepositoryRoot);
    if (!root) {
      return null;
    }
    const normalized = generatedContractNormalizePath(pathText);
    const absolute = pathText.startsWith("/") || /^[A-Za-z]:[\\/]/.test(pathText);
    if (absolute) {
      if (normalized === root) {
        return "implementation target_files must name repo-owned files, not the repository root";
      }
      if (!normalized.startsWith(root + "/")) {
        return "implementation target file is outside target repository root";
      }
    } else if (normalized === "." || normalized === ".." || normalized.startsWith("../")) {
      return "implementation target file escapes target repository root";
    }
    return null;
  }

  function generatedContractTargetFilesIssue(item) {
    const targets = generatedContractRawStrings(item || {}, ["target_files"]);
    if (targets.length === 0) {
      return "implementation item is missing target files";
    }
    for (const target of targets) {
      const issue = generatedContractTargetFileIssue(target);
      if (issue) {
        return issue + ": " + target;
      }
    }
    return null;
  }

  function generatedContractIssue(kind, item, field, message) {
    return {
      kind,
      field,
      message,
      item_id: item.item_id || item.id || null,
      canonical_task_ids: generatedContractArray(item.canonical_task_ids)
    };
  }

  function generatedContractTaskDependencies(taskId) {
    const task = generatedContractArray(taskUniverse.tasks || []).find((candidate) => candidate && candidate.canonical_task_id === taskId);
    return generatedContractUnique(generatedContractArray(task && task.dependency_ids).map((id) => generatedContractResolveTaskId(id) || id));
  }

  function generatedContractIsSupportItem(item) {
    const workType = String((item && (item.work_type || item.workType || item.kind)) || "").toLowerCase();
    return workType.endsWith("_support") && generatedContractArray(item && item.canonical_task_ids).length === 0;
  }

  function generatedContractInventoryGraphIssues(items, completedIds) {
    const issues = [];
    if (generatedContractArray(items).length > 0 && generatedContractArray(items).every((item) => generatedContractFocusedVerificationItem(item))) return issues;
    const completedSet = new Set(generatedContractArray(completedIds));
    const claimsByTask = new Map();
    for (const item of generatedContractArray(items)) {
      for (const taskId of generatedContractArray(item && item.canonical_task_ids)) {
        if (!claimsByTask.has(taskId)) {
          claimsByTask.set(taskId, []);
        }
        claimsByTask.get(taskId).push(item);
      }
      const claimed = new Set(generatedContractArray(item && item.canonical_task_ids));
      for (const dep of generatedContractArray(item && item.dependency_ids)) {
        if (claimed.has(dep)) {
          issues.push(generatedContractIssue("dependency_graph_repair", item, "dependency_ids", "inventory item dependency '" + dep + "' is also claimed by the same item"));
        }
      }
      for (const taskId of generatedContractArray(item && item.canonical_task_ids)) {
        for (const dep of generatedContractTaskDependencies(taskId)) {
          if (claimed.has(dep)) {
            issues.push(generatedContractIssue("dependency_graph_repair", item, "canonical_task_ids", "inventory item groups '" + taskId + "' with its prerequisite '" + dep + "'"));
          }
        }
      }
    }
    for (const [taskId, claims] of claimsByTask.entries()) {
      if (claims.length > 1) {
        for (const item of claims) {
          issues.push(generatedContractIssue("dependency_graph_repair", item, "canonical_task_ids", "canonical task '" + taskId + "' is assigned to multiple inventory items"));
        }
      }
    }
    const allClaimed = new Set(Array.from(claimsByTask.keys()));
    for (const taskId of canonicalTaskUniverse) {
      if (!allClaimed.has(taskId) && !completedSet.has(taskId)) {
        issues.push({
          kind: "dependency_graph_repair",
          field: "canonical_task_ids",
          message: "canonical task '" + taskId + "' is not represented by an implementation or verified_noop item",
          item_id: null,
          canonical_task_ids: [taskId]
        });
      }
    }
    for (const item of generatedContractArray(items)) {
      const required = new Set(generatedContractArray(item && item.dependency_ids));
      for (const taskId of generatedContractArray(item && item.canonical_task_ids)) {
        for (const dep of generatedContractTaskDependencies(taskId)) {
          required.add(dep);
        }
      }
      for (const dep of required) {
        if (String(dep).startsWith("__unresolved__:") || !canonicalTaskUniverse.has(dep)) {
          continue;
        }
        if (!allClaimed.has(dep) && !completedSet.has(dep)) {
          issues.push(generatedContractIssue("dependency_graph_repair", item, "dependency_ids", "inventory dependency '" + dep + "' is not represented by an implementation or verified_noop item"));
        }
      }
    }
    const seen = new Set();
    return issues.filter((issue) => {
      const key = JSON.stringify([issue.kind, issue.field, issue.message, issue.item_id, issue.canonical_task_ids]);
      if (seen.has(key)) {
        return false;
      }
      seen.add(key);
      return true;
    });
  }

  function generatedContractItemIssues(item) {
    const issues = [];
    if (generatedContractIsSupportItem(item)) {
      return issues;
    }
    const workType = item && item.work_type;
    if (!item || !(item.item_id || item.id)) {
      issues.push(generatedContractIssue("inventory_shape_repair", item || {}, "item_id", "inventory item is missing item_id/id"));
    }
    if (!item || generatedContractArray(item.canonical_task_ids).length === 0) {
      issues.push(generatedContractIssue("inventory_shape_repair", item || {}, "canonical_task_ids", "inventory item is missing canonical task IDs"));
    }
    for (const taskId of generatedContractArray(item && item.canonical_task_ids)) {
      if (!canonicalTaskUniverse.has(taskId)) {
        issues.push(generatedContractIssue("task_universe_reconcile", item, "canonical_task_ids", "inventory item has task IDs outside the canonical task universe"));
      }
    }
    for (const dep of generatedContractArray(item && item.dependency_ids)) {
      if (String(dep).startsWith("__unresolved__:")) {
        issues.push(generatedContractIssue("task_universe_reconcile", item, "dependency_ids", "inventory item has unresolved dependency IDs"));
      }
    }
    if (generatedContractFocusedVerificationItem(item)) return issues;
    if (workType === "implementation") {
      const targetFileIssue = generatedContractTargetFilesIssue(item);
      if (targetFileIssue) {
        issues.push(generatedContractIssue("target_file_discovery", item, "target_files", targetFileIssue));
      }
      if (!generatedContractPresent(item.acceptance_criteria)) {
        issues.push(generatedContractIssue("verification_requirements_discovery", item, "acceptance_criteria", "implementation item is missing acceptance criteria"));
      }
      if (!generatedContractPresent(item.focused_verification)) {
        issues.push(generatedContractIssue("verification_requirements_discovery", item, "focused_verification", "implementation item is missing focused verification requirements"));
      }
      if (item.artifact_requirements === undefined) {
        issues.push(generatedContractIssue("artifact_requirements_discovery", item, "artifact_requirements", "implementation item is missing artifact requirements"));
      }
    } else if (workType === "verified_noop") {
      if (!generatedContractPresent(item.acceptance_criteria)) {
        issues.push(generatedContractIssue("verification_requirements_discovery", item, "acceptance_criteria", "verified_noop item is missing acceptance criteria"));
      }
      if (!generatedContractPresent(item.noop_proof) || !generatedContractPresent(item.noop_proof_refs)) {
        issues.push(generatedContractIssue("evidence_repair", item, "noop_proof_refs", "verified_noop item is missing no-op proof or proof references"));
      }
      if (item.artifact_requirements === undefined) {
        issues.push(generatedContractIssue("artifact_requirements_discovery", item, "artifact_requirements", "verified_noop item is missing artifact requirements metadata"));
      }
    } else {
      issues.push(generatedContractIssue("inventory_shape_repair", item || {}, "work_type", "inventory item work_type must be implementation or verified_noop"));
    }
    if (item && item.provider_required === true && !generatedContractPresent(item.provider_evidence) && !generatedContractPresent(item.provider_env_requirements) && !generatedContractPresent(item.provider_env_proof)) {
      issues.push(generatedContractIssue("provider_environment_discovery", item, "provider_evidence", "provider-dependent item is missing provider/environment evidence"));
    }
    return issues;
  }

  function normalizeGeneratedInventory(value) {
    const source = value && typeof value === "object" ? value : {};
    const rawItems = generatedContractInventorySourceItems(source);
    const normalizedItems = rawItems.map((item) => normalizeGeneratedItem(item));
    const support_items = normalizedItems.filter((item) => generatedContractIsSupportItem(item));
    const items = normalizedItems.filter((item) => !generatedContractIsSupportItem(item));
    const sourceIssues = generatedContractInventorySourceIssues(source);
    const unresolved_issues = sourceIssues.concat(items.flatMap((item) => generatedContractItemIssues(item))).concat(generatedContractInventoryGraphIssues(items));
    return { ...source, items, support_items, unresolved_issues };
  }

  function issuesOfKind(inventory, kind) {
    return generatedContractArray(inventory && inventory.unresolved_issues).filter((issue) => issue && issue.kind === kind);
  }

  function mergeInventoryRepair(inventory, repair) {
    const data = repair && repair.data;
    const dataItems = data && data.items;
    const repairItems = generatedContractArray((repair && repair.items) || (repair && repair.inventory && repair.inventory.items) || (data && data.items)).concat(generatedContractArray(data && data.repaired_items), generatedContractArray(data && data.implementation_items), generatedContractArray(data && data.verified_noop_items), generatedContractArray(dataItems && dataItems.repaired_items), generatedContractArray(dataItems && dataItems.implementation_items), generatedContractArray(dataItems && dataItems.verified_noop_items));
    if (repairItems.length === 0) {
      return inventory;
    }
    const existingItems = generatedContractArray(inventory && inventory.items).map((item) => normalizeGeneratedItem(item));
    const mergedByKey = new Map();
    const order = [];
    const itemKeys = (item) => {
      const keys = [];
      if (item && item.item_id) {
        keys.push("item:" + item.item_id);
      }
      for (const id of generatedContractArray(item && item.canonical_task_ids)) {
        keys.push("task:" + id);
      }
      return keys;
    };
    const primaryKey = (item) => {
      const keys = itemKeys(item);
      return keys.length > 0 ? keys[0] : null;
    };
    const putItem = (item) => {
      const key = primaryKey(item);
      if (!key) {
        return;
      }
      if (!mergedByKey.has(key)) {
        order.push(key);
      }
      mergedByKey.set(key, item);
      for (const alias of itemKeys(item)) {
        mergedByKey.set(alias, item);
      }
    };
    for (const item of existingItems) {
      putItem(item);
    }
    for (const rawRepairItem of repairItems) {
      const repairItem = normalizeGeneratedItem(rawRepairItem);
      const keys = itemKeys(repairItem);
      const matched = keys.find((key) => mergedByKey.has(key));
      const tombstone = repairItem.remove === true || repairItem.tombstone === true || repairItem.deleted === true;
      if (matched && tombstone && generatedContractPresent(repairItem.evidence)) {
        const existing = mergedByKey.get(matched);
        const existingPrimary = primaryKey(existing);
        if (existingPrimary) {
          mergedByKey.delete(existingPrimary);
          const orderIndex = order.indexOf(existingPrimary);
          if (orderIndex >= 0) {
            order.splice(orderIndex, 1);
          }
        }
        for (const alias of itemKeys(existing)) {
          mergedByKey.delete(alias);
        }
        continue;
      }
      if (matched) {
        const existing = mergedByKey.get(matched);
        const existingPrimary = primaryKey(existing);
        const merged = { ...existing, ...repairItem };
        if (existingPrimary) {
          mergedByKey.set(existingPrimary, merged);
        }
        for (const alias of itemKeys(existing).concat(itemKeys(merged))) {
          mergedByKey.set(alias, merged);
        }
        continue;
      }
      putItem(repairItem);
    }
    return { ...inventory, unresolved_issues: [], items: order.map((key) => mergedByKey.get(key)).filter(Boolean) };
  }

  function recordRepairAttempt(attempts, call_id, issue_kind, issues, result) {
    attempts.push({
      call_id,
      issue_kind,
      canonical_task_ids: generatedContractUnique(generatedContractArray(issues).flatMap((issue) => generatedContractArray(issue.canonical_task_ids))),
      files_read: generatedContractRawStrings(result || {}, ["files_read", "filesRead"]),
      commands_run: generatedContractRawStrings(result || {}, ["commands_run", "commandsRun", "commands"]),
      artifact_paths_checked: generatedContractRawStrings(result || {}, ["artifact_paths", "artifactPaths", "artifacts"]),
      redacted_env_keys_checked: generatedContractRawStrings(result || {}, ["env_keys_checked", "envKeysChecked", "redacted_env_keys_checked", "redactedEnvKeysChecked"]),
      evidence_refs: generatedContractRawStrings(result || {}, ["evidence_refs", "evidenceRefs", "proof_references", "proofReferences", "proof_refs", "proofRefs"]),
      reason: result && result.summary ? result.summary : "repair or investigation result recorded"
    });
  }
