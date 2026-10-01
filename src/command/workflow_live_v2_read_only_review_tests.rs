//! REM-5: the review map's re-run loop, driven by a scripted pass.

use std::cell::RefCell;
use std::collections::VecDeque;

use archon_workflow::{
    WorkflowError, WorkflowV2BranchOutcome, WorkflowV2FanoutItem, WorkflowV2FanoutReport,
    WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2Result,
    WorkflowV2Status,
};

use super::run_review_passes;

fn item(id: &str) -> WorkflowV2FanoutItem {
    let call = WorkflowV2HostCall {
        id: "adversarial-review-map".to_string(),
        method: WorkflowV2HostMethod::Parallel,
        write_mode: None,
        options: WorkflowV2HostOptions::default(),
    };
    WorkflowV2FanoutItem::read_only(id, "critic", call, serde_json::json!({ "item": id }))
}

/// A reviewed branch (a verdict, no findings) or one with no result.
fn outcome(id: &str, reviewed: bool) -> WorkflowV2BranchOutcome {
    WorkflowV2BranchOutcome {
        item_id: id.to_string(),
        role: "critic".to_string(),
        status: if reviewed {
            WorkflowV2Status::Accepted
        } else {
            WorkflowV2Status::Failed
        },
        result: reviewed.then(|| WorkflowV2Result::accepted("reviewed")),
        error: (!reviewed).then(|| "agent transport failed: timed out after 60s".to_string()),
        failure_kind: None,
        item_input_hash: None,
        completion_evidence: Vec::new(),
    }
}

fn report(outcomes: Vec<WorkflowV2BranchOutcome>) -> WorkflowV2FanoutReport {
    WorkflowV2FanoutReport {
        outcomes,
        max_parallelism: 4,
        peak_parallelism: 1,
        cancelled: false,
    }
}

