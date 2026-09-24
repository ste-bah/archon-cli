use std::collections::VecDeque;
use std::sync::Mutex;

use archon_workflow::v2::WorkflowV2Status;

use super::*;

const WALL: &str = "agent transport failed: workflow stage failed: subagent timed out after 14400s";

fn inactivity() -> WorkflowError {
    WorkflowError::HostCallTimeout(format!(
        "agent transport failed: {} no model output, tool call or tool result for 1800s",
        archon_workflow::error::INACTIVITY_TIMEOUT_MARKER
    ))
}

fn accepted() -> WorkflowV2Result {
    WorkflowV2Result {
        status: WorkflowV2Status::Accepted,
        summary: "reviewed".into(),
        ..WorkflowV2Result::default()
    }
}

/// Drive the retry loop over scripted attempt outcomes; report what it
/// returned, how many attempts it made, and each re-ask it announced.
async fn drive(
    review_map: bool,
    script: Vec<WorkflowResult<WorkflowV2Result>>,
) -> (
    WorkflowResult<WorkflowV2Result>,
    usize,
    Vec<(HostReask, String)>,
) {
    let script = Mutex::new(VecDeque::from(script));
    let attempts = Mutex::new(0usize);
    let reasks = Mutex::new(Vec::new());
    let on_reask = |reason: HostReask, first: &str| {
        reasks.lock().unwrap().push((reason, first.to_string()));
    };
    let result = with_host_retry(review_map, &on_reask, || {
        *attempts.lock().unwrap() += 1;
        let next = script
            .lock()
            .unwrap()
            .pop_front()
            .expect("the loop asked for more attempts than were scripted");
        async move { next }
    })
    .await;
    let attempts = *attempts.lock().unwrap();
    (result, attempts, reasks.into_inner().unwrap())
}

/// A failed review map branch is re-asked once, the re-ask is announced, and
/// the review that then completes says it was re-asked.
#[tokio::test]
async fn a_failed_review_map_branch_is_reasked_once() {
    let (result, attempts, reasks) = drive(
        true,
        vec![
            Err(WorkflowError::HostCallTimeout(WALL.into())),
            Ok(accepted()),
        ],
    )
    .await;
    assert_eq!(attempts, 2);
    assert_eq!(reasks.len(), 1);
    assert_eq!(reasks[0].0, HostReask::ReviewIncomplete);
    assert!(reasks[0].1.contains("subagent timed out after 14400s"));
    // The second attempt is judged on its own: nothing is added to it, so a
    // review with no evidence of its own still fails validation.
    let result = result.expect("the second attempt reviewed the task");
    assert_eq!(result, accepted());
}

/// A review map branch that fails twice stops there — one re-ask, never more —
/// and its failure keeps its type and says it was re-asked. (The unreviewed
/// finding its task then gets is the host's attachment; see
/// `review_unreviewed`.)
#[tokio::test]
async fn a_review_map_branch_failing_twice_stops_after_one_reask() {
    let (result, attempts, reasks) = drive(
        true,
        vec![
            Err(WorkflowError::HostCallTimeout(WALL.into())),
            Err(inactivity()),
        ],
    )
    .await;
    assert_eq!(attempts, 2);
    assert_eq!(reasks.len(), 1);
    let error = result.expect_err("two failures are a failed branch");
    assert!(error.is_host_call_timeout(), "{error}");
    let text = error.to_string();
    assert!(text.contains("host re-ask spent"), "{text}");
    assert!(archon_workflow::error::is_inactivity_timeout_text(&text));
    // Only the failure that ended the branch is in its text.
    assert!(!text.contains("timed out after"), "{text}");
}

