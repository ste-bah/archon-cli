  function normalizeRemediationInventory(value) {
    const source = value && typeof value === "object" ? value : {};
    const items = generatedContractInventorySourceItems(source).map((item) => normalizeGeneratedItem(item));
    const sourceIssues = generatedContractInventorySourceIssues(source);
    return { ...source, items, unresolved_issues: sourceIssues.concat(items.flatMap((item) => remediationItemIssues(item))) };
  }

  function normalizeRemediationInventoryForSources(value, sourceItems, fallbackItems, sourceCallId) {
    const normalized = normalizeRemediationInventory(value);
    const items = generatedContractArray(normalized.items).map((item) => {
      const source = remediationSourceForItem(item, sourceItems, fallbackItems, sourceCallId);
      return source ? normalizeGeneratedItem(remediationItemWithSourceOwnership(item, source)) : item;
    });
    const sourceIssues = generatedContractInventorySourceIssues(normalized);
    return { ...normalized, items, unresolved_issues: sourceIssues.concat(items.flatMap((item) => remediationItemIssues(item))) };
  }

  function remediationSourceForItem(item, sourceItems, fallbackItems, sourceCallId) {
    const sources = generatedContractArray(sourceItems).map((source) => normalizeGeneratedItem(source));
    const fallbacks = generatedContractArray(fallbackItems).map((source) => normalizeGeneratedItem(source));
    return remediationSourceById(item, sources, sourceCallId)
      || remediationSourceByTask(item, sources)
      || remediationSourceById(item, fallbacks, sourceCallId)
      || remediationSourceByTask(item, fallbacks);
  }

  function remediationSourceById(item, sources, sourceCallId) {
    const rawIds = generatedContractRawStrings(item || {}, ["source_item_id", "sourceItemId", "failed_item_id", "failedItemId", "item_id", "id"]);
    const ids = new Set(rawIds.flatMap((id) => [id, stripSourceCallPrefix(id, sourceCallId)]).filter(Boolean));
    return generatedContractArray(sources).find((source) => ids.has(source.item_id) || ids.has(source.id));
  }

  function stripSourceCallPrefix(value, sourceCallId) {
    const prefix = sourceCallId ? String(sourceCallId).trim() + "-" : "";
    const text = String(value || "").trim();
    return prefix && text.startsWith(prefix) ? text.slice(prefix.length) : text;
  }

  function remediationSourceByTask(item, sources) {
    const ids = canonicalIdsFor(item);
    if (ids.length === 0) return null;
    const matches = generatedContractArray(sources).filter((source) => canonicalIdsFor(source).some((id) => ids.includes(id)));
    return matches.length === 1 ? matches[0] : null;
  }

  function remediationItemWithSourceOwnership(item, source) {
    const merged = { ...(item || {}) };
    const targets = generatedContractRawStrings(source || {}, ["target_files"]);
    if (targets.length > 0) merged.target_files = targets;
    if (canonicalIdsFor(merged).length === 0 && canonicalIdsFor(source).length > 0) merged.canonical_task_ids = canonicalIdsFor(source);
    for (const field of ["dependency_ids", "artifact_requirements", "focused_verification", "acceptance_criteria"]) {
      if (!generatedContractPresent(merged[field]) && generatedContractPresent(source && source[field])) merged[field] = source[field];
    }
    if (!generatedContractPresent(merged.source_item_id)) merged.source_item_id = source && (source.item_id || source.id);
    return merged;
  }

  function remediationInventoryReady(inventory) {
    return generatedContractArray(inventory && inventory.items).length > 0
      && !generatedContractInventoryHasIssues(inventory);
  }

  function remediationTaskIdSet(items) {
    const ids = new Set();
    for (const item of generatedContractArray(items)) {
      for (const id of canonicalIdsFor(item)) ids.add(id);
    }
    return ids;
  }

  function filterRemediationInventoryByTaskIds(inventory, allowedTaskIds) {
    const allowed = allowedTaskIds instanceof Set ? allowedTaskIds : new Set(generatedContractArray(allowedTaskIds));
    if (allowed.size === 0) return inventory;
    const items = generatedContractArray(inventory && inventory.items)
      .filter((item) => canonicalIdsFor(item).some((id) => allowed.has(id)));
    return { ...(inventory || {}), items };
  }

  function remediationItemIssues(item) {
    const issues = [];
    if (!item || !(item.item_id || item.id)) issues.push(generatedContractIssue("inventory_shape_repair", item || {}, "item_id", "remediation item is missing item_id/id"));
    if (!item || canonicalIdsFor(item).length === 0) issues.push(generatedContractIssue("task_universe_reconcile", item || {}, "canonical_task_ids", "remediation item must use canonical taskUniverse IDs"));
    for (const dep of invalidDependencyIdsFor(item || {})) issues.push(generatedContractIssue("task_universe_reconcile", item, "dependency_ids", "remediation dependency is not canonical: " + dep));
    for (const field of ["source_item_id", "failure_status", "failure_evidence", "required_fix"]) {
      if (!generatedContractPresent(item && item[field])) issues.push(generatedContractIssue("inventory_shape_repair", item || {}, field, "remediation item is missing " + field));
    }
    if (!generatedContractPresent(item && (item.focused_verification || item.verification_requirements))) {
      issues.push(generatedContractIssue("verification_requirements_discovery", item || {}, "focused_verification", "remediation item is missing focused verification"));
    }
    if (!item || item.target_files === undefined) issues.push(generatedContractIssue("target_file_discovery", item || {}, "target_files", "remediation item must include target_files"));
    for (const target of generatedContractRawStrings(item || {}, ["target_files"])) {
      const issue = generatedContractTargetFileIssue(target);
      if (issue) issues.push(generatedContractIssue("target_file_discovery", item, "target_files", issue + ": " + target));
    }
    if (!item || item.artifact_requirements === undefined) issues.push(generatedContractIssue("artifact_requirements_discovery", item || {}, "artifact_requirements", "remediation item is missing artifact requirements"));
    return issues;
  }
