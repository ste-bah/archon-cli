use super::*;

pub(super) fn preflight_write_fanout_source_contract(
    call: &WorkflowV2HostCall,
    branches: &[archon_workflow::WorkflowV2FanoutItem],
    write_items: &[WorkflowV2WriteItem],
    plan: &WorkflowV2WritePlan,
    _repository_root: Option<&str>,
) -> Option<WorkflowV2Result> {
    if !is_implementation_write_fanout(call) {
        return None;
    }
    let mut issues = Vec::new();
    if !plan
        .conflicts
        .iter()
        .all(|conflict| conflict.isolated_by_worktree)
    {
        issues.extend(duplicate_broad_ownership_issues(write_items));
        issues.extend(shared_hot_target_issues(write_items));
    }
    (!issues.is_empty()).then(|| write_source_preflight_result(call, branches, plan, issues))
}

pub(super) fn is_implementation_write_fanout(call: &WorkflowV2HostCall) -> bool {
    call.method == archon_workflow::WorkflowV2HostMethod::Fanout
        && call
            .options
            .item_kind
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("implementation"))
}

pub(super) fn duplicate_broad_ownership_issues(
    write_items: &[WorkflowV2WriteItem],
) -> Vec<serde_json::Value> {
    let mut by_targets: BTreeMap<String, (Vec<String>, Vec<String>)> = BTreeMap::new();
    for item in write_items {
        let mut targets = item.owned_targets.clone();
        targets.sort();
        let key = targets.join("\u{0}");
        by_targets
            .entry(key)
            .or_insert_with(|| (targets, Vec::new()))
            .1
            .push(item.id.clone());
    }
    by_targets
        .into_values()
        .filter(|(targets, owners)| targets.len() >= 3 && owners.len() > 1)
        .map(|(targets, owners)| {
            issue(
                "duplicate_broad_ownership",
                owners,
                targets.iter().take(8).cloned().collect(),
            )
        })
        .collect()
}

pub(super) fn shared_hot_target_issues(
    write_items: &[WorkflowV2WriteItem],
) -> Vec<serde_json::Value> {
    let mut by_target: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for item in write_items {
        for target in &item.owned_targets {
            by_target
                .entry(target.clone())
                .or_default()
                .push(item.id.clone());
        }
    }
    by_target
        .into_iter()
        .filter(|(_, owners)| owners.len() >= 3)
        .take(25)
        .map(|(target, owners)| issue("shared_hot_target", owners, vec![target]))
        .collect()
}

pub(super) fn issue(kind: &str, item_ids: Vec<String>, targets: Vec<String>) -> serde_json::Value {
    serde_json::json!({
        "kind": kind,
        "severity": "review",
        "item_ids": item_ids,
        "targets": targets
    })
}

pub(super) fn write_source_preflight_result(
    call: &WorkflowV2HostCall,
    branches: &[archon_workflow::WorkflowV2FanoutItem],
    plan: &WorkflowV2WritePlan,
    issues: Vec<serde_json::Value>,
) -> WorkflowV2Result {
    let branch_results = branches
        .iter()
        .map(|branch| preflight_branch_result(branch, &issues))
        .collect::<Vec<_>>();
    let mut result = result_from_write_fanout(call, branch_results, plan, 0, None);
    result.summary = format!(
        "write-capable fanout '{}' source preflight found {} contract issue(s); workflow.js must repair the inventory before branch launch",
        call.id,
        issues.len()
    );
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "write source preflight blocked branch launch; source ownership/size issues are data for workflow.js repair",
    ));
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!(
            "write_source_preflight_{}",
            sanitize_v2_path_segment(&call.id)
        ),
        description: result.summary.clone(),
        severity: Some("review".to_string()),
    });
    if let Some(object) = result.data.as_object_mut() {
        object.insert(
            "source_preflight_issues".to_string(),
            serde_json::Value::Array(issues),
        );
    }
    result
}

pub(super) fn preflight_branch_result(
    branch: &archon_workflow::WorkflowV2FanoutItem,
    issues: &[serde_json::Value],
) -> WorkflowV2Result {
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: format!(
            "write branch '{}' was not launched because source preflight found contract issues",
            branch.id
        ),
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "branch launch blocked before agent execution by source preflight",
    ));
    result.data = serde_json::json!({
        "item_id": branch.id,
        "canonical_task_ids": branch_task_ids(branch),
        "failure_kind": BranchFailureKind::Contract,
        "contract_valid": false,
        "source_preflight_issues": issues_for_branch(&branch.id, issues),
    });
    result
}

pub(super) fn branch_task_ids(branch: &archon_workflow::WorkflowV2FanoutItem) -> Vec<String> {
    branch
        .input
        .get("item")
        .map(|item| canonical_task_ids_from_generated_value(item, None))
        .unwrap_or_default()
}

pub(super) fn issues_for_branch(
    item_id: &str,
    issues: &[serde_json::Value],
) -> Vec<serde_json::Value> {
    issues
        .iter()
        .filter(|issue| issue_mentions_item(issue, item_id))
        .cloned()
        .collect()
}

pub(super) fn issue_mentions_item(issue: &serde_json::Value, item_id: &str) -> bool {
    issue
        .get("item_ids")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|items| items.iter().any(|item| item.as_str() == Some(item_id)))
}
