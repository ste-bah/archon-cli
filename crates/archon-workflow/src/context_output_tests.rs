use super::{
    output_reports_blocked, output_reports_failed_execution, output_reports_failed_verification,
};

#[test]
fn confirmation_seeking_output_is_blocked() {
    let body = "I inspected the files and have a plan. Would you like me to proceed?";
    let reason = output_reports_blocked(body).expect("confirmation request must block");

    assert!(reason.contains("confirmation"), "{reason}");
}

#[test]
fn explicit_blocked_status_is_blocked() {
    let body = "**Status:** `blocked`\n\nMissing provider credentials.";
    let reason = output_reports_blocked(body).expect("blocked status must block");

    assert!(reason.contains("blocked"), "{reason}");
}

#[test]
fn top_level_json_blocked_status_is_blocked() {
    let body = r#"{
        "status": "blocked",
        "missing_evidence": ["provider credentials unavailable"]
    }"#;
    let reason = output_reports_blocked(body).expect("top-level blocked status must block");

    assert!(reason.contains("blocked"), "{reason}");
}

#[test]
fn item_dependency_blocked_status_does_not_block_item_producer() {
    let body = r#"{
        "items": [
            {
                "id": "TASK-TDL-020",
                "task_id": "TASK-TDL-020",
                "status": "blocked",
                "depends_on": ["TASK-TDL-010"],
                "evidence": ["Task dependency says this item waits for T010."],
                "target_files": ["crates/archon-trading/src/data_lake.rs"]
            }
        ]
    }"#;

    assert_eq!(output_reports_blocked(body), None);
}

#[test]
fn quoted_stale_status_blocked_evidence_does_not_block_review_artifact() {
    let body = r#"
## Read-only adversarial review

**Status:** `accepted_with_must_fix_constraints`

### Major conflicts / stale assumptions

Discover artifact says "Coverage matrix command exists"; task file still says `status: blocked`.
This is acceptable only if downstream treats T080 as verification/hardening, not initial implementation.
"#;

    assert_eq!(output_reports_blocked(body), None);
}

#[test]
fn accepted_status_without_evidence_is_rejected() {
    let reason = output_reports_failed_verification(r#"{"status":"accepted"}"#)
        .expect("accepted stub should be rejected");
    assert!(
        reason.contains("required evidence fields"),
        "reason should name evidence failure: {reason}"
    );
}

#[test]
fn accepted_status_without_implementation_evidence_is_allowed_for_execution_contract() {
    assert_eq!(
        output_reports_failed_execution(
            r#"{"status":"accepted","findings":[],"summary":"review artifact"}"#
        ),
        None
    );
}

#[test]
fn failed_execution_status_still_fails_execution_contract() {
    assert!(output_reports_failed_execution(r#"{"status":"failed"}"#).is_some());
}

#[test]
fn accepted_status_with_required_evidence_is_allowed() {
    let body = r#"
status: accepted
target_files:
  - src/lib.rs
acceptance_checks:
  - checked declared target
commands_run:
  - command: cargo test -p archon-workflow context_output
residual_gaps: []
"#;

    assert_eq!(output_reports_failed_verification(body), None);
}

#[test]
fn accepted_with_findings_is_not_bare_accepted_signoff() {
    let body = r#"{
        "status": "accepted_with_findings",
        "commands_run": [{"command": "rg write_ahdm src", "exit_status": 0}],
        "findings": [{"severity": "medium", "evidence": "src/command/trading.rs needs follow-up"}]
    }"#;

    assert_eq!(output_reports_failed_verification(body), None);
}

#[test]
fn accepted_json_with_implementation_evidence_is_allowed() {
    let body = r#"{
        "status": "accepted",
        "changed_files": ["src/lib.rs"],
        "commands_run": [
            {"command": "cargo test -p archon-workflow context_output", "exit_status": 0}
        ],
        "line_count_evidence": {"src/lib.rs": 42},
        "residual_gaps": []
    }"#;

    assert_eq!(output_reports_failed_verification(body), None);
}

#[test]
fn accepted_json_with_verification_evidence_is_allowed() {
    let body = r#"{
        "status": "accepted",
        "changed_files": ["crates/archon-trading/src/data_store.rs"],
        "line_count_evidence": "Largest changed source file is 449 lines.",
        "residual_gaps": [],
        "verification": [
            {
                "command": "cargo test -p archon-trading data_store -- --nocapture",
                "exit_status": 0,
                "result": "18 passed; 0 failed"
            }
        ]
    }"#;

    assert_eq!(output_reports_failed_verification(body), None);
}

