use super::{
    WorkflowV2CommandKind, WorkflowV2CommandRecord, WorkflowV2CommandStatus, WorkflowV2Evidence,
    WorkflowV2EvidenceKind, WorkflowV2ResidualGap, WorkflowV2Result, WorkflowV2Status,
    WorkflowV2TaskCoverage, WorkflowV2TaskCoverageStatus,
};

pub fn semantic_branch_result_from_value(value: &serde_json::Value) -> Option<WorkflowV2Result> {
    let object = value.as_object()?;
    if !looks_like_branch_evidence(object) {
        return None;
    }
    let mut result = WorkflowV2Result {
        status: status_from_value(value).unwrap_or(WorkflowV2Status::NeedsReview),
        summary: summary_from_value(value),
        ..WorkflowV2Result::default()
    };
    result.commands_run = command_records_from_value(value);
    result.task_coverage = task_coverage_from_value(value);
    result.evidence = evidence_from_value(value);
    copy_semantic_data(&mut result, value);
    add_default_task_coverage(&mut result);
    downgrade_failed_commands(&mut result);
    Some(result)
}

fn looks_like_branch_evidence(object: &serde_json::Map<String, serde_json::Value>) -> bool {
    object.contains_key("canonical_task_ids")
        || object.contains_key("canonical_task_id")
        || object.contains_key("task_ids")
        || object.contains_key("task_id")
        || object.contains_key("task_coverage")
}

fn status_from_value(value: &serde_json::Value) -> Option<WorkflowV2Status> {
    for key in ["status", "final_status", "overall_status", "overall_result"] {
        let Some(raw) = value.get(key).and_then(serde_json::Value::as_str) else {
            continue;
        };
        let normalized = raw.trim().to_ascii_lowercase();
        return Some(match normalized.as_str() {
            "pass" | "passed" | "success" | "succeeded" | "accepted" | "complete" => {
                WorkflowV2Status::Accepted
            }
            "noop" | "no_op" | "verified_noop" => WorkflowV2Status::Noop,
            "blocked" => WorkflowV2Status::Blocked,
            "failed" | "fail" | "failure" => WorkflowV2Status::Failed,
            "needs_review" | "needs-review" | "review" | "partial" => WorkflowV2Status::NeedsReview,
            "cancelled" | "canceled" => WorkflowV2Status::Cancelled,
            _ => WorkflowV2Status::NeedsReview,
        });
    }
    None
}

fn summary_from_value(value: &serde_json::Value) -> String {
    ["summary", "output_summary", "reason", "message"]
        .iter()
        .filter_map(|key| value.get(*key).and_then(serde_json::Value::as_str))
        .map(str::trim)
        .find(|text| !text.is_empty())
        .unwrap_or("schema-less project artifact branch evidence")
        .to_string()
}

fn command_records_from_value(value: &serde_json::Value) -> Vec<WorkflowV2CommandRecord> {
    ["commands_run", "commands", "focused_verification_executed"]
        .iter()
        .filter_map(|key| value.get(*key))
        .flat_map(command_records_from_field)
        .collect()
}

fn command_records_from_field(value: &serde_json::Value) -> Vec<WorkflowV2CommandRecord> {
    match value {
        serde_json::Value::Array(items) => items.iter().filter_map(command_record).collect(),
        item => command_record(item).into_iter().collect(),
    }
}