/// The first attempt's error never leaks into the final one: a re-ask that
/// ends in a content rejection classifies as that rejection, not as the
/// host cut that preceded it.
#[tokio::test]
async fn the_final_error_is_the_second_attempts_own() {
    let (result, attempts, reasks) = drive(
        true,
        vec![
            Err(inactivity()),
            Err(WorkflowError::StageFailed(
                "agent result failed validation: missing findings".into(),
            )),
        ],
    )
    .await;
    assert_eq!(attempts, 2);
    assert_eq!(reasks.len(), 1);
    let text = result.expect_err("failed twice").to_string();
    assert!(text.contains("agent result failed validation"), "{text}");
    for leaked in [
        archon_workflow::error::INACTIVITY_TIMEOUT_MARKER,
        archon_workflow::error::HOST_CALL_TIMEOUT_MARKER,
        "agent transport failed",
        "timed out",
        "cancelled",
    ] {
        assert!(!text.contains(leaked), "{leaked} leaked into: {text}");
    }
    assert!(!transport_retry::is_transport_failure(&text), "{text}");
}

/// An inactivity cut on any read-only branch is re-asked once, and only once.
#[tokio::test]
async fn an_inactivity_cut_is_reasked_once_on_any_read_only_branch() {
    let (result, attempts, reasks) = drive(false, vec![Err(inactivity()), Ok(accepted())]).await;
    assert!(result.is_ok());
    assert_eq!(attempts, 2);
    assert_eq!(reasks[0].0, HostReask::Inactivity);

    let (result, attempts, _) = drive(false, vec![Err(inactivity()), Err(inactivity())]).await;
    assert!(result.is_err());
    assert_eq!(attempts, 2);
}

/// Nothing else changes: a wall-clock cut on an ordinary read-only branch is
/// still not re-asked, and a control signal never is.
#[tokio::test]
async fn existing_budgets_are_unchanged() {
    let (result, attempts, reasks) = drive(
        false,
        vec![Err(WorkflowError::HostCallTimeout(WALL.into()))],
    )
    .await;
    assert!(result.is_err());
    assert_eq!(attempts, 1);
    assert!(reasks.is_empty());

    for review_map in [false, true] {
        let (result, attempts, _) = drive(
            review_map,
            vec![Err(WorkflowError::ControlPaused("pause".into()))],
        )
        .await;
        assert!(matches!(result, Err(WorkflowError::ControlPaused(_))));
        assert_eq!(attempts, 1);
    }

    // A transport drop keeps its own budget first; only once that is spent
    // does a review map branch get its one re-ask.
    let drops = || -> Vec<WorkflowResult<WorkflowV2Result>> {
        (0..=transport_retry::MAX_TRANSPORT_RETRIES)
            .map(|_| {
                Err(WorkflowError::StageFailed(
                    "agent transport failed: connection reset".into(),
                ))
            })
            .collect()
    };
    let (result, attempts, _) = drive(false, drops()).await;
    assert!(result.is_err());
    assert_eq!(attempts, transport_retry::MAX_TRANSPORT_RETRIES + 1);
    let mut script = drops();
    script.push(Ok(accepted()));
    let (result, attempts, reasks) = drive(true, script).await;
    assert!(result.is_ok());
    assert_eq!(attempts, transport_retry::MAX_TRANSPORT_RETRIES + 2);
    assert_eq!(reasks.len(), 1);
}

/// The branch event names the bound that fired: an inactivity cut is never
/// labelled as the wall clock, though both travel as host cuts.
#[test]
fn the_branch_event_tells_inactivity_from_the_wall_clock() {
    let outcome = |error: String| archon_workflow::v2::WorkflowV2BranchOutcome {
        item_id: "b".into(),
        role: "critic".into(),
        status: WorkflowV2Status::Failed,
        result: None,
        error: Some(error),
        failure_kind: None,
        item_input_hash: None,
        completion_evidence: Vec::new(),
    };
    let wall = WorkflowError::HostCallTimeout(WALL.into()).to_string();
    let idle = inactivity().to_string();
    assert_eq!(
        super::super::branch_event_label(&outcome(wall)),
        "branch_timed_out"
    );
    assert_eq!(
        super::super::branch_event_label(&outcome(idle)),
        "branch_inactive"
    );
}
