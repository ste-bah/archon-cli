use crate::v2::{
    BranchFailureKind, WorkflowV2BranchOutcome, WorkflowV2CommandStatus, WorkflowV2Result,
    WorkflowV2Status,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VerificationFailureClass {
    RetryableVerificationIssue,
    ActionableImplementationFailure,
    ExternalUnavailable,
    ContractFailure,
    RuntimeOrSafetyFailure,
}

impl VerificationFailureClass {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::RetryableVerificationIssue => "retryable_verification_issue",
            Self::ActionableImplementationFailure => "actionable_implementation_failure",
            Self::ExternalUnavailable => "external_unavailable",
            Self::ContractFailure => "contract_failure",
            Self::RuntimeOrSafetyFailure => "runtime_or_safety_failure",
        }
    }

    fn next_action(self) -> &'static str {
        match self {
            Self::RetryableVerificationIssue => "retry_verification",
            Self::ActionableImplementationFailure => "write_remediation",
            Self::ExternalUnavailable => "blocked_external",
            Self::ContractFailure => "repair_contract",
            Self::RuntimeOrSafetyFailure => "terminal_runtime_or_safety",
        }
    }
}

pub(super) fn annotate_verification_failure_outcome(
    call_id: &str,
    outcome: &mut WorkflowV2BranchOutcome,
) {
    if !is_focused_verification_call(call_id) || is_terminal_outcome(outcome) {
        return;
    }
    if matches!(
        outcome.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) {
        return;
    }
    let Some(result) = outcome.result.as_mut() else {
        return;
    };
    let inferred_task_ids = inferred_task_ids(result, &outcome.item_id);
    let class = classify_verification_result(result);
    let reason = classification_reason(result, class);
    let mut data = result.data.as_object().cloned().unwrap_or_default();
    if !inferred_task_ids.is_empty() && data.get("canonical_task_ids").is_none() {
        data.insert(
            "canonical_task_ids".to_string(),
            serde_json::json!(inferred_task_ids),
        );
    }
    if data.get("source_item_id").is_none() {
        data.insert(
            "source_item_id".to_string(),
            serde_json::json!(outcome.item_id.clone()),
        );
    }
    data.insert(
        "verification_failure_class".to_string(),
        serde_json::json!(class.as_str()),
    );
    data.insert(
        "verification_failure_next_action".to_string(),
        serde_json::json!(class.next_action()),
    );
    data.insert(
        "verification_failure_reason".to_string(),
        serde_json::json!(reason),
    );
    data.insert(
        "verification_remediation_required".to_string(),
        serde_json::json!(matches!(
            class,
            VerificationFailureClass::ActionableImplementationFailure
        )),
    );
    result.data = serde_json::Value::Object(data);
}

pub(super) fn classify_verification_result(result: &WorkflowV2Result) -> VerificationFailureClass {
    let text = verification_text(result).to_ascii_lowercase();
    if runtime_or_safety_text(&text) {
        return VerificationFailureClass::RuntimeOrSafetyFailure;
    }
    if zero_matched_text(&text) {
        return VerificationFailureClass::RetryableVerificationIssue;
    }
    if provider_proof_mismatch_text(&text) {
        return VerificationFailureClass::ActionableImplementationFailure;
    }
    if provider_credential_text(&text) {
        return match provider_env_proof_state(result) {
            Some("missing") => VerificationFailureClass::ExternalUnavailable,
            Some("present") => VerificationFailureClass::ActionableImplementationFailure,
            _ => VerificationFailureClass::ContractFailure,
        };
    }
    if missing_concrete_artifact_failure(result) {
        return VerificationFailureClass::ActionableImplementationFailure;
    }
    if has_intended_failures(result)
        || explicit_intended_failure_count(result) > 0
            && failed_test_command_with_failures(result, &text)
    {
        return VerificationFailureClass::ActionableImplementationFailure;
    }
    if external_unavailable_text(&text) {
        return VerificationFailureClass::ExternalUnavailable;
    }
    if result.commands_run.is_empty() || missing_core_contract(result) {
        return VerificationFailureClass::ContractFailure;
    }
    VerificationFailureClass::RetryableVerificationIssue
}

