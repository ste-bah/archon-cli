use serde_json::Value;

use crate::generated_lifecycle_support::{
    LifecycleContract, array, present, raw_strings, strings_of,
};

fn issue(kind: &str, item: &Value, field: &str, message: &str) -> Value {
    serde_json::json!({
        "kind": kind,
        "field": field,
        "message": message,
        "item_id": item.get("item_id").or_else(|| item.get("id")),
        "canonical_task_ids": array(item.get("canonical_task_ids")),
    })
}

fn target_file_issue_message(
    contract: &LifecycleContract<'_>,
    target: &str,
) -> Option<&'static str> {
    crate::generated_contract::lifecycle_target_file_issue(target, contract.target_repository_root)
}

pub(super) fn remediation_item_issues(
    contract: &LifecycleContract<'_>,
    item: &Value,
) -> Vec<Value> {
    let mut issues = Vec::new();
    push_remediation_shape_issues(contract, item, &mut issues);
    push_remediation_target_issues(contract, item, &mut issues);
    if item.get("artifact_requirements").is_none() {
        issues.push(issue(
            "artifact_requirements_discovery",
            item,
            "artifact_requirements",
            "remediation item is missing artifact requirements",
        ));
    }
    issues
}

fn push_remediation_shape_issues(
    contract: &LifecycleContract<'_>,
    item: &Value,
    issues: &mut Vec<Value>,
) {
    push_identity_issue("inventory_shape_repair", "remediation", item, issues);
    push_task_issue(
        contract,
        "task_universe_reconcile",
        "remediation",
        item,
        issues,
    );
    push_invalid_dependencies(contract, "task_universe_reconcile", item, issues);
    push_missing_fields(
        "inventory_shape_repair",
        item,
        &[
            "source_item_id",
            "failure_status",
            "failure_evidence",
            "required_fix",
        ],
        issues,
    );
    if !present(item.get("focused_verification")) && !present(item.get("verification_requirements"))
    {
        issues.push(issue(
            "verification_requirements_discovery",
            item,
            "focused_verification",
            "remediation item is missing focused verification",
        ));
    }
}

fn push_remediation_target_issues(
    contract: &LifecycleContract<'_>,
    item: &Value,
    issues: &mut Vec<Value>,
) {
    if item.get("target_files").is_none() {
        issues.push(issue(
            "target_file_discovery",
            item,
            "target_files",
            "remediation item must include target_files",
        ));
    }
    push_target_file_issue_values(contract, "target_file_discovery", item, issues);
}

pub(super) fn review_remediation_item_issues(
    contract: &LifecycleContract<'_>,
    item: &Value,
) -> Vec<Value> {
    let mut issues = Vec::new();
    push_review_shape_issues(contract, item, &mut issues);
    push_review_target_issues(contract, item, &mut issues);
    issues
}

fn push_review_shape_issues(
    contract: &LifecycleContract<'_>,
    item: &Value,
    issues: &mut Vec<Value>,
) {
    push_identity_issue(
        "review_remediation_shape_repair",
        "review remediation",
        item,
        issues,
    );
    push_task_issue(
        contract,
        "review_remediation_task_reconcile",
        "review remediation",
        item,
        issues,
    );
    for dep in strings_of(item.get("dependency_ids")) {
        if !contract.canonical_universe().contains(&dep) {
            issues.push(issue(
                "review_remediation_task_reconcile",
                item,
                "dependency_ids",
                "review remediation dependency is not a canonical task ID",
            ));
        }
    }
    push_missing_fields(
        "review_remediation_shape_repair",
        item,
        &[
            "source_item_id",
            "failure_status",
            "failure_evidence",
            "required_fix",
            "focused_verification",
        ],
        issues,
    );
}

fn push_review_target_issues(
    contract: &LifecycleContract<'_>,
    item: &Value,
    issues: &mut Vec<Value>,
) {
    if item.get("target_files").is_none() {
        issues.push(issue(
            "review_remediation_target_discovery",
            item,
            "target_files",
            "review remediation item must include target_files, using [] only for artifact-only remediation",
        ));
    }
    push_target_file_issue_values(
        contract,
        "review_remediation_target_discovery",
        item,
        issues,
    );
    if item.get("artifact_requirements").is_none() {
        issues.push(issue(
            "review_remediation_artifact_discovery",
            item,
            "artifact_requirements",
            "review remediation item is missing artifact requirements",
        ));
    }
    if raw_strings(item, &["target_files"]).is_empty()
        && !present(item.get("artifact_requirements"))
    {
        issues.push(issue(
            "review_remediation_artifact_discovery",
            item,
            "artifact_requirements",
            "artifact-only remediation needs concrete artifact requirements",
        ));
    }
}

fn push_identity_issue(kind: &str, label: &str, item: &Value, issues: &mut Vec<Value>) {
    if !(item.get("item_id").is_some() || item.get("id").is_some()) {
        issues.push(issue(
            kind,
            item,
            "item_id",
            &format!("{label} item is missing item_id/id"),
        ));
    }
}

fn push_task_issue(
    contract: &LifecycleContract<'_>,
    kind: &str,
    label: &str,
    item: &Value,
    issues: &mut Vec<Value>,
) {
    if contract.canonical_ids_for(item).is_empty() {
        issues.push(issue(
            kind,
            item,
            "canonical_task_ids",
            &format!("{label} item must use canonical taskUniverse IDs"),
        ));
    }
}

fn push_invalid_dependencies(
    contract: &LifecycleContract<'_>,
    kind: &str,
    item: &Value,
    issues: &mut Vec<Value>,
) {
    for dep in contract.invalid_dependency_ids_for(item) {
        issues.push(issue(
            kind,
            item,
            "dependency_ids",
            &format!("remediation dependency is not canonical: {dep}"),
        ));
    }
}

fn push_missing_fields(kind: &str, item: &Value, fields: &[&str], issues: &mut Vec<Value>) {
    for required in fields {
        if !present(item.get(*required)) {
            issues.push(issue(
                kind,
                item,
                required,
                &format!("remediation item is missing {required}"),
            ));
        }
    }
}

fn push_target_file_issue_values(
    contract: &LifecycleContract<'_>,
    kind: &str,
    item: &Value,
    issues: &mut Vec<Value>,
) {
    for target in raw_strings(item, &["target_files"]) {
        if let Some(problem) = target_file_issue_message(contract, &target) {
            issues.push(issue(
                kind,
                item,
                "target_files",
                &format!("{problem}: {target}"),
            ));
        }
    }
}
