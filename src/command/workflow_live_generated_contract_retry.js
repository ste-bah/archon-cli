  function generatedContractFocusedVerificationItem(item) {
    return item && generatedContractVerificationIntent(item)
      && generatedContractPresent(item.focused_verification)
      && (item.expected_evidence !== undefined || item.artifact_requirements !== undefined)
      && !generatedContractPresent(item.target_files);
  }

  function generatedContractVerificationIntent(item) {
    const workType = String((item && (item.work_type || item.workType)) || "").toLowerCase();
    if (!workType || workType.includes("verification")) return true;
    return workType === "verified_noop" && generatedContractRetryVerificationIntent(item);
  }

  function generatedContractRetryVerificationIntent(item) {
    return generatedContractPresent(item && (item.retry_command_shape || item.retryCommandShape))
      || generatedContractPresent(item && (item.retry_steps || item.retrySteps))
      || generatedContractPresent(item && (item.expected_result_shape || item.expectedResultShape))
      || generatedContractPresent(item && (item.retry_plan || item.retryPlan));
  }

  function generatedContractEmbeddedTaskIds(source) {
    const haystack = generatedContractRawStrings(source, ["item_id", "id", "source_item_id", "sourceItemId", "failed_item_id", "failedItemId", "source_failed_item_id", "sourceFailedItemId"]).join(" ").toLowerCase();
    return generatedContractUnique(generatedContractArray(taskUniverse.tasks || []).map((task) => task && task.canonical_task_id).filter((id) => {
      return id && generatedContractEmbeddedTaskCandidates(String(id)).some((candidate) => haystack.includes(candidate));
    }));
  }

  function generatedContractEmbeddedTaskCandidates(canonical) {
    const parts = String(canonical).split("-");
    const candidates = [String(canonical).toLowerCase()];
    if (parts.length > 2) candidates.push(parts.slice(1).join("-").toLowerCase());
    const digits = parts[parts.length - 1] || "";
    if (/^[0-9]+$/.test(digits)) candidates.push(("T" + digits).toLowerCase());
    return generatedContractUnique(candidates);
  }

  function generatedContractInventoryRoots(source) {
    const roots = [source].filter(Boolean);
    for (const path of [["data"], ["result"], ["result", "data"], ["result", "result"], ["result", "result", "data"], ["data", "result"], ["data", "result", "data"]]) {
      const root = path.reduce((current, key) => current && current[key], source);
      if (root && typeof root === "object") roots.push(root);
    }
    return roots;
  }

  function generatedContractInventoryRootItems(root) {
    const dataItems = root && root.items;
    return generatedContractArray(root && root.items)
      .concat(generatedContractArray(root && root.inventory && root.inventory.items))
      .concat(generatedContractArray(root && root.repaired_items))
      .concat(generatedContractArray(root && root.implementation_items))
      .concat(generatedContractArray(root && root.verified_noop_items))
      .concat(generatedContractArray(dataItems && dataItems.repaired_items))
      .concat(generatedContractArray(dataItems && dataItems.implementation_items))
      .concat(generatedContractArray(dataItems && dataItems.verified_noop_items));
  }

  function generatedContractInventorySourceItems(source) {
    if (Array.isArray(source && source.items)) {
      return source.items;
    }
    return generatedContractInventoryRoots(source)
      .flatMap((root) => generatedContractInventoryRootItems(root))
      .concat(generatedContractInventoryRetryItems(source));
  }

  function generatedContractInventorySourceIssues(source) {
    if (Array.isArray(source && source.items)) {
      return generatedContractArray(source.unresolved_issues);
    }
    return generatedContractInventoryRoots(source)
      .flatMap((root) => generatedContractArray(root && root.unresolved_issues));
  }

  function generatedContractRetryValues(source, keys) {
    return generatedContractRawValues(source, ["retry_steps", "retrySteps", "retry_plan", "retryPlan"])
      .flatMap((step) => generatedContractRawValues(step, keys));
  }

  function generatedContractInventoryRetryItems(source) {
    return generatedContractInventoryRoots(source).flatMap((root) => {
      const plan = root.repair_plan || root.repairPlan || {};
      return generatedContractRawValues(root, ["retry_items", "retryItems"])
        .concat(generatedContractRawValues(plan, ["retry_items", "retryItems"]));
    });
  }

  function generatedContractApplyRetryAliases(source, out) {
    const manualRetry = source.manual_fixture_retry || source.manualFixtureRetry || {};
    const commandEntry = source.commands_run_entry || source.commandsRunEntry || {};
    const completionEvidence = source.required_completion_evidence || source.requiredCompletionEvidence || source.completion_evidence_shape || source.completionEvidenceShape || {};
    const retryCommands = generatedContractRawValues(manualRetry, ["commands", "commands_run", "commandsRun"])
      .concat(generatedContractRawValues(source, ["retry_command", "retryCommand", "retry_commands", "retryCommands"]))
      .concat(generatedContractNestedAliasValues(source, ["retry_command_shape", "retryCommandShape"], ["command", "commands"]))
      .concat(generatedContractRawValues(commandEntry, ["command", "commands"]))
      .concat(generatedContractRawValues(completionEvidence, ["command_refs", "commandRefs"]))
      .concat(generatedContractRetryValues(source, ["command", "commands", "commands_run", "commandsRun"]));
    if (retryCommands.length > 0) {
      out.focused_verification = generatedContractArray(out.focused_verification).concat(retryCommands);
    }
    const retryExpected = generatedContractRawValues(source, ["acceptance_rule", "acceptanceRule"])
      .concat(generatedContractRawValues(completionEvidence, ["evidence_refs", "evidenceRefs", "required_summary_points", "requiredSummaryPoints"]))
      .concat(generatedContractRetryValues(source, ["required_evidence", "requiredEvidence", "evidence_to_capture", "evidenceToCapture", "expected_result", "expectedResult"]));
    if (retryExpected.length > 0) {
      out.expected_evidence = generatedContractArray(out.expected_evidence).concat(retryExpected);
    }
    const retryArtifacts = generatedContractRawValues(manualRetry, ["artifact_checks", "artifactChecks", "expected_artifacts", "expectedArtifacts", "artifact_paths", "artifactPaths"])
      .concat(generatedContractRawValues(completionEvidence, ["artifact_paths", "artifactPaths"]))
      .concat(generatedContractRetryValues(source, ["artifact_checks", "artifactChecks", "expected_artifacts", "expectedArtifacts", "artifact_paths", "artifactPaths"]));
    if (retryArtifacts.length > 0) {
      out.artifact_requirements = generatedContractArray(out.artifact_requirements).concat(retryArtifacts);
    }
    if (!out.source_item_id && (source.source_failed_item_id || source.sourceFailedItemId)) {
      out.source_item_id = source.source_failed_item_id || source.sourceFailedItemId;
    }
  }

  function generatedContractNestedAliasValues(source, roots, keys) {
    return roots.flatMap((root) => generatedContractArrayOrOne(source && source[root]))
      .flatMap((entry) => generatedContractRawValues(entry, keys));
  }

  function generatedContractArrayOrOne(value) {
    if (Array.isArray(value)) return value;
    return value === undefined || value === null ? [] : [value];
  }

  function generatedContractApplyProviderEnvAliases(source, out) {
    const keys = generatedContractProviderEnvKeys(source);
    if (keys.length > 0) {
      out.provider_env_requirements = generatedContractArray(out.provider_env_requirements).concat(keys);
      out.expected_evidence = generatedContractArray(out.expected_evidence).concat(keys.flatMap((key) => generatedContractStrings(key).map((name) => "provider_env_proof:" + name)));
    }
    if (!out.provider_env_proof && (source.provider_env_proof || source.providerEnvProof)) {
      out.provider_env_proof = source.provider_env_proof || source.providerEnvProof;
    }
  }

  function generatedContractProviderEnvKeys(source) {
    return generatedContractUnique(generatedContractRawValues(source, ["provider_env_requirements", "providerEnvRequirements", "provider_env_required_keys", "providerEnvRequiredKeys", "required_env_keys", "requiredEnvKeys", "credential_env_keys", "credentialEnvKeys"])
      .flatMap((key) => generatedContractStrings(key))
      .map((key) => String(key).trim().toUpperCase())
      .filter(Boolean));
  }
