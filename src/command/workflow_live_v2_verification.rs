use archon_workflow::{
    BranchFailureKind, WorkflowV2BranchOutcome, WorkflowV2CommandStatus, WorkflowV2Evidence,
    WorkflowV2EvidenceKind, WorkflowV2Result, WorkflowV2Status, WorkflowV2TaskCoverageStatus,
};

use super::super::workflow_live_verification_contract::annotate_verification_failure_outcome;

pub(super) const FOCUSED_VERIFICATION_EVIDENCE_CONTRACT_VERSION: &str =
    "focused-verification-evidence-v2";

pub(super) fn is_focused_verification_call(call_id: &str) -> bool {
    call_id.starts_with("verification-wave-") || call_id.starts_with("review-verification-wave-")
}

pub(super) fn stamp_focused_verification_input(call_id: &str, input: &mut serde_json::Value) {
    if !is_focused_verification_call(call_id) {
        return;
    }
    let Some(object) = input.as_object_mut() else {
        return;
    };
    object.insert(
        "verification_evidence_contract_version".to_string(),
        serde_json::json!(FOCUSED_VERIFICATION_EVIDENCE_CONTRACT_VERSION),
    );
}

pub(super) fn normalize_focused_verification_outcome(
    call_id: &str,
    outcome: &mut WorkflowV2BranchOutcome,
) {
    if !is_focused_verification_call(call_id) || is_terminal_runtime_or_safety(outcome) {
        return;
    }
    let should_accept_duplicate_pass = {
        let Some(result) = outcome.result.as_mut() else {
            return;
        };
        stamp_focused_verification_result(result);
        has_duplicate_harness_false_gap(result) && focused_verification_command_passed(result)
    };
    if matches!(
        outcome.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) {
        return;
    }
    if !should_accept_duplicate_pass {
        annotate_verification_failure_outcome(call_id, outcome);
        return;
    }

    let Some(result) = outcome.result.as_mut() else {
        return;
    };
    result.status = WorkflowV2Status::Accepted;
    result.summary = format!(
        "focused verification accepted by canonical evidence contract: {}",
        result.summary.trim()
    );
    result
        .residual_gaps
        .retain(|gap| !is_duplicate_harness_gap(&format!("{} {}", gap.id, gap.description)));
    for coverage in &mut result.task_coverage {
        if matches!(
            coverage.status,
            WorkflowV2TaskCoverageStatus::Partial | WorkflowV2TaskCoverageStatus::Unknown
        ) {
            coverage.status = WorkflowV2TaskCoverageStatus::Accepted;
            coverage.summary = format!(
                "accepted by canonical focused verification evidence contract: {}",
                coverage.summary.trim()
            );
        }
    }
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Test,
        "canonical focused verification accepted duplicate cargo harness pass sections as one successful targeted check",
    ));
    if let Some(object) = result.data.as_object_mut() {
        object.insert(
            "verification_evidence_normalized".to_string(),
            serde_json::json!(true),
        );
        object.insert(
            "verification_evidence_normalization_reason".to_string(),
            serde_json::json!("duplicate cargo harness pass sections collapsed into one canonical focused verification pass"),
        );
    }
    outcome.status = WorkflowV2Status::Accepted;
    outcome.failure_kind = None;
    outcome.error = None;
}

fn stamp_focused_verification_result(result: &mut WorkflowV2Result) {
    let mut object = result.data.as_object().cloned().unwrap_or_default();
    object.insert(
        "verification_evidence_contract_version".to_string(),
        serde_json::json!(FOCUSED_VERIFICATION_EVIDENCE_CONTRACT_VERSION),
    );
    result.data = serde_json::Value::Object(object);
}

fn is_terminal_runtime_or_safety(outcome: &WorkflowV2BranchOutcome) -> bool {
    matches!(
        outcome.failure_kind,
        Some(BranchFailureKind::Execution | BranchFailureKind::Safety)
    ) || matches!(outcome.status, WorkflowV2Status::Cancelled)
}

fn has_duplicate_harness_false_gap(result: &WorkflowV2Result) -> bool {
    verification_text(result).iter().any(|text| {
        let lower = text.to_ascii_lowercase();
        is_duplicate_harness_gap(&lower) || lower.contains("exactly one targeted test")
    })
}

fn is_duplicate_harness_gap(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    (lower.contains("duplicate") && lower.contains("targeted") && lower.contains("test"))
        || (lower.contains("reported") && lower.contains("twice") && lower.contains("passed"))
        || lower.contains("same fully qualified test passing twice")
        || lower.contains("same exact test passing in two")
}

fn focused_verification_command_passed(result: &WorkflowV2Result) -> bool {
    result.commands_run.iter().any(|command| {
        command.status == WorkflowV2CommandStatus::Succeeded
            && command.exit_code == Some(0)
            && command_output_has_target_pass(&command.output_summary)
            && !command_output_has_failure_marker(&command.output_summary)
    })
}

fn command_output_has_target_pass(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("... ok") || lower.contains(" ok twice") || contains_positive_pass_count(&lower)
}

fn contains_positive_pass_count(text: &str) -> bool {
    for (idx, _) in text.match_indices(" passed") {
        let prefix = &text[..idx];
        let digits = prefix
            .chars()
            .rev()
            .skip_while(|ch| ch.is_ascii_whitespace())
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>();
        if digits.is_empty() {
            continue;
        }
        let number = digits.chars().rev().collect::<String>();
        if number.parse::<u64>().is_ok_and(|value| value > 0) {
            return true;
        }
    }
    false
}

fn command_output_has_failure_marker(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("test result: failed")
        || lower.contains("error: test failed")
        || lower.contains("failures:")
        || lower.contains("panicked at")
}

fn verification_text(result: &WorkflowV2Result) -> Vec<String> {
    let mut text = vec![result.summary.clone()];
    text.extend(result.evidence.iter().map(|item| item.summary.clone()));
    text.extend(
        result
            .commands_run
            .iter()
            .flat_map(|command| [command.command.clone(), command.output_summary.clone()]),
    );
    text.extend(result.task_coverage.iter().flat_map(|coverage| {
        std::iter::once(coverage.summary.clone()).chain(
            coverage
                .evidence
                .iter()
                .map(|evidence| evidence.summary.clone()),
        )
    }));
    text.extend(result.residual_gaps.iter().flat_map(|gap| {
        [
            gap.id.clone(),
            gap.description.clone(),
            gap.severity.clone().unwrap_or_default(),
        ]
    }));
    if let Some(value) = result.data.get("evidence") {
        collect_json_strings(value, &mut text);
    }
    text
}

fn collect_json_strings(value: &serde_json::Value, output: &mut Vec<String>) {
    match value {
        serde_json::Value::String(text) => output.push(text.clone()),
        serde_json::Value::Array(items) => {
            for item in items {
                collect_json_strings(item, output);
            }
        }
        serde_json::Value::Object(object) => {
            for value in object.values() {
                collect_json_strings(value, output);
            }
        }
        _ => {}
    }
}
