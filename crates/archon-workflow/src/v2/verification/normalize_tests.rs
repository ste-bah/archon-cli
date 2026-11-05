//! The `pre_existing` channel on `demote_failed_test_acceptance`.
//!
//! Run wf-719ff3b0 accepted TASK-DL-003 with a repo-wide file-size gate exiting
//! 1; the verifier attributed the failure to eight files the task never touched
//! (true at the baseline commit), but said so only in prose. The host demoted
//! on the exit code and remediation burned rounds on a gate no in-scope edit
//! could close. These tests pin the typed channel that replaces the prose —
//! and, just as much, the ways it must NOT be abusable.

use super::normalize_focused_verification_outcome;
use crate::v2::{
    WorkflowV2BranchOutcome, WorkflowV2CommandKind, WorkflowV2CommandRecord,
    WorkflowV2CommandStatus, WorkflowV2Result, WorkflowV2Status,
};

fn command(
    status: WorkflowV2CommandStatus,
    pre_existing: bool,
    output_summary: &str,
) -> WorkflowV2CommandRecord {
    WorkflowV2CommandRecord {
        kind: WorkflowV2CommandKind::Test,
        command: format!("gate-{}-{}", status as u8, pre_existing),
        status,
        exit_code: Some(if status == WorkflowV2CommandStatus::Failed {
            1
        } else {
            0
        }),
        output_summary: output_summary.to_string(),
        pre_existing,
    }
}

/// An accepted outcome with one genuinely passing command, so neither the
/// commandless nor the zero-match backstop fires and only the failed-test rule
/// is under test.
fn accepted_with(commands: Vec<WorkflowV2CommandRecord>) -> WorkflowV2BranchOutcome {
    let mut result = WorkflowV2Result::accepted("focused verification passed");
    result.commands_run.push(command(
        WorkflowV2CommandStatus::Succeeded,
        false,
        "test result: ok. 3 passed; 0 failed",
    ));
    result.commands_run.extend(commands);
    WorkflowV2BranchOutcome {
        item_id: "verify-task".to_string(),
        role: "verifier".to_string(),
        status: WorkflowV2Status::Accepted,
        result: Some(result),
        error: None,
        failure_kind: None,
        item_input_hash: None,
        completion_evidence: Vec::new(),
    }
}

fn normalized(outcome: &mut WorkflowV2BranchOutcome) -> &WorkflowV2Result {
    normalize_focused_verification_outcome("verification-wave-1", outcome);
    outcome
        .result
        .as_ref()
        .expect("result survives normalization")
}

fn gap_ids(result: &WorkflowV2Result) -> Vec<&str> {
    result
        .residual_gaps
        .iter()
        .map(|gap| gap.id.as_str())
        .collect()
}

/// The live shape: failure attributed with evidence. The verdict stands, the
/// attribution is surfaced for review, and the old demotion gap is absent.
#[test]
fn evidenced_pre_existing_failure_keeps_the_accepted_verdict() {
    let mut outcome = accepted_with(vec![command(
        WorkflowV2CommandStatus::Failed,
        true,
        "8 pre-existing files >500 lines, all outside the task's crate; none touched by this task",
    )]);
    let result = normalized(&mut outcome);

    assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    let ids = gap_ids(result);
    assert!(ids.contains(&"pre_existing_failing_command"), "{ids:?}");
    assert!(
        !ids.contains(&"failed_test_command_verification"),
        "{ids:?}"
    );
    let gap = result
        .residual_gaps
        .iter()
        .find(|gap| gap.id == "pre_existing_failing_command")
        .expect("attribution gap");
    assert_eq!(gap.severity.as_deref(), Some("review"));
    assert!(
        gap.description.contains("gate-1-true"),
        "{}",
        gap.description
    );
    assert_eq!(
        result.data["pre_existing_failing_commands"],
        serde_json::json!(["gate-1-true"])
    );
    assert!(
        result.data.get("failed_test_commands").is_none(),
        "{:#?}",
        result.data
    );
    assert_eq!(outcome.status, WorkflowV2Status::Accepted);
    assert!(outcome.failure_kind.is_none());
}

