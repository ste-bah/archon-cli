use std::collections::BTreeSet;

use serde_json::Value;

use super::{
    WorkflowV2CommandKind, WorkflowV2CommandRecord, WorkflowV2CommandStatus, WorkflowV2Evidence,
    WorkflowV2EvidenceKind, WorkflowV2ResidualGap, WorkflowV2Result, WorkflowV2Status,
    WorkflowV2TaskCoverage, WorkflowV2TaskCoverageStatus,
};

pub fn manifest_scope_verification_result(input: &Value) -> Option<WorkflowV2Result> {
    if !is_diff_scope_check(input) {
        return None;
    }
    let scope = input.get("write_coordination_scope")?;
    let declared = string_set(scope.get("declared_target_files"));
    let observed = observed_files(scope);
    let escaped: Vec<String> = observed.difference(&declared).cloned().collect();
    Some(scope_result(input, scope, escaped))
}

fn is_diff_scope_check(input: &Value) -> bool {
    let text = ["item_id", "focused_verification", "expected_evidence"]
        .into_iter()
        .flat_map(|key| string_values(input.get(key)))
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    text.contains("diff")
        && (text.contains("scope") || text.contains("ownership") || text.contains("owned"))
}

fn observed_files(scope: &Value) -> BTreeSet<String> {
    ["changed_files", "created_files", "deleted_files"]
        .into_iter()
        .flat_map(|key| string_set(scope.get(key)))
        .collect()
}

fn scope_result(input: &Value, scope: &Value, escaped: Vec<String>) -> WorkflowV2Result {
    let accepted = escaped.is_empty();
    let summary = if accepted {
        "write-coordination manifest confirms all branch changes are declared"
    } else {
        "write-coordination manifest contains changed files outside declared ownership"
    };
    let mut result = WorkflowV2Result::accepted(summary);
    if !accepted {
        result.status = WorkflowV2Status::Failed;
        result.residual_gaps.push(WorkflowV2ResidualGap {
            id: "write-coordination-scope-escape".to_string(),
            description: format!("undeclared manifest paths: {}", escaped.join(", ")),
            severity: Some("high".to_string()),
        });
    }
    attach_scope_evidence(&mut result, input, scope, accepted);
    result
}

fn attach_scope_evidence(result: &mut WorkflowV2Result, input: &Value, scope: &Value, ok: bool) {
    let summary = result.summary.clone();
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        summary.clone(),
    ));
    result.commands_run.push(WorkflowV2CommandRecord {
        kind: WorkflowV2CommandKind::Inspect,
        command: "host:validate_write_coordination_scope".to_string(),
        status: if ok {
            WorkflowV2CommandStatus::Succeeded
        } else {
            WorkflowV2CommandStatus::Failed
        },
        exit_code: Some(if ok { 0 } else { 1 }),
        output_summary: summary.clone(),
    });
    for task_id in string_values(input.get("canonical_task_ids")) {
        result.task_coverage.push(WorkflowV2TaskCoverage {
            task_id,
            status: if ok {
                WorkflowV2TaskCoverageStatus::Accepted
            } else {
                WorkflowV2TaskCoverageStatus::Partial
            },
            summary: summary.clone(),
            evidence: result.evidence.clone(),
        });
    }
    result.data = serde_json::json!({
        "source_item_id": input.get("source_item_id"),
        "write_coordination_scope": scope,
        "host_validated_diff_scope": true,
    });
}

fn string_set(value: Option<&Value>) -> BTreeSet<String> {
    string_values(value).into_iter().collect()
}

fn string_values(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(text)) => vec![text.clone()],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
#[path = "manifest_scope_tests.rs"]
mod tests;
