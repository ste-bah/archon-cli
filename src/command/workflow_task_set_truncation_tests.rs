//! Issue 260: a truncated judge reply at the freeze level.

use super::*;

/// Issue 260: a truncated reply is continued, never parsed as it stands; one
/// whose continuation adds nothing new is incomplete, and a staged freeze
/// reports that as resumable (the executor retries, then pauses), never as
/// an operational failure that ends the run.
#[tokio::test]
async fn a_judge_reply_that_stays_truncated_is_incomplete_never_parsed() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, original) = seed(&temp);
    let judge = || {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let client = Arc::new(FinishReasonJudge {
            calls: calls.clone(),
            content: "{ definitely incomplete".into(),
            stop_reason: Some("max_tokens".into()),
        });
        (calls, client)
    };
    let (calls, client) = judge();
    let error = prepare_acceptance_freeze(
        temp.path(),
        &tasks,
        &prd,
        archon_core::config::GateMode::Observe,
        client,
    )
    .await
    .unwrap_err();
    // The reply, then one continuation that only repeated it.
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert!(
        judge::JudgeIncomplete::caused(&error).is_some(),
        "{error:#}"
    );
    assert!(
        !format!("{error:#}").contains("malformed batched JSON"),
        "a truncated reply is never parsed: {error:#}"
    );
    assert_eq!(
        std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap(),
        original
    );
    assert!(!tasks.join(ACCEPTANCE_LOCK_FILE).exists());

    let (_, client) = judge();
    let resume = crate::command::workflow_freeze_budget::FreezeResume::saving(
        crate::command::workflow_freeze_budget::FreezeBudget::unlimited(),
        true,
    );
    let error = prepare_acceptance_freeze_resumable(
        temp.path(),
        &tasks,
        &prd,
        archon_core::config::GateMode::Observe,
        original.clone(),
        client,
        &resume,
    )
    .await
    .unwrap_err();
    let incomplete = crate::command::workflow_freeze_budget::FreezeIncomplete::caused(&error)
        .expect("a staged freeze whose judge cannot complete is resumable");
    assert!(
        incomplete.report().contains("judge"),
        "{}",
        incomplete.report()
    );
    assert!(
        incomplete.report().ends_with(&resume.progress.line()),
        "the progress line is last: {}",
        incomplete.report()
    );
}
