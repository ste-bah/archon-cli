//! Real selected guardian/check/transport path, with a scaled enclosing clock.
use super::*;
use crate::command::acceptance_scratch_guardian::{launch::TEST_WINDOW, launch_selected};

async fn active_then_silent(text: &str) {
    let fixture = frozen_fixture(vec![criterion("AC-X-001", command(text.into()))]);
    let (repo, head) = source_repository();
    let scratch = tempfile::tempdir().unwrap();
    let mut policy = policy(
        repo.path(),
        fixture.project.path(),
        &fixture.task_root,
        scratch.path(),
        "/usr/bin:/bin",
        vec![],
    );
    policy.timeout_secs = 5;
    // Apply the same injected enclosing window to both implementations:
    // old = total, new = inactivity. Real subprocesses keep their real clock.
    TEST_WINDOW.with(|window| window.set(Some(6)));
    let result = launch_selected(
        request(
            &fixture,
            policy.clone(),
            head.clone(),
            scratch.path().join("active"),
        ),
        Some(["AC-X-001".into()].into()),
    )
    .await;
    TEST_WINDOW.with(|window| window.set(None));
    let result = result.expect("an active selected check must outlive the enclosing window");
    assert_eq!(result.checks[0].exit_code, Some(0));
    assert!(
        result.checks[0].operational_error.is_none(),
        "{:?}",
        result.checks
    );
    assert!(result.teardown_verified);

    // A real silent check must save an operational result, never a verdict.
    let silent = frozen_fixture(vec![criterion("AC-X-001", command("sleep 30".into()))]);
    policy.project = silent.project.path().into();
    policy.task_root = silent.task_root.clone();
    let result = launch_selected(
        request(&silent, policy, head, scratch.path().join("silent")),
        Some(["AC-X-001".into()].into()),
    )
    .await
    .unwrap();
    assert_eq!(
        result.checks[0].operational_error.as_deref(),
        Some(archon_workflow::acceptance_scratch::CHECK_TIMED_OUT)
    );
    assert!(!result.passed());
    assert!(result.teardown_verified);
    assert!(scratch.path().join("silent/observation.json").is_file());
}

#[tokio::test]
async fn issue356_guardian_selected_stdout_survives() {
    active_then_silent("for i in $(seq 1 40); do printf x; sleep 0.2; done").await;
}
#[tokio::test]
async fn issue356_guardian_selected_stderr_survives() {
    active_then_silent("for i in $(seq 1 40); do printf x >&2; sleep 0.2; done").await;
}
#[tokio::test]
async fn issue356_guardian_selected_descendant_activity_survives() {
    active_then_silent("perl -MTime::HiRes=time -e '$end=time()+8.5; while(time()<$end) {$n++}'")
        .await;
}

async fn enclosing_stall(text: &str) {
    let fixture = frozen_fixture(vec![criterion("AC-X-001", command(text.into()))]);
    let (repo, head) = source_repository();
    let scratch = tempfile::tempdir().unwrap();
    let mut policy = policy(
        repo.path(),
        fixture.project.path(),
        &fixture.task_root,
        scratch.path(),
        "/usr/bin:/bin",
        vec![],
    );
    policy.timeout_secs = 15;
    TEST_WINDOW.with(|window| window.set(Some(6)));
    let result = launch_selected(
        request(&fixture, policy, head, scratch.path().join("stalled")),
        Some(["AC-X-001".into()].into()),
    )
    .await;
    TEST_WINDOW.with(|window| window.set(None));
    assert!(
        matches!(
            result,
            Err(archon_workflow::WorkflowError::ControlPaused(_))
        ),
        "a real enclosing stall must pause: {result:?}"
    );
    let evidence: serde_json::Value = serde_json::from_slice(
        &std::fs::read(scratch.path().join("stalled/observation.json")).unwrap(),
    )
    .unwrap();
    assert!(evidence["checks"][0]["operational_error"].is_string());
    assert_eq!(evidence["teardown_verified"], true);
}
#[tokio::test]
async fn issue356_guardian_selected_silence_pauses() {
    enclosing_stall("sleep 30").await;
}
#[tokio::test]
async fn issue356_guardian_selected_output_then_silence_pauses() {
    enclosing_stall("for i in 1 2 3 4 5 6 7 8; do printf x; sleep 0.2; done; sleep 30").await;
}
#[tokio::test]
async fn issue356_guardian_selected_cpu_then_silence_pauses() {
    enclosing_stall(
        "perl -MTime::HiRes=time -e '$end=time()+1.6; while(time()<$end) {$n++}'; sleep 30",
    )
    .await;
}

async fn short_batch(text: &str) {
    let ids: std::collections::BTreeSet<String> =
        (1..=20).map(|n| format!("AC-X-{n:03}")).collect();
    let fixture = frozen_fixture(
        ids.iter()
            .map(|id| criterion(id, command(text.into())))
            .collect(),
    );
    let (repo, head) = source_repository();
    let scratch = tempfile::tempdir().unwrap();
    let mut policy = policy(
        repo.path(),
        fixture.project.path(),
        &fixture.task_root,
        scratch.path(),
        "/usr/bin:/bin",
        vec![],
    );
    policy.timeout_secs = 5;
    TEST_WINDOW.with(|window| window.set(Some(6)));
    let result = launch_selected(
        request(&fixture, policy, head, scratch.path().join("batch")),
        Some(ids),
    )
    .await;
    TEST_WINDOW.with(|window| window.set(None));
    let result = result.expect("short selected checks must keep a progressing batch alive");
    assert_eq!(result.checks.len(), 20);
    assert!(result.passed(), "{:?}", result.checks);
    assert!(result.teardown_verified);
}
#[tokio::test]
async fn issue356_guardian_selected_short_output_batch_survives() {
    short_batch("printf x; sleep 0.4").await;
}
#[tokio::test]
async fn issue356_guardian_selected_short_silent_batch_survives() {
    short_batch("sleep 0.4").await;
}
#[tokio::test]
async fn issue356_guardian_selected_short_cpu_batch_survives() {
    short_batch("perl -MTime::HiRes=time -e '$end=time()+0.4; while(time()<$end) {$n++}'").await;
}