/// A bare flag is an assertion, not evidence. Without text in output_summary
/// the command is an ordinary failure and demotes exactly as before.
#[test]
fn pre_existing_flag_without_evidence_text_demotes_as_an_ordinary_failure() {
    let mut outcome = accepted_with(vec![command(WorkflowV2CommandStatus::Failed, true, "   ")]);
    let result = normalized(&mut outcome);

    assert_eq!(result.status, WorkflowV2Status::NeedsReview, "{result:#?}");
    let ids = gap_ids(result);
    assert!(ids.contains(&"failed_test_command_verification"), "{ids:?}");
    assert!(!ids.contains(&"pre_existing_failing_command"), "{ids:?}");
    assert!(result.data.get("pre_existing_failing_commands").is_none());
}

/// The envelope normaliser fills a missing output_summary with a placeholder
/// before the typed record exists, so "empty" alone would never be seen on the
/// live path. The host's own filler must not count as the verifier's evidence.
#[test]
fn pre_existing_flag_with_only_the_synthesized_placeholder_demotes() {
    let placeholder = format!(
        "{}; command status: failed)",
        crate::v2::agent_output_normalize::SYNTHESIZED_OUTPUT_SUMMARY_PREFIX
    );
    let mut outcome = accepted_with(vec![command(
        WorkflowV2CommandStatus::Failed,
        true,
        &placeholder,
    )]);
    let result = normalized(&mut outcome);

    assert_eq!(result.status, WorkflowV2Status::NeedsReview, "{result:#?}");
    assert!(gap_ids(result).contains(&"failed_test_command_verification"));
}

/// An ordinary failure alongside an attributed one: the ordinary failure wins
/// the verdict, and the attribution is still recorded so the reader sees both.
#[test]
fn ordinary_failure_beside_a_pre_existing_one_demotes_and_records_both() {
    let mut outcome = accepted_with(vec![
        command(
            WorkflowV2CommandStatus::Failed,
            true,
            "fails identically at the baseline commit",
        ),
        command(
            WorkflowV2CommandStatus::Failed,
            false,
            "assertion failed in the task's own test",
        ),
    ]);
    let result = normalized(&mut outcome);

    assert_eq!(result.status, WorkflowV2Status::NeedsReview, "{result:#?}");
    let ids = gap_ids(result);
    assert!(ids.contains(&"failed_test_command_verification"), "{ids:?}");
    assert!(ids.contains(&"pre_existing_failing_command"), "{ids:?}");
    assert_eq!(
        result.data["failed_test_commands"],
        serde_json::json!(["gate-1-false"]),
        "only the unattributed failure is charged to the task"
    );
    assert_eq!(
        result.data["pre_existing_failing_commands"],
        serde_json::json!(["gate-1-true"])
    );
}

/// The flag describes a failure. On a command that succeeded it is noise and
/// must produce neither a gap nor a data key.
#[test]
fn pre_existing_on_a_succeeded_command_is_ignored() {
    let mut outcome = accepted_with(vec![command(
        WorkflowV2CommandStatus::Succeeded,
        true,
        "test result: ok. 1 passed",
    )]);
    let result = normalized(&mut outcome);

    assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    assert!(gap_ids(result).is_empty(), "{:?}", gap_ids(result));
    assert!(result.data.get("pre_existing_failing_commands").is_none());
}

/// Wire compatibility: every stored envelope and fixture predates the field,
/// so absence must read as false, and false must not be written back — a
/// record that never carried the flag round-trips byte-identical.
#[test]
fn pre_existing_defaults_false_and_is_not_serialised_when_false() {
    let raw = serde_json::json!({
        "kind": "test",
        "command": "cargo test gate",
        "status": "failed",
        "exit_code": 1,
        "output_summary": "1 failed"
    });
    let parsed: WorkflowV2CommandRecord =
        serde_json::from_value(raw.clone()).expect("record without the field parses");
    assert!(!parsed.pre_existing);
    assert_eq!(serde_json::to_value(&parsed).expect("serialises"), raw);

    let flagged: WorkflowV2CommandRecord = serde_json::from_value(serde_json::json!({
        "kind": "test",
        "command": "cargo test gate",
        "status": "failed",
        "exit_code": 1,
        "output_summary": "fails at baseline",
        "pre_existing": true
    }))
    .expect("flagged record parses");
    assert!(flagged.pre_existing);
    assert_eq!(
        serde_json::to_value(&flagged).expect("serialises")["pre_existing"],
        serde_json::json!(true)
    );
}

