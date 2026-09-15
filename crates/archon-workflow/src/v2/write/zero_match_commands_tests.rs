use super::*;
use crate::v2::{WorkflowV2CommandKind, WorkflowV2CommandRecord, WorkflowV2Status};

fn command(
    kind: WorkflowV2CommandKind,
    text: &str,
    status: WorkflowV2CommandStatus,
    output: &str,
) -> WorkflowV2CommandRecord {
    WorkflowV2CommandRecord {
        kind,
        command: text.to_string(),
        status,
        exit_code: Some(if status == WorkflowV2CommandStatus::Succeeded {
            0
        } else {
            101
        }),
        output_summary: output.to_string(),
    }
}

fn accepted(commands: Vec<WorkflowV2CommandRecord>) -> WorkflowV2Result {
    WorkflowV2Result {
        status: WorkflowV2Status::Accepted,
        summary: "did the work".to_string(),
        commands_run: commands,
        files_changed: vec![crate::v2::WorkflowV2FileRecord::new("docs/report.md")],
        data: serde_json::json!({}),
        ..WorkflowV2Result::default()
    }
}

fn declared(commands: &[&str]) -> Vec<String> {
    commands.iter().map(|command| command.to_string()).collect()
}

/// The live shape (wf-719ff3b0 agents-2-0): a `--list` invocation whose
/// summary talks about "0 ... entries", real filtered runs with non-zero
/// counts, and prose about modules that "compile zero tests".
fn live_like_result() -> WorkflowV2Result {
    let mut result = accepted(vec![
        command(
            WorkflowV2CommandKind::Test,
            "cargo test -p demo --lib data_store",
            WorkflowV2CommandStatus::Failed,
            "61 passed; 32 failed; 155 filtered out",
        ),
        command(
            WorkflowV2CommandKind::Test,
            "cargo test -p demo --lib data_store::validation_tests",
            WorkflowV2CommandStatus::Succeeded,
            "13 passed; 0 failed",
        ),
        command(
            WorkflowV2CommandKind::Test,
            "cargo test -p demo --lib -- --list",
            WorkflowV2CommandStatus::Succeeded,
            "250 tests listed; used to verify module reachability (0 ingest/coverage_tests entries; running 0 tests there)",
        ),
        command(
            WorkflowV2CommandKind::Inspect,
            "focused test loop: test -s R; grep schema R; wc -l < R",
            WorkflowV2CommandStatus::Succeeded,
            "12/12 checks OK, ALL_FAIL=0, lines=154",
        ),
    ]);
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "cross-check against compiled test list: ingest families and coverage_tests compile zero tests",
    ));
    result
}

#[test]
fn live_list_and_prose_envelope_is_accepted_without_a_gap() {
    let mut result = live_like_result();
    let declared = declared(&["test -s R", "grep schema R"]);
    reject_declared_zero_match(&result, &declared).expect("no zero-match verdict");
    report_incidental_zero_match(&mut result, "agents-2-0", &declared);
    assert!(
        result.residual_gaps.is_empty(),
        "{:?}",
        result.residual_gaps
    );
    assert_eq!(result.status, WorkflowV2Status::Accepted);
    // The generic output gate sees the same envelope and agrees.
    let body = serde_json::to_string(&result).unwrap();
    assert_eq!(
        crate::context::output_reports_failed_verification(&body),
        None
    );
}

#[test]
fn declared_focused_test_that_ran_nothing_is_still_rejected() {
    let result = accepted(vec![command(
        WorkflowV2CommandKind::Test,
        "cargo test -p demo focused_case -- --nocapture",
        WorkflowV2CommandStatus::Failed,
        "running 0 tests\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 42 filtered out.",
    )]);
    let err = reject_declared_zero_match(&result, &declared(&["cargo test -p demo focused_case"]))
        .expect_err("a declared focused test matching zero tests is a false green");
    let text = err.to_string();
    assert!(text.contains("output not usable"), "{text}");
    assert!(text.contains("matched zero tests"), "{text}");
    assert!(
        text.contains("cargo test -p demo focused_case -- --nocapture"),
        "{text}"
    );
}

