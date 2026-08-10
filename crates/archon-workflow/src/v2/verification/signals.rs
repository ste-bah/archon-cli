use crate::v2::{WorkflowV2CommandStatus, WorkflowV2Result};

pub(super) fn is_duplicate_harness_gap(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    (lower.contains("duplicate") && lower.contains("targeted") && lower.contains("test"))
        || (lower.contains("reported") && lower.contains("twice") && lower.contains("passed"))
        || lower.contains("same fully qualified test passing twice")
        || lower.contains("same exact test passing in two")
}

pub(super) fn focused_verification_command_passed(result: &WorkflowV2Result) -> bool {
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

pub(super) fn verification_text(result: &WorkflowV2Result) -> Vec<String> {
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
mod declared_contract_enforcement_tests {
    use super::super::contracts::{
        ContractVerification, demote_failed_contract, enforce_declared_contracts,
        run_contract_verifier, stamp_passed_contracts, verdict_failure, verifier_verdicts,
    };
    use crate::v2::{
        BranchFailureKind, WorkflowV2BranchOutcome, WorkflowV2Result, WorkflowV2Status,
    };

    fn accepted_outcome(item_id: &str) -> WorkflowV2BranchOutcome {
        WorkflowV2BranchOutcome {
            item_id: item_id.to_string(),
            role: "implementer".to_string(),
            status: WorkflowV2Status::Accepted,
            result: Some(WorkflowV2Result::accepted("branch claims acceptance")),
            error: None,
            failure_kind: None,
            item_input_hash: Some("input".to_string()),
            completion_evidence: Vec::new(),
        }
    }

    fn failure_detail(verification: ContractVerification) -> String {
        match verification {
            ContractVerification::Failed(detail) => detail,
            ContractVerification::Passed => panic!("expected the verifier to fail closed"),
        }
    }

    #[tokio::test]
    async fn typed_precheck_pass_cannot_mask_a_contract_verifier_failure() {
        // verification_command chains the typed pre-check AHEAD of the contract
        // verifier, so a permissive `verified` is printed first. Reading only
        // the first verdict is what let fabricated artifacts pass as verified.
        let detail = failure_detail(
            run_contract_verifier(
                r#"printf '%s\n' '{"status":"verified","verified_cells":3}' '{"status":"failed","failure_count":1,"failures":["close steps are constant"]}'; exit 1"#,
            )
            .await,
        );
        assert_eq!(detail, "close steps are constant");
    }

    #[tokio::test]
    async fn failures_reported_without_a_status_field_are_still_caught() {
        // The verifier's early exits print `{"failures": [...]}` with no status
        // field; that text is the only account of what actually broke.
        let detail = failure_detail(
            run_contract_verifier(
                r#"printf '%s\n' '{"failures":["declared deliverable is missing"]}'; exit 1"#,
            )
            .await,
        );
        assert_eq!(detail, "declared deliverable is missing");
    }

    #[tokio::test]
    async fn a_clean_terminal_verdict_passes() {
        let verification = run_contract_verifier(
            r#"printf '%s\n' '{"status":"verified"}' '{"status":"substantive_deliverable_verified","verified_cells":4}'"#,
        )
        .await;
        assert!(matches!(verification, ContractVerification::Passed));
    }

    #[tokio::test]
    async fn exit_zero_without_any_verdict_fails_closed() {
        // "We could not check" is never a pass.
        let detail = failure_detail(run_contract_verifier("echo 'no json here'").await);
        assert!(detail.contains("no parseable status"), "{detail}");
    }

    #[tokio::test]
    async fn a_nonzero_exit_with_no_verdict_fails_closed() {
        let detail = failure_detail(run_contract_verifier("echo 'boom' >&2; exit 3").await);
        assert!(detail.contains("exited non-zero"), "{detail}");
        assert!(detail.contains("boom"), "{detail}");
    }

    /// END-TO-END, both directions. The helper test below proves the stamp
    /// writes; this proves ENFORCEMENT actually calls it, which is the seam
    /// that was silently missing. A trace that has only ever been seen green
    /// is not evidence — so the same contract shape is driven to a pass and to
    /// a failure, and the two must be distinguishable.
    #[tokio::test]
    async fn enforcement_records_a_pass_and_a_failure_differently() {
        let project = tempfile::tempdir().expect("project");
        let root = project.path().to_str().expect("root").to_string();
        let present = project.path().join(".archon/demo/present.json");
        std::fs::create_dir_all(present.parent().expect("parent")).expect("dir");
        std::fs::write(&present, r#"{"ok":true}"#).expect("artifact");

        // Same contract shape; only the declared artifact differs.
        let passing = serde_json::json!({"artifact_path": ".archon/demo/present.json"});
        let failing = serde_json::json!({"artifact_path": ".archon/demo/absent.json"});

        let mut outcomes = [
            accepted_outcome("verify-passes"),
            accepted_outcome("verify-fails"),
        ];
        let contracts = std::collections::BTreeMap::from([
            ("verify-passes".to_string(), (root.clone(), vec![passing])),
            ("verify-fails".to_string(), (root, vec![failing])),
        ]);
        enforce_declared_contracts(&mut outcomes, &contracts).await;

        let passed = outcomes[0].result.as_ref().expect("result");
        assert_eq!(outcomes[0].status, WorkflowV2Status::Accepted);
        assert_eq!(passed.data["declared_contract_verification"], "passed");
        assert_eq!(passed.data["declared_contracts_verified"], 1);

        let failed = outcomes[1].result.as_ref().expect("result");
        assert_eq!(outcomes[1].status, WorkflowV2Status::NeedsReview);
        assert_eq!(failed.data["declared_contract_verification"], "failed");
        assert_eq!(
            failed.data["verification_failure_class"],
            "declared_contract_violation"
        );
    }

    /// A gate you cannot observe succeeding is indistinguishable from one that
    /// never ran — the ambiguity that hid this enforcement being dead.
    #[tokio::test]
    async fn a_passing_contract_leaves_a_positive_trace() {
        let mut outcomes = [accepted_outcome("verify-task-ex-001")];
        let contracts = std::collections::BTreeMap::from([(
            "verify-task-ex-001".to_string(),
            (
                "/proj".to_string(),
                vec![serde_json::json!({"artifact_path": ".archon/none.json"})],
            ),
        )]);
        // Stub the verifier's verdict shape rather than the contract itself:
        // what matters is that a clean pass is recorded, not silently dropped.
        let verification = run_contract_verifier(r#"printf '%s\n' '{"status":"verified"}'"#).await;
        assert!(matches!(verification, ContractVerification::Passed));
        stamp_passed_contracts(&mut outcomes[0], 1);
        let data = &outcomes[0].result.as_ref().expect("result").data;
        assert_eq!(data["declared_contract_verification"], "passed");
        assert_eq!(data["declared_contracts_verified"], 1);
        assert_eq!(outcomes[0].status, WorkflowV2Status::Accepted);
        let _ = contracts;
    }

    #[tokio::test]
    async fn enforcement_is_inert_when_no_item_declared_a_contract() {
        let mut outcomes = [accepted_outcome("build-thing")];
        enforce_declared_contracts(&mut outcomes, &std::collections::BTreeMap::new()).await;
        assert_eq!(outcomes[0].status, WorkflowV2Status::Accepted);
    }

    #[test]
    fn every_printed_object_is_collected_in_order() {
        let verdicts = verifier_verdicts(
            "noise {\"status\":\"verified\",\"nested\":{\"cells\":2}} tail\n{\"status\":\"failed\"}\n",
        );
        assert_eq!(verdicts.len(), 2, "{verdicts:?}");
        assert_eq!(verdicts[0]["status"], "verified");
        assert_eq!(verdicts[1]["status"], "failed");
    }

    #[test]
    fn an_empty_failures_array_is_not_a_failure() {
        assert!(verdict_failure(&serde_json::json!({"status": "ok", "failures": []})).is_none());
        assert!(verdict_failure(&serde_json::json!({"status": "verified"})).is_none());
    }

    #[test]
    fn a_failed_contract_demotes_the_branch_with_a_typed_gap() {
        let mut outcome = accepted_outcome("implement-tdl-080");
        demote_failed_contract(&mut outcome, "close steps are constant");
        assert_eq!(outcome.status, WorkflowV2Status::NeedsReview);
        assert_eq!(outcome.failure_kind, Some(BranchFailureKind::Semantic));
        let result = outcome.result.expect("result");
        assert_eq!(
            result.data["verification_failure_class"],
            "declared_contract_violation"
        );
        assert!(
            result
                .residual_gaps
                .iter()
                .any(|gap| gap.id == "declared_contract_verification_failed"
                    && gap.description.contains("close steps are constant")),
            "{:?}",
            result.residual_gaps
        );
    }
}

#[cfg(test)]
mod commandless_demotion_tests {
    use super::super::normalize::normalize_focused_verification_outcome;
    use crate::v2::{
        BranchFailureKind, WorkflowV2BranchOutcome, WorkflowV2CommandKind, WorkflowV2CommandRecord,
        WorkflowV2CommandStatus, WorkflowV2Result, WorkflowV2Status,
    };

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