fn is_focused_verification_call(call_id: &str) -> bool {
    call_id.starts_with("verification-wave-") || call_id.starts_with("review-verification-wave-")
}

fn is_terminal_outcome(outcome: &WorkflowV2BranchOutcome) -> bool {
    matches!(
        outcome.failure_kind,
        Some(BranchFailureKind::Execution | BranchFailureKind::Safety)
    ) || matches!(outcome.status, WorkflowV2Status::Cancelled)
}

fn has_intended_failures(result: &WorkflowV2Result) -> bool {
    result
        .data
        .get("pass_fail_count")
        .and_then(|value| value.get("intended_target_failed"))
        .and_then(serde_json::Value::as_u64)
        .is_some_and(|count| count > 0)
}

fn explicit_intended_failure_count(result: &WorkflowV2Result) -> usize {
    if let Some(count) = result
        .data
        .get("pass_fail_count")
        .and_then(|value| value.get("intended_target_failed"))
        .and_then(serde_json::Value::as_u64)
    {
        return count as usize;
    }
    if let Some(count) = result
        .data
        .get("fail_count")
        .and_then(serde_json::Value::as_u64)
    {
        return count as usize;
    }
    result
        .data
        .get("matched_test_check_names")
        .and_then(|value| value.get("failed"))
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or_default()
}

fn missing_concrete_artifact_failure(result: &WorkflowV2Result) -> bool {
    let checked_artifacts = result
        .data
        .get("artifacts_checked")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|items| !items.is_empty());
    let missing_gap = result.residual_gaps.iter().any(|gap| {
        let text = format!("{} {}", gap.id, gap.description).to_ascii_lowercase();
        (text.contains("missing") || text.contains("absent"))
            && (text.contains("artifact") || text.contains("registry"))
    });
    checked_artifacts && explicit_intended_failure_count(result) > 0 && missing_gap
}

