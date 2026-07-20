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
        // D73 companion: a verification that only ran test commands matching
        // zero tests proved nothing — the same fail-closed rule the write
        // path applies via patch validation.
        demote_commandless_acceptance(outcome);
        if matches!(
            outcome.status,
            WorkflowV2Status::Accepted | WorkflowV2Status::Noop
        ) {
            demote_zero_test_acceptance(outcome);
        }
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

/// Demote an accepted/noop focused-verification outcome that recorded NO
/// successful command execution at all. commands_run is agent-reported, so a
/// verifier that ran nothing and self-reported acceptance must fail closed —
/// this is the execution-side backstop for goal-oriented verifiers whose
/// items pin no commands.
fn demote_commandless_acceptance(outcome: &mut WorkflowV2BranchOutcome) {
    let Some(result) = outcome.result.as_mut() else {
        return;
    };
    let ran_any_successful_command = result
        .commands_run
        .iter()
        .any(|command| command.status == archon_workflow::WorkflowV2CommandStatus::Succeeded);
    if ran_any_successful_command {
        return;
    }
    result.status = WorkflowV2Status::NeedsReview;
    result.residual_gaps.push(archon_workflow::WorkflowV2ResidualGap {
        id: "zero_command_verification".to_string(),
        description:
            "this focused verification recorded no successful command execution; a run that executed nothing is not verification evidence"
                .to_string(),
        severity: Some("review".to_string()),
    });
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "accepted verification demoted: no successful command execution recorded",
    ));
    let mut data = result.data.as_object().cloned().unwrap_or_default();
    data.insert(
        "zero_command_verification".to_string(),
        serde_json::json!(true),
    );
    data.insert(
        "verification_failure_class".to_string(),
        serde_json::json!("retryable_verification_shape_issue"),
    );
    result.data = serde_json::Value::Object(data);
    outcome.status = WorkflowV2Status::NeedsReview;
    outcome.failure_kind = Some(BranchFailureKind::Semantic);
}

/// Demote an accepted/noop focused-verification outcome whose only test
/// evidence is commands that matched zero tests. A run that executed nothing
/// cannot verify anything; triage routes it as a retry with corrected names.
fn demote_zero_test_acceptance(outcome: &mut WorkflowV2BranchOutcome) {
    let Some(result) = outcome.result.as_mut() else {
        return;
    };
    let test_commands: Vec<_> = result
        .commands_run
        .iter()
        .filter(|command| command.kind == archon_workflow::WorkflowV2CommandKind::Test)
        .collect();
    if test_commands.is_empty() {
        return;
    }
    // ANY zero-match test command demotes, not only "all of them". A single
    // command carrying several filters can report overall success while named
    // filters inside it matched nothing — that command proves nothing about
    // those filters, and treating the batch as passing credits untested work.
    let any_zero_matched = test_commands.iter().any(|command| {
        archon_workflow::context::output_reports_zero_matched_tests(&command.output_summary)
    });
    if !any_zero_matched {
        return;
    }
    result.status = WorkflowV2Status::NeedsReview;
    result.residual_gaps.push(
        archon_workflow::WorkflowV2ResidualGap {
            id: "zero_test_match_verification".to_string(),
            description:
                "every test command in this focused verification matched zero tests; a run that executed nothing is not verification evidence"
                    .to_string(),
            severity: Some("review".to_string()),
        },
    );
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "accepted verification demoted: filtered test commands matched zero tests",
    ));
    let mut data = result.data.as_object().cloned().unwrap_or_default();
    data.insert("zero_test_match".to_string(), serde_json::json!(true));
    data.insert(
        "verification_failure_class".to_string(),
        serde_json::json!("retryable_verification_shape_issue"),
    );
    result.data = serde_json::Value::Object(data);
    outcome.status = WorkflowV2Status::NeedsReview;
    outcome.failure_kind = Some(BranchFailureKind::Semantic);
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

#[cfg(test)]
mod commandless_demotion_tests {
    use super::*;
    use archon_workflow::{WorkflowV2CommandKind, WorkflowV2CommandRecord};

    fn accepted_outcome(commands: Vec<WorkflowV2CommandRecord>) -> WorkflowV2BranchOutcome {
        let mut result = WorkflowV2Result::accepted("verifier claims acceptance");
        result.commands_run = commands;
        WorkflowV2BranchOutcome {
            item_id: "verify-check".to_string(),
            role: "verifier".to_string(),
            status: WorkflowV2Status::Accepted,
            result: Some(result),
            error: None,
            failure_kind: None,
            item_input_hash: Some("input".to_string()),
            completion_evidence: Vec::new(),
        }
    }

    #[test]
    fn accepted_verification_with_no_successful_command_is_demoted() {
        // commands_run is agent-reported: a verifier that executed nothing
        // (or only failed commands) must never verify anything.
        let mut outcome = accepted_outcome(vec![WorkflowV2CommandRecord {
            kind: WorkflowV2CommandKind::Test,
            command: "cargo test broken".to_string(),
            status: WorkflowV2CommandStatus::Failed,
            exit_code: Some(101),
            output_summary: "compile error".to_string(),
        }]);
        normalize_focused_verification_outcome("verification-wave-1-1", &mut outcome);
        assert_eq!(outcome.status, WorkflowV2Status::NeedsReview);
        assert_eq!(outcome.failure_kind, Some(BranchFailureKind::Semantic));
        let result = outcome.result.expect("result");
        assert_eq!(result.data["zero_command_verification"], true);
        assert!(
            result
                .residual_gaps
                .iter()
                .any(|gap| gap.id == "zero_command_verification")
        );

        let mut empty = accepted_outcome(Vec::new());
        normalize_focused_verification_outcome("verification-wave-1-2", &mut empty);
        assert_eq!(empty.status, WorkflowV2Status::NeedsReview);
    }

    #[test]
    fn one_zero_match_command_among_passing_ones_still_demotes() {
        // A verifier can run several test commands, one of which matched zero
        // tests (e.g. a misnamed filter) while the others passed. ANY zero
        // match demotes — the batch passing does not excuse the filter that
        // proved nothing.
        let mut outcome = accepted_outcome(vec![
            WorkflowV2CommandRecord {
                kind: WorkflowV2CommandKind::Test,
                command: "cargo test real_pass".to_string(),
                status: WorkflowV2CommandStatus::Succeeded,
                exit_code: Some(0),
                output_summary: "test result: ok. 5 passed; 0 failed".to_string(),
            },
            WorkflowV2CommandRecord {
                kind: WorkflowV2CommandKind::Test,
                command: "cargo test misnamed_filter".to_string(),
                status: WorkflowV2CommandStatus::Succeeded,
                exit_code: Some(0),
                output_summary: "test result: ok. 0 passed; 0 failed; 12 filtered out".to_string(),
            },
        ]);
        normalize_focused_verification_outcome("verification-wave-1-4", &mut outcome);
        assert_eq!(outcome.status, WorkflowV2Status::NeedsReview);
    }

    #[test]
    fn accepted_verification_with_a_successful_command_is_not_demoted() {
        let mut outcome = accepted_outcome(vec![WorkflowV2CommandRecord {
            kind: WorkflowV2CommandKind::Test,
            command: "cargo test -p archon-trading data_lake --lib".to_string(),
            status: WorkflowV2CommandStatus::Succeeded,
            exit_code: Some(0),
            output_summary: "test result: ok. 16 passed; 0 failed".to_string(),
        }]);
        normalize_focused_verification_outcome("verification-wave-1-3", &mut outcome);
        assert_eq!(outcome.status, WorkflowV2Status::Accepted);
    }
}