/// The schema-less branch-evidence path rebuilds command records field by
/// field rather than through serde, so the flag has to be copied explicitly
/// or it is silently dropped before the demotion rule can see it.
#[test]
fn schema_less_branch_evidence_carries_the_flag_through_its_rebuild() {
    let result =
        crate::v2::branch_evidence::semantic_branch_result_from_value(&serde_json::json!({
            "canonical_task_ids": ["TASK-1"],
            "status": "accepted",
            "summary": "gate fails at baseline",
            "commands_run": [
                {"kind": "test", "command": "gate", "status": "failed", "exit_code": 1,
                 "output_summary": "fails identically at baseline", "pre_existing": true},
                {"kind": "test", "command": "own", "status": "succeeded", "exit_code": 0,
                 "output_summary": "ok"}
            ]
        }))
        .expect("branch evidence parses");
    let flags: Vec<bool> = result
        .commands_run
        .iter()
        .map(|command| command.pre_existing)
        .collect();
    assert_eq!(flags, vec![true, false]);
}

/// Issue-78: a zero-match test command no longer demotes on its own when it
/// carries an evidenced attribution AND another test command in the same
/// result matched tests and passed. Both conjuncts are pinned here, as is the
/// unchanged demotion for every other shape.
mod zero_match_supersession {
    use super::{gap_ids, normalized};
    use crate::v2::{
        WorkflowV2BranchOutcome, WorkflowV2CommandKind, WorkflowV2CommandRecord,
        WorkflowV2CommandStatus, WorkflowV2Result, WorkflowV2Status,
    };

    /// A captured zero-match run whose tail carries the verifier's attribution;
    /// `output_summary` is the one field both the zero-match reader and the
    /// attribution predicate look at, so on the live path they share this text.
    const ATTRIBUTED_ZERO_MATCH: &str = "running 0 tests\ntest result: ok. 0 passed; 0 failed; 412 filtered out\nthe declared filter is one path segment short of where the module is mounted; the module itself is exercised by the sibling command below";
    const BARE_ZERO_MATCH: &str =
        "running 0 tests\ntest result: ok. 0 passed; 0 failed; 412 filtered out";
    const MATCHED: &str = "running 3 tests\ntest result: ok. 3 passed; 0 failed";

    fn named(
        command: &str,
        status: WorkflowV2CommandStatus,
        pre_existing: bool,
        output_summary: &str,
    ) -> WorkflowV2CommandRecord {
        WorkflowV2CommandRecord {
            kind: WorkflowV2CommandKind::Test,
            command: command.to_string(),
            status,
            exit_code: Some(if status == WorkflowV2CommandStatus::Failed {
                1
            } else {
                0
            }),
            output_summary: output_summary.to_string(),
            pre_existing,
        }
    }

    fn accepted(commands: Vec<WorkflowV2CommandRecord>) -> WorkflowV2BranchOutcome {
        let mut result = WorkflowV2Result::accepted("focused verification passed");
        result.commands_run = commands;
        WorkflowV2BranchOutcome {
            item_id: "verify-task".to_string(),
            role: "verifier".to_string(),
            status: WorkflowV2Status::Accepted,
            result: Some(result),
            error: None,
            failure_kind: None,
            item_input_hash: None,
            completion_evidence: Vec::new(),
        }
    }

