use super::failure_class::annotate_verification_failure_outcome;
use super::signals::{
    focused_verification_command_passed, is_duplicate_harness_gap, verification_text,
};
use crate::v2::{
    BranchFailureKind, WorkflowV2BranchOutcome, WorkflowV2Evidence, WorkflowV2EvidenceKind,
    WorkflowV2Result, WorkflowV2Status, WorkflowV2TaskCoverageStatus,
};

// Defined beside the evidence it versions, in archon-workflow, so the stamp
// written into a call's input and the fingerprint read back off its minted
// completion evidence cannot drift apart.
use crate::v2::completion_evidence::FOCUSED_VERIFICATION_EVIDENCE_CONTRACT_VERSION;

fn is_focused_verification_call(call_id: &str) -> bool {
    call_id.starts_with("verification-wave-") || call_id.starts_with("review-verification-wave-")
}

pub fn stamp_focused_verification_input(call_id: &str, input: &mut serde_json::Value) {
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

pub fn normalize_focused_verification_outcome(
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
        // Ordered last of the three: a failing test is the most specific signal
        // and carries the most actionable gap text, so it should win the verdict
        // if more than one rule applies.
        if matches!(
            outcome.status,
            WorkflowV2Status::Accepted | WorkflowV2Status::Noop
        ) {
            demote_failed_test_acceptance(outcome);
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
        .any(|command| command.status == crate::WorkflowV2CommandStatus::Succeeded);
    if ran_any_successful_command {
        return;
    }
    result.status = WorkflowV2Status::NeedsReview;
    result.residual_gaps.push(crate::WorkflowV2ResidualGap {
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

/// Demote an accepted/noop outcome that recorded a FAILING test command.
///
/// The two guards either side of this one look for shapes of *absence* — no
/// command ran, or the commands that ran matched nothing. Neither asks the
/// simpler question: did a test actually fail? A live run was accepted with
/// `tests 2/4`, both failures being `archon trading data` commands exiting 1 on
/// a real registry defect, because a failing command is neither "absent" nor
/// "zero-matched" and so matched no existing rule.
///
/// Anomaly detection fails open on every state nobody anticipated; this asserts
/// the positive instead — no `Test` command may be left failing under an
/// accepted verdict.
fn demote_failed_test_acceptance(outcome: &mut WorkflowV2BranchOutcome) {
    let Some(result) = outcome.result.as_mut() else {
        return;
    };
    let failed: Vec<String> = result
        .commands_run
        .iter()
        .filter(|command| command.kind == crate::WorkflowV2CommandKind::Test)
        .filter(|command| command.status == crate::WorkflowV2CommandStatus::Failed)
        .map(|command| command.command.clone())
        .collect();
    if failed.is_empty() {
        return;
    }
    let listed = failed.join("; ");
    result.status = WorkflowV2Status::NeedsReview;
    result.residual_gaps.push(crate::WorkflowV2ResidualGap {
        id: "failed_test_command_verification".to_string(),
        description: format!(
            "{} test command(s) in this verification failed, so the accepted verdict is not \
             supported by its own evidence: {listed}",
            failed.len()
        ),
        severity: Some("review".to_string()),
    });
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "accepted verification demoted: recorded test commands failed",
    ));
    let mut data = result.data.as_object().cloned().unwrap_or_default();
    data.insert(
        "failed_test_commands".to_string(),
        serde_json::json!(failed),
    );
    data.insert(
        "verification_failure_class".to_string(),
        serde_json::json!("actionable_verification_failure"),
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
        .filter(|command| command.kind == crate::WorkflowV2CommandKind::Test)
        .collect();
    if test_commands.is_empty() {
        return;
    }
    // ANY zero-match test command demotes, not only "all of them". A single
    // command carrying several filters can report overall success while named
    // filters inside it matched nothing — that command proves nothing about
    // those filters, and treating the batch as passing credits untested work.
    let any_zero_matched = test_commands
        .iter()
        .any(|command| crate::context::output_reports_zero_matched_tests(&command.output_summary));
    if !any_zero_matched {
        return;
    }
    result.status = WorkflowV2Status::NeedsReview;
    result.residual_gaps.push(
        crate::WorkflowV2ResidualGap {
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
