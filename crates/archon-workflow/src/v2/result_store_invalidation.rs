fn downstream_call_ids(
    executions: &[super::WorkflowV2CallExecution],
    call_id: &str,
) -> BTreeSet<String> {
    let mut by_dependency: HashMap<&str, Vec<&str>> = HashMap::new();
    for execution in executions {
        for dependency in &execution.depends_on {
            by_dependency
                .entry(dependency.as_str())
                .or_default()
                .push(execution.call.id.as_str());
        }
    }

    let mut invalidated = BTreeSet::new();
    let mut queue = vec![call_id];
    while let Some(current) = queue.pop() {
        if !invalidated.insert(current.to_string()) {
            continue;
        }
        if let Some(children) = by_dependency.get(current) {
            queue.extend(children.iter().copied());
        }
    }
    invalidated
}

fn dynamic_wave_invalidated_call_ids(
    records: &[WorkflowV2CallRecord],
    call_id: &str,
) -> BTreeSet<String> {
    let Some(source_record) = records.iter().find(|record| record.call.id == call_id) else {
        return BTreeSet::new();
    };
    let restart_position = generated_prd_call_position(call_id);
    let mut impacted_task_ids = record_completed_or_owned_task_ids(source_record);
    let mut invalidated = BTreeSet::from([call_id.to_string()]);
    let mut changed = true;
    while changed {
        changed = false;
        for record in records {
            if invalidated.contains(&record.call.id) {
                continue;
            }
            let later_generated_call = restart_position
                .zip(generated_prd_call_position(&record.call.id))
                .is_some_and(|(restart, candidate)| candidate > restart);
            if later_generated_call || source_graph_intersects_tasks(record, &impacted_task_ids) {
                invalidated.insert(record.call.id.clone());
                impacted_task_ids.extend(record_completed_or_owned_task_ids(record));
                changed = true;
            }
        }
    }
    invalidated
}

fn record_intersects_tasks(record: &WorkflowV2CallRecord, task_ids: &BTreeSet<String>) -> bool {
    record.completed_ids.iter().any(|task_id| task_ids.contains(task_id))
        || record
            .completion_evidence
            .iter()
            .any(|evidence| task_ids.contains(&evidence.task_id))
        || source_graph_intersects_tasks(record, task_ids)
}

fn branch_outcome_task_ids(outcome: &WorkflowV2BranchOutcome) -> BTreeSet<String> {
    outcome
        .completion_evidence
        .iter()
        .map(|evidence| evidence.task_id.trim())
        .filter(|task_id| !task_id.is_empty())
        .map(str::to_string)
        .collect()
}

fn source_graph_intersects_tasks(
    record: &WorkflowV2CallRecord,
    task_ids: &BTreeSet<String>,
) -> bool {
    let Some(graph) = record.source_task_graph.as_ref() else {
        return false;
    };
    graph.items.iter().any(|item| {
        item.canonical_task_ids
            .iter()
            .chain(item.dependency_ids.iter())
            .any(|task_id| task_ids.contains(task_id))
    })
}

fn record_completed_or_owned_task_ids(record: &WorkflowV2CallRecord) -> BTreeSet<String> {
    let Some(graph) = record.source_task_graph.as_ref() else {
        return BTreeSet::new();
    };
    let completed = record
        .completed_ids
        .iter()
        .filter(|task_id| !task_id.trim().is_empty())
        .cloned()
        .collect::<BTreeSet<_>>();
    if !completed.is_empty() {
        return completed;
    }
    let completed = graph
        .completed_ids
        .iter()
        .filter(|task_id| !task_id.trim().is_empty())
        .cloned()
        .collect::<BTreeSet<_>>();
    if !completed.is_empty() {
        return completed;
    }
    graph
        .items
        .iter()
        .flat_map(|item| item.canonical_task_ids.iter())
        .filter(|task_id| !task_id.trim().is_empty())
        .cloned()
        .collect()
}

fn generated_prd_call_position(call_id: &str) -> Option<u32> {
    for (prefix, rank) in [
        ("noop-proof-verification-", 10),
        ("noop-evidence-repair-", 11),
        ("noop-proof-reverification-", 12),
        ("implementation-wave-", 20),
        ("remediation-inventory-", 25),
        ("remediation-empty-inventory-repair-", 26),
        ("remediation-wave-", 30),
        ("remediation-outcome-repair-", 31),
        ("verification-plan-", 40),
        ("verification-plan-repair-", 41),
        ("verification-wave-", 50),
        ("verification-repair-plan-", 51),
        ("wave-completion-evidence-repair-", 55),
        ("artifact-inventory", 60),
        ("save-artifact-inventory", 61),
        ("adversarial-review-", 70),
        ("review-remediation-inventory-", 75),
        ("review-remediation-wave-", 80),
        ("review-verification-plan-", 85),
        ("review-verification-wave-", 90),
        ("final-evidence-reconciliation-", 100),
        ("completion-claim-repair-", 101),
        ("artifact-existence-investigation-", 102),
        ("require-final-artifacts", 110),
        ("final-zero-gap-audit", 120),
        ("final-acceptance-gate", 130),
        ("blocked-final-readiness", 140),
        ("final-acceptance-report", 150),
    ] {
        if let Some(index) = dynamic_call_index(call_id, prefix) {
            return Some(rank * 10_000 + index);
        }
        if call_id == prefix {
            return Some(rank * 10_000);
        }
    }
    if call_id.starts_with("blocked-") {
        return Some(135 * 10_000);
    }
    None
}

fn dynamic_call_index(call_id: &str, prefix: &str) -> Option<u32> {
    let rest = call_id.strip_prefix(prefix)?;
    let digits = rest
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}