    /// The live shape: one declared filter was stale and matched nothing, the
    /// other matched tests and passed. The verdict stands and the stale
    /// declaration is surfaced under its own gap id, never the demotion's.
    #[test]
    fn an_attributed_zero_match_beside_a_passing_match_keeps_the_verdict() {
        let mut outcome = accepted(vec![
            named(
                "focused check two",
                WorkflowV2CommandStatus::Succeeded,
                false,
                MATCHED,
            ),
            named(
                "focused check one",
                WorkflowV2CommandStatus::Failed,
                true,
                ATTRIBUTED_ZERO_MATCH,
            ),
        ]);
        let result = normalized(&mut outcome);

        assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
        let ids = gap_ids(result);
        assert!(ids.contains(&"zero_test_match_superseded"), "{ids:?}");
        assert!(!ids.contains(&"zero_test_match_verification"), "{ids:?}");
        let gap = result
            .residual_gaps
            .iter()
            .find(|gap| gap.id == "zero_test_match_superseded")
            .expect("supersession gap");
        assert_eq!(gap.severity.as_deref(), Some("review"));
        assert!(
            gap.description.contains("focused check one"),
            "{}",
            gap.description
        );
        assert_eq!(
            result.data["zero_test_match_superseded_commands"],
            serde_json::json!(["focused check one"])
        );
        assert!(result.data.get("zero_test_match").is_none(), "{result:#?}");

        assert_eq!(outcome.status, WorkflowV2Status::Accepted);
        assert!(outcome.failure_kind.is_none());
    }

    /// Fail-closed conjunct one: nothing else matched, so an attribution on
    /// every zero-match command rescues nothing. Both commands here are
    /// attributed and both matched zero tests.
    #[test]
    fn every_test_command_matching_zero_tests_still_demotes() {
        let mut outcome = accepted(vec![
            named(
                "focused check one",
                WorkflowV2CommandStatus::Succeeded,
                true,
                ATTRIBUTED_ZERO_MATCH,
            ),
            named(
                "focused check two",
                WorkflowV2CommandStatus::Failed,
                true,
                ATTRIBUTED_ZERO_MATCH,
            ),
        ]);
        let result = normalized(&mut outcome);

        assert_eq!(result.status, WorkflowV2Status::NeedsReview, "{result:#?}");
        let ids = gap_ids(result);
        assert!(ids.contains(&"zero_test_match_verification"), "{ids:?}");
        assert!(!ids.contains(&"zero_test_match_superseded"), "{ids:?}");
        assert_eq!(result.data["zero_test_match"], serde_json::json!(true));

        assert_eq!(outcome.status, WorkflowV2Status::NeedsReview);
        assert_eq!(
            outcome.failure_kind,
            Some(crate::v2::BranchFailureKind::Semantic)
        );
    }

    /// Fail-closed conjunct two: a passing sibling is present, but the
    /// zero-match command carries no evidenced attribution, so it demotes
    /// exactly as before.
    #[test]
    fn an_unattributed_zero_match_still_demotes_beside_a_passing_match() {
        let mut outcome = accepted(vec![
            named(
                "focused check two",
                WorkflowV2CommandStatus::Succeeded,
                false,
                MATCHED,
            ),
            named(
                "focused check one",
                WorkflowV2CommandStatus::Failed,
                false,
                BARE_ZERO_MATCH,
            ),
        ]);
        let result = normalized(&mut outcome);

        assert_eq!(result.status, WorkflowV2Status::NeedsReview, "{result:#?}");
        let ids = gap_ids(result);
        assert!(ids.contains(&"zero_test_match_verification"), "{ids:?}");
        assert!(!ids.contains(&"zero_test_match_superseded"), "{ids:?}");
        assert_eq!(result.data["zero_test_match"], serde_json::json!(true));
    }

    /// The host's own filler is not the verifier's evidence here either: a
    /// `pre_existing` flag on a summary the envelope normaliser synthesized is
    /// not an attribution, so the zero match demotes beside a passing sibling.
    #[test]
    fn a_synthesized_summary_is_not_an_attribution_and_still_demotes() {
        let synthesized = format!(
            "{}; command status: failed)\n{BARE_ZERO_MATCH}",
            crate::v2::agent_output_normalize::SYNTHESIZED_OUTPUT_SUMMARY_PREFIX
        );
        let mut outcome = accepted(vec![
            named(
                "focused check two",
                WorkflowV2CommandStatus::Succeeded,
                false,
                MATCHED,
            ),
            named(
                "focused check one",
                WorkflowV2CommandStatus::Failed,
                true,
                &synthesized,
            ),
        ]);
        let result = normalized(&mut outcome);

        assert_eq!(result.status, WorkflowV2Status::NeedsReview, "{result:#?}");
        let ids = gap_ids(result);
        assert!(ids.contains(&"zero_test_match_verification"), "{ids:?}");
        assert!(!ids.contains(&"zero_test_match_superseded"), "{ids:?}");
    }
}