#[test]
fn zero_match_presented_as_succeeded_is_rejected_by_the_output_gate() {
    let result = accepted(vec![command(
        WorkflowV2CommandKind::Test,
        "cargo test -p wrong-crate some_test",
        WorkflowV2CommandStatus::Succeeded,
        "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 42 filtered out.",
    )]);
    assert!(reject_declared_zero_match(&result, &[]).is_err());
    let body = serde_json::to_string(&result).unwrap();
    let reason = crate::context::output_reports_failed_verification(&body).expect("rejected");
    assert!(
        reason.contains("cargo test -p wrong-crate some_test"),
        "{reason}"
    );
}

#[test]
fn zero_match_cited_in_evidence_is_rejected() {
    let mut result = accepted(vec![command(
        WorkflowV2CommandKind::Test,
        "cargo test -p demo stale_name",
        WorkflowV2CommandStatus::Failed,
        "running 0 tests",
    )]);
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Test,
        "verified by cargo test -p demo   stale_name",
    ));
    assert!(reject_declared_zero_match(&result, &[]).is_err());
}

#[test]
fn incidental_zero_match_is_a_review_gap_not_a_rejection() {
    let mut result = accepted(vec![
        command(
            WorkflowV2CommandKind::Test,
            "cargo test -p demo focused_case",
            WorkflowV2CommandStatus::Succeeded,
            "test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 40 filtered out.",
        ),
        command(
            WorkflowV2CommandKind::Test,
            "cargo test -p demo exploratory_filter",
            WorkflowV2CommandStatus::Failed,
            "running 0 tests",
        ),
    ]);
    let declared = declared(&["cargo test -p demo focused_case"]);
    reject_declared_zero_match(&result, &declared).expect("incidental zero match is not a verdict");
    let body = serde_json::to_string(&result).unwrap();
    assert_eq!(
        crate::context::output_reports_failed_verification(&body),
        None
    );
    report_incidental_zero_match(&mut result, "agents-2-0", &declared);
    assert_eq!(result.status, WorkflowV2Status::Accepted);
    let gap = result
        .residual_gaps
        .iter()
        .find(|gap| gap.id == "zero_match_test_command_agents-2-0")
        .expect("review gap");
    assert_eq!(gap.severity.as_deref(), Some("review"));
    assert!(
        gap.description
            .contains("cargo test -p demo exploratory_filter"),
        "{}",
        gap.description
    );
    assert!(
        !gap.description.contains("focused_case"),
        "{}",
        gap.description
    );
    assert_eq!(
        result.data["zero_match_test_commands"],
        serde_json::json!(["cargo test -p demo exploratory_filter"])
    );
}

#[test]
fn nextest_and_pytest_zero_forms_reject_declared_tests() {
    for (text, output) in [
        (
            "cargo nextest run -p demo focused_case",
            "Starting 0 tests across 2 binaries (120 skipped)",
        ),
        (
            "pytest tests/test_demo.py -k focused_case",
            "===== no tests ran in 0.02s =====",
        ),
        (
            "pytest tests/test_demo.py -k focused_case",
            "collected 0 items",
        ),
        (
            "go test ./pkg -run FocusedCase",
            "testing: warning: no tests to run",
        ),
        (
            "npx jest focused_case",
            "No tests found, exiting with code 1",
        ),
    ] {
        let result = accepted(vec![command(
            WorkflowV2CommandKind::Test,
            text,
            WorkflowV2CommandStatus::Failed,
            output,
        )]);
        assert!(
            reject_declared_zero_match(&result, &declared(&[text])).is_err(),
            "{text}: {output}"
        );
    }
}

#[test]
fn declared_tests_are_read_from_the_item_under_any_alias() {
    let input = serde_json::json!({
        "item": {"focused_verification": ["cargo test -p demo focused_case", {"command": "grep -q schema R"}]}
    });
    assert_eq!(
        declared_focused_tests(&input),
        vec![
            "cargo test -p demo focused_case".to_string(),
            "grep -q schema R".to_string()
        ]
    );
    let flat = serde_json::json!({"focusedTests": ["pytest -k focused"]});
    assert_eq!(
        declared_focused_tests(&flat),
        vec!["pytest -k focused".to_string()]
    );
    assert!(declared_focused_tests(&serde_json::json!({"item": {}})).is_empty());
}