/// Each pass answers each branch it is handed from that branch's script:
/// `true` reviews it, `false` leaves it without a verdict.
struct Scripted {
    answers: RefCell<Vec<(&'static str, VecDeque<bool>)>>,
    passes: RefCell<Vec<Vec<String>>>,
}

impl Scripted {
    fn new(answers: Vec<(&'static str, Vec<bool>)>) -> Self {
        Self {
            answers: RefCell::new(
                answers
                    .into_iter()
                    .map(|(id, list)| (id, list.into()))
                    .collect(),
            ),
            passes: RefCell::new(Vec::new()),
        }
    }

    fn pass(&self, items: Vec<WorkflowV2FanoutItem>) -> WorkflowV2FanoutReport {
        self.passes
            .borrow_mut()
            .push(items.iter().map(|item| item.id.clone()).collect());
        let mut answers = self.answers.borrow_mut();
        report(
            items
                .iter()
                .map(|item| {
                    let reviewed = answers
                        .iter_mut()
                        .find(|(id, _)| *id == item.id)
                        .and_then(|(_, list)| list.pop_front())
                        .unwrap_or(false);
                    outcome(&item.id, reviewed)
                })
                .collect(),
        )
    }
}

async fn drive(review_map: bool, script: &Scripted) -> (WorkflowV2FanoutReport, Vec<Vec<String>>) {
    let items = vec![item("a"), item("b"), item("c")];
    let report = run_review_passes(
        items,
        review_map,
        |items| std::future::ready(Ok(script.pass(items))),
        |_| Ok(()),
    )
    .await
    .expect("passes");
    (report, script.passes.borrow().clone())
}

fn reviewed(report: &WorkflowV2FanoutReport, id: &str) -> bool {
    report
        .outcomes
        .iter()
        .find(|outcome| outcome.item_id == id)
        .is_some_and(|outcome| outcome.result.is_some())
}

#[tokio::test]
async fn an_incomplete_branch_is_re_run_until_it_reviews_its_task() {
    // `b` fails twice, then reviews; `c` fails once, then reviews.
    let script = Scripted::new(vec![
        ("a", vec![true]),
        ("b", vec![false, false, true]),
        ("c", vec![false, true]),
    ]);
    let (report, passes) = drive(true, &script).await;
    assert_eq!(
        passes,
        vec![
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
            vec!["b".to_string(), "c".to_string()],
            vec!["b".to_string()],
        ],
        "only the incomplete branches are re-run, pass after pass"
    );
    assert!(["a", "b", "c"].iter().all(|id| reviewed(&report, id)));
    assert_eq!(
        report.outcomes.len(),
        3,
        "each branch once, its last outcome"
    );
}

#[tokio::test]
async fn a_pass_that_completes_nothing_ends_the_loop_and_the_branch_stays_unreviewed() {
    let script = Scripted::new(vec![
        ("a", vec![true]),
        ("b", vec![false, false, false, true]),
        ("c", vec![true]),
    ]);
    let (report, passes) = drive(true, &script).await;
    // Pass 2 re-ran `b` and completed nothing: a plateau, no pass 3.
    assert_eq!(passes.len(), 2, "{passes:?}");
    assert!(
        !reviewed(&report, "b"),
        "left incomplete, recorded unreviewed"
    );
    assert!(reviewed(&report, "a") && reviewed(&report, "c"));
}

#[tokio::test]
async fn any_other_fanout_runs_one_pass() {
    let script = Scripted::new(vec![("a", vec![true]), ("b", vec![false, true])]);
    let (report, passes) = drive(false, &script).await;
    assert_eq!(passes.len(), 1);
    assert!(!reviewed(&report, "b"));
}

#[tokio::test]
async fn a_pause_before_a_re_run_stops_the_map_as_itself() {
    let script = Scripted::new(vec![("a", vec![false, true])]);
    let error = run_review_passes(
        vec![item("a")],
        true,
        |items| std::future::ready(Ok(script.pass(items))),
        |_| Err(WorkflowError::ControlPaused("paused".to_string())),
    )
    .await
    .expect_err("the poll's error ends the loop");
    assert!(matches!(error, WorkflowError::ControlPaused(_)));
    assert_eq!(script.passes.borrow().len(), 1, "no re-run once paused");
}

/// Major 2: a review branch gets a shell only where the host can bound it.
#[test]
fn a_shell_needs_the_os_boundary_and_a_drawn_scope() {
    use super::shell_allowed;
    assert!(shell_allowed(true, true));
    assert!(
        !shell_allowed(false, true),
        "no OS boundary on this platform"
    );
    assert!(!shell_allowed(true, false), "no boundary scope drawn");
}

fn review_request(input: serde_json::Value) -> archon_workflow::StageRunRequest {
    let mut input = input;
    input["v2_call"] = serde_json::json!({ "id": "adversarial-review-map-0", "method": "agent",
        "role": "critic", "write_mode": null, "target_files": [] });
    archon_workflow::StageRunRequest {
        run_id: "run".into(),
        stage_id: "adversarial-review-map-0".into(),
        stage_kind: archon_workflow::StageKind::Agent,
        agent: Some("critic".into()),
        task: "review".into(),
        attempt: 1,
        provider_tier: archon_workflow::ProviderTier::Critic,
        depends_on: Vec::new(),
        input,
    }
}

fn granted(shell: bool) -> archon_workflow::StageRunRequest {
    use archon_workflow::v2::review_findings::commands::grant_review_commands;
    let mut items = vec![item("adversarial-review-map-0")];
    items[0].input["required_tools"] = serde_json::json!(["memory_store"]);
    grant_review_commands(&mut items, shell);
    review_request(items.remove(0).input)
}

/// Major 2: withheld, the branch has no shell and is told so.
#[test]
fn a_withheld_grant_leaves_no_shell_and_says_so() {
    use archon_workflow::stage_command_policy::command_execution_stage;
    use archon_workflow::v2::review_findings::commands::REVIEW_NO_SHELL_RULE;
    let request = granted(false);
    assert!(!command_execution_stage(&request));
    assert_eq!(
        request.input["review_execution"],
        serde_json::json!(REVIEW_NO_SHELL_RULE)
    );
    let tools = super::super::super::super::workflow_live_runner::allowed_tools(&request);
    assert!(!tools.iter().any(|tool| tool == "Bash"), "{tools:?}");
}

/// Minor 2: a granted review branch gets read-only tools plus Bash, never
/// full access, so no declared native tool widens it.
#[test]
fn a_granted_review_branch_is_read_only_plus_bash() {
    use super::super::super::super::workflow_live_runner::{allowed_tools, full_tool_access};
    let request = granted(true);
    assert!(!full_tool_access(&request));
    let tools = allowed_tools(&request);
    assert!(tools.iter().any(|tool| tool == "Bash"), "{tools:?}");
    for widening in ["memory_store", "Write", "Edit"] {
        assert!(
            !tools.iter().any(|tool| tool == widening),
            "{widening}: {tools:?}"
        );
    }
}