#[test]
fn accepted_json_with_tests_evidence_is_allowed() {
    let body = r#"{
        "status": "accepted",
        "changed_files": [
            "src/command/trading_data_provider.rs",
            "src/command/trading_data_provider_tests.rs"
        ],
        "line_counts": {
            "src/command/trading_data_provider.rs": 332,
            "src/command/trading_data_provider_tests.rs": 254
        },
        "residual_gaps": "TradingView live MCP execution depends on project-local tooling.",
        "tests": [
            {
                "command": "cargo test --bin archon trading_data_provider_tests -- --nocapture",
                "exit_status": 0,
                "output": "7 passed; 0 failed"
            }
        ]
    }"#;

    assert_eq!(output_reports_failed_verification(body), None);
}

#[test]
fn accepted_json_with_generated_agent_evidence_aliases_is_allowed() {
    let body = r#"{
        "status": "accepted",
        "completed_task_ids": ["TASK-TDL-010"],
        "source_files_changed": [
            "crates/archon-trading/src/data_store.rs",
            "crates/archon-trading/src/data_store/io.rs"
        ],
        "implementation_summary": "implemented v2 data-store migration and fail-closed metadata",
        "file_size_check": {
            "crates/archon-trading/src/data_store.rs": 497
        },
        "focused_tests": [
            {
                "command": "cargo test -p archon-trading data_store::tests::metadata_json_contains_self_describing_paths_and_checksums",
                "exit_status": 0,
                "result": "passed"
            }
        ],
        "notes": "No residual gaps for this task."
    }"#;

    assert_eq!(output_reports_failed_verification(body), None);
}

#[test]
fn emergency_completed_focused_tests_with_positive_matches_are_allowed() {
    let body = r#"{
        "status": "completed_with_emerg_condition",
        "summary": {"commands_run": 2, "passed": 2, "failed": 0},
        "results": [
            {
                "command": "cargo test --bin archon trading_data_provider_tests -- --nocapture",
                "exit_status": 0,
                "stdout": "test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 124 filtered out."
            },
            {
                "command": "cargo test --bin archon focused_case -- --nocapture",
                "exit_status": 0,
                "stdout": "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1007 filtered out."
            }
        ],
        "residual_gaps": []
    }"#;

    assert_eq!(output_reports_failed_execution(body), None);
    assert_eq!(output_reports_failed_verification(body), None);
}

#[test]
fn wrapped_accepted_json_with_verification_evidence_is_allowed() {
    let body = r#"{
        "body": {
            "status": "accepted",
            "changed_files": ["src/lib.rs"],
            "residual_gaps": [],
            "verification": [
                {"command": "cargo test -p archon-workflow context_output", "exit_status": 0}
            ]
        }
    }"#;

    assert_eq!(output_reports_failed_verification(body), None);
}

#[test]
fn accepted_json_without_command_evidence_is_rejected() {
    let reason = output_reports_failed_verification(
        r#"{"status":"accepted","changed_files":["src/lib.rs"],"residual_gaps":[]}"#,
    )
    .expect("accepted implementation artifact without command evidence should be rejected");
    assert!(
        reason.contains("required evidence fields"),
        "reason should name evidence failure: {reason}"
    );
}

#[test]
fn accepted_false_verification_artifact_is_rejected() {
    let reason = output_reports_failed_verification(
        r#"{
            "accepted": false,
            "status": "failed-partially-verified",
            "commands_run": [{"command": "cargo test -p invalid"}],
            "residual_gaps": ["invalid package id"]
        }"#,
    )
    .expect("accepted=false content must reject the artifact");

    assert!(
        reason.contains("accepted=false") || reason.contains("failed"),
        "reason should name failed verification evidence: {reason}"
    );
}

#[test]
fn zero_test_filtered_command_artifact_is_rejected() {
    let reason = output_reports_failed_verification(
        r#"{
            "status": "accepted",
            "changed_files": ["src/lib.rs"],
            "commands_run": [
                {"command": "cargo test -p demo missing_filter", "result": "running 0 tests"}
            ],
            "residual_gaps": []
        }"#,
    )
    .expect("0-test filtered command must reject the artifact");

    assert!(
        reason.contains("zero tests"),
        "reason should name zero-test filtering: {reason}"
    );
}

#[test]
fn current_accepted_json_cannot_borrow_prior_attempt_evidence() {
    let reason = output_reports_failed_verification(
        r#"{
            "status": "accepted",
            "changed_files": ["src/lib.rs"],
            "residual_gaps": [],
            "prior_attempts": [
                {
                    "commands_run": [
                        {"command": "cargo test stale", "exit_status": 0}
                    ]
                }
            ]
        }"#,
    )
    .expect("current accepted output should carry current command evidence");
    assert!(
        reason.contains("required evidence fields"),
        "reason should name evidence failure: {reason}"
    );
}