fn command_record(value: &serde_json::Value) -> Option<WorkflowV2CommandRecord> {
    let object = value.as_object()?;
    let command = object
        .get("command")
        .or_else(|| object.get("cmd"))
        .and_then(serde_json::Value::as_str)?
        .trim()
        .to_string();
    (!command.is_empty()).then(|| WorkflowV2CommandRecord {
        kind: command_kind(object.get("kind").and_then(serde_json::Value::as_str)),
        command,
        status: command_status(object),
        exit_code: object
            .get("exit_code")
            .or_else(|| object.get("exitStatus"))
            .and_then(serde_json::Value::as_i64)
            .map(|code| code as i32),
        output_summary: object
            .get("output_summary")
            .or_else(|| object.get("summary"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}

fn command_kind(raw: Option<&str>) -> WorkflowV2CommandKind {
    match raw.unwrap_or_default().to_ascii_lowercase().as_str() {
        "test" => WorkflowV2CommandKind::Test,
        "build" => WorkflowV2CommandKind::Build,
        "format" | "fmt" => WorkflowV2CommandKind::Format,
        "review" => WorkflowV2CommandKind::Review,
        "inspect" | "inspection" => WorkflowV2CommandKind::Inspect,
        _ => WorkflowV2CommandKind::Other,
    }
}

fn command_status(object: &serde_json::Map<String, serde_json::Value>) -> WorkflowV2CommandStatus {
    if let Some(code) = object
        .get("exit_code")
        .or_else(|| object.get("exitStatus"))
        .and_then(serde_json::Value::as_i64)
    {
        return if code == 0 {
            WorkflowV2CommandStatus::Succeeded
        } else {
            WorkflowV2CommandStatus::Failed
        };
    }
    match object
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "succeeded" | "success" | "passed" | "pass" => WorkflowV2CommandStatus::Succeeded,
        "failed" | "failure" | "fail" => WorkflowV2CommandStatus::Failed,
        "skipped" | "skip" => WorkflowV2CommandStatus::Skipped,
        _ => WorkflowV2CommandStatus::Skipped,
    }
}

fn task_coverage_from_value(value: &serde_json::Value) -> Vec<WorkflowV2TaskCoverage> {
    serde_json::from_value(value.get("task_coverage").cloned().unwrap_or_default())
        .unwrap_or_default()
}

fn evidence_from_value(value: &serde_json::Value) -> Vec<WorkflowV2Evidence> {
    serde_json::from_value(value.get("evidence").cloned().unwrap_or_default()).unwrap_or_default()
}

fn copy_semantic_data(result: &mut WorkflowV2Result, value: &serde_json::Value) {
    let mut data = serde_json::Map::new();
    for key in [
        "item_id",
        "source_item_id",
        "branch_id",
        "canonical_task_ids",
    ] {
        if let Some(raw) = value.get(key) {
            data.insert(key.to_string(), raw.clone());
        }
    }
    result.data = serde_json::Value::Object(data);
}

fn add_default_task_coverage(result: &mut WorkflowV2Result) {
    if !result.task_coverage.is_empty() {
        return;
    }
    let ids = result
        .data
        .get("canonical_task_ids")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    for id in ids.iter().filter_map(serde_json::Value::as_str) {
        result.task_coverage.push(WorkflowV2TaskCoverage {
            task_id: id.to_string(),
            status: coverage_status(result.status),
            summary: result.summary.clone(),
            evidence: vec![WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Implementation,
                result.summary.clone(),
            )],
        });
    }
}

fn coverage_status(status: WorkflowV2Status) -> WorkflowV2TaskCoverageStatus {
    match status {
        WorkflowV2Status::Accepted => WorkflowV2TaskCoverageStatus::Accepted,
        WorkflowV2Status::Noop => WorkflowV2TaskCoverageStatus::Noop,
        WorkflowV2Status::Blocked => WorkflowV2TaskCoverageStatus::Blocked,
        WorkflowV2Status::NeedsReview | WorkflowV2Status::Failed => {
            WorkflowV2TaskCoverageStatus::Partial
        }
        _ => WorkflowV2TaskCoverageStatus::Unknown,
    }
}

fn downgrade_failed_commands(result: &mut WorkflowV2Result) {
    let failed = result
        .commands_run
        .iter()
        .find(|command| command.status == WorkflowV2CommandStatus::Failed);
    let Some(command) = failed else {
        return;
    };
    if matches!(
        result.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) {
        result.status = WorkflowV2Status::NeedsReview;
    }
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!("failed_command_{}", safe_id(&command.command)),
        description: format!(
            "failed command evidence requires review: {}",
            command.command
        ),
        severity: Some("review".to_string()),
    });
}

fn safe_id(raw: &str) -> String {
    raw.chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}