fn inferred_task_ids(result: &WorkflowV2Result, item_id: &str) -> Vec<String> {
    let mut ids = result
        .task_coverage
        .iter()
        .map(|coverage| coverage.task_id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect::<Vec<_>>();
    ids.extend(task_ids_in_text(item_id));
    ids.sort();
    ids.dedup();
    ids
}

fn task_ids_in_text(text: &str) -> Vec<String> {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-'))
        .filter(|part| {
            part.starts_with("TASK-") && part.chars().filter(|ch| *ch == '-').count() >= 2
        })
        .map(str::to_string)
        .collect()
}

fn failed_test_command_with_failures(result: &WorkflowV2Result, text: &str) -> bool {
    let failed_command = result.commands_run.iter().any(|command| {
        command.status == WorkflowV2CommandStatus::Failed
            || command.exit_code.is_some_and(|code| code != 0)
    });
    failed_command
        && (text.contains("failing tests:")
            || text.contains("failures:")
            || text.contains("intended target failures")
            || text.contains("test result: failed"))
}

fn missing_core_contract(result: &WorkflowV2Result) -> bool {
    result.data.get("canonical_task_ids").is_none()
        && result.task_coverage.is_empty()
        && result.commands_run.is_empty()
}

fn classification_reason(result: &WorkflowV2Result, class: VerificationFailureClass) -> String {
    match class {
        VerificationFailureClass::ActionableImplementationFailure => {
            let failures = result
                .data
                .get("matched_test_check_names")
                .and_then(|value| value.get("failed"))
                .and_then(serde_json::Value::as_array)
                .map(|items| items.len())
                .unwrap_or_default();
            format!("focused verification found {failures} intended failing check(s)")
        }
        VerificationFailureClass::RetryableVerificationIssue => {
            "focused verification shape or target selection can be retried".to_string()
        }
        VerificationFailureClass::ExternalUnavailable => {
            "verification evidence depends on unavailable external/provider input".to_string()
        }
        VerificationFailureClass::ContractFailure => {
            "verification branch result is missing required structured evidence".to_string()
        }
        VerificationFailureClass::RuntimeOrSafetyFailure => {
            "verification branch failed through runtime or safety path".to_string()
        }
    }
}

fn runtime_or_safety_text(text: &str) -> bool {
    text.contains("agent transport failed")
        || text.contains("tool execution failed")
        || text.contains("process failed")
        || text.contains("timed out")
        || text.contains("cancelled")
        || text.contains("rate limit")
        || text.contains("outside declared")
        || text.contains("ownership violation")
        || text.contains("read-only mutation")
}

fn external_unavailable_text(text: &str) -> bool {
    let unavailable = text.contains("unavailable")
        || text.contains("not configured")
        || text.contains("cannot produce");
    text.contains("credential") && text.contains("missing")
        || unavailable
            && (text.contains("provider")
                || text.contains("external service")
                || text.contains("external api")
                || text.contains("external dependency"))
}

fn provider_credential_text(text: &str) -> bool {
    (text.contains("credential")
        || text.contains("api key")
        || text.contains("env key")
        || text.contains("token")
        || text.contains("auth"))
        && (text.contains("missing") || text.contains("not configured") || text.contains("403"))
}

fn provider_proof_mismatch_text(text: &str) -> bool {
    let mismatch = text.contains("provider-env-status-mismatch")
        || text.contains("provider-env-proof-mismatch")
        || text.contains("conflicts with current redacted provider_env_proof")
        || text.contains("while redacted environment proof reports");
    mismatch && (text.contains("artifact") || text.contains("provider_environment"))
}

fn provider_env_proof_state(result: &WorkflowV2Result) -> Option<&str> {
    result
        .data
        .get("provider_env_proof")
        .and_then(|proof| proof.get("credential_state"))
        .and_then(serde_json::Value::as_str)
}

fn zero_matched_text(text: &str) -> bool {
    text.contains("zero matched")
        || text.contains("no target test matched")
        || text.contains("no tests matched")
        || text.contains("0 passed") && no_failures_reported(text) && text.contains("filtered out")
}

fn no_failures_reported(text: &str) -> bool {
    text.contains("0 failed")
        && !text.contains("1 failed")
        && !text.contains("test result: failed")
        && !text.contains("failures:")
}

fn verification_text(result: &WorkflowV2Result) -> String {
    let mut parts = vec![result.summary.clone()];
    parts.extend(result.evidence.iter().map(|item| item.summary.clone()));
    parts.extend(result.commands_run.iter().flat_map(|command| {
        [
            command.output_summary.clone(),
            format!("{:?}", command.status),
        ]
    }));
    parts.extend(result.residual_gaps.iter().flat_map(|gap| {
        [
            gap.id.clone(),
            gap.description.clone(),
            gap.severity.clone().unwrap_or_default(),
        ]
    }));
    parts.extend(verification_data_signals(result));
    parts.join("\n")
}

fn verification_data_signals(result: &WorkflowV2Result) -> Vec<String> {
    [
        "provider_env_proof",
        "pass_fail_count",
        "matched_test_check_names",
        "missing_required_artifact_path_fields",
        "artifact_evidence_status",
        "failure_status",
        "status",
    ]
    .iter()
    .filter_map(|key| result.data.get(*key))
    .map(serde_json::Value::to_string)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_live_generated_contract_wf2d24_data_store_failure_requires_write_remediation() {
        let outcome: WorkflowV2BranchOutcome = serde_json::from_str(
            archon_test_support::fixtures::WF2D24_VERIFICATION_WAVE_1_3_DATA_STORE_FAILED,
        )
        .expect("fixture");
        let result = outcome.result.expect("result");

        assert_eq!(
            classify_verification_result(&result),
            VerificationFailureClass::ActionableImplementationFailure
        );
    }

    #[test]
    fn workflow_live_verification_missing_provider_proof_is_contract_failure() {
        let result: WorkflowV2Result = serde_json::from_str(
            archon_test_support::fixtures::WFF9_PROVIDER_MISSING_ENV_PROOF_RESULT,
        )
        .expect("fixture");

        assert_eq!(
            classify_verification_result(&result),
            VerificationFailureClass::ContractFailure
        );
    }

    #[test]
    fn workflow_live_verification_verifier_shape_failure_is_not_actionable() {
        let outcome: WorkflowV2BranchOutcome = serde_json::from_str(
            archon_test_support::fixtures::WFFED_VERIFICATION_WAVE_1_1_BAD_BRANCH,
        )
        .expect("fixture");
        let result = outcome.result.expect("result");

        assert_eq!(
            classify_verification_result(&result),
            VerificationFailureClass::RetryableVerificationIssue
        );
    }

    #[test]
    fn workflow_live_verification_present_provider_proof_is_not_missing_credential_blocker() {
        let result = WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            summary: "provider returned missing credentials for API key".to_string(),
            commands_run: vec![crate::WorkflowV2CommandRecord {
                kind: crate::WorkflowV2CommandKind::Other,
                command: "provider check".to_string(),
                status: WorkflowV2CommandStatus::Failed,
                exit_code: Some(1),
                output_summary: "403 missing API key".to_string(),
            }],
            data: serde_json::json!({
                "canonical_task_ids": ["TASK-X-001"],
                "provider_env_proof": {
                    "credential_state": "present",
                    "redacted_env_keys_checked": [{"key": "POLYGON_API_KEY", "state": "present"}]
                }
            }),
            ..WorkflowV2Result::default()
        };

        assert_eq!(
            classify_verification_result(&result),
            VerificationFailureClass::ActionableImplementationFailure
        );
    }

    #[test]
    fn provider_artifact_mismatch_requires_write_remediation() {
        let result = WorkflowV2Result {
            status: WorkflowV2Status::Failed,
            summary:
                "artifact reports API key missing while redacted environment proof reports present"
                    .to_string(),
            residual_gaps: vec![crate::WorkflowV2ResidualGap {
                id: "provider-env-status-mismatch".to_string(),
                description:
                    "provider_environment conflicts with current redacted provider_env_proof"
                        .to_string(),
                severity: Some("high".to_string()),
            }],
            data: serde_json::json!({
                "canonical_task_ids": ["TASK-FIXTURE-030"],
                "provider_env_proof": { "credential_state": "missing" }
            }),
            ..WorkflowV2Result::default()
        };

        assert_eq!(
            classify_verification_result(&result),
            VerificationFailureClass::ActionableImplementationFailure
        );
    }

    #[test]
    fn workflow_live_verification_zero_passed_with_failed_test_requires_remediation() {
        let result = WorkflowV2Result {
            status: WorkflowV2Status::Failed,
            summary: "focused test failed: 0 passed, 1 failed".to_string(),
            commands_run: vec![crate::WorkflowV2CommandRecord {
                kind: crate::WorkflowV2CommandKind::Test,
                command: "cargo test focused_check".to_string(),
                status: WorkflowV2CommandStatus::Failed,
                exit_code: Some(101),
                output_summary: "test result: failed. 0 passed; 1 failed".to_string(),
            }],
            data: serde_json::json!({
                "canonical_task_ids": ["TASK-X-001"],
                "pass_fail_count": { "intended_target_failed": 1 }
            }),
            ..WorkflowV2Result::default()
        };

        assert_eq!(
            classify_verification_result(&result),
            VerificationFailureClass::ActionableImplementationFailure
        );
    }

    #[test]
    fn missing_concrete_artifacts_route_to_write_remediation() {
        let result: WorkflowV2Result = serde_json::from_str(
            archon_test_support::fixtures::WF346_VERIFICATION_MISSING_PROJECT_ARTIFACTS,
        )
        .expect("fixture");

        assert_eq!(
            classify_verification_result(&result),
            VerificationFailureClass::ActionableImplementationFailure
        );
    }
}
