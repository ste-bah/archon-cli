  function generatedContractInventoryHasIssues(inventory) {
    return generatedContractArray(inventory && inventory.unresolved_issues).length > 0;
  }

  function generatedContractVerificationItems(inventory) {
    return splitFocusedVerificationItems(generatedContractArray(inventory && inventory.items).map((item) => normalizeGeneratedItem(item)));
  }

  function generatedContractConstrainInventoryTasks(inventory, allowedTaskIds) {
    const allowed = new Set(generatedContractArray(allowedTaskIds).map(String));
    if (allowed.size === 0) return inventory;
    const items = [];
    const issues = generatedContractArray(inventory && inventory.unresolved_issues).slice();
    for (const rawItem of generatedContractArray(inventory && inventory.items)) {
      const item = normalizeGeneratedItem(rawItem);
      const taskIds = generatedContractArray(item.canonical_task_ids).map(String);
      const outside = taskIds.filter((id) => !allowed.has(id));
      if (outside.length === 0) {
        items.push(item);
      } else {
        issues.push({ kind: "verification_requirements_discovery", field: "canonical_task_ids", message: "verification repair introduced out-of-scope canonical task IDs", item_id: item.item_id || item.id, canonical_task_ids: taskIds });
      }
    }
    return { ...inventory, items, unresolved_issues: issues };
  }

  function generatedContractVerificationInventoryReady(inventory) {
    return generatedContractArray(inventory && inventory.items).length > 0
      && !generatedContractInventoryHasIssues(inventory);
  }
