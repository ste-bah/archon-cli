//! Issue 260: a truncated judge reply at the freeze level.

use super::*;

/// Issue 260: a truncated reply is continued, never parsed as it stands.
///
/// #552 replaces `a_judge_reply_that_stays_truncated_is_incomplete_never_parsed`:
/// a check whose replies stay unusable for the judge's no-progress window is
/// now its author's finding and the freeze completes, in a staged freeze too,
/// instead of a resumable incomplete freeze. The call count, never-parsed and
/// unchanged-contract assertions are kept.
#[tokio::test]
async fn a_judge_reply_that_stays_truncated_is_never_parsed_and_goes_to_its_author_552() {
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
    let prepared = prepare_acceptance_freeze(
        temp.path(),
        &tasks,
        &prd,
        archon_core::config::GateMode::Observe,
        client,
    )
    .await
    .expect("no host stop and no pause: the check goes back to its author");
    // Not a JSON document even as a prefix: nothing to continue, so the
    // judge is asked afresh up to its no-progress bound, then the check
    // goes back to its author.
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 4);
    let texts = (prepared.findings.iter())
        .map(|finding| finding.text.as_str())
        .collect::<Vec<_>>();
    let undecided = (prepared.findings.iter())
        .find(|finding| {
            finding
                .text
                .starts_with("the judge returned no usable verdict for check")
        })
        .unwrap_or_else(|| panic!("the author receives a finding: {texts:?}"));
    assert_eq!(
        undecided.remediation_scope,
        archon_workflow::RemediationScope::CandidateArtifact
    );
    assert!(
        !texts
            .iter()
            .any(|text| text.contains("malformed batched JSON")),
        "a truncated reply is never parsed: {texts:?}"
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
    let prepared = prepare_acceptance_freeze_resumable(
        temp.path(),
        &tasks,
        &prd,
        archon_core::config::GateMode::Observe,
        original.clone(),
        client,
        &resume,
    )
    .await
    .expect("a staged freeze completes with the check's finding");
    assert!(
        (prepared.findings.iter()).any(|finding| finding
            .text
            .starts_with("the judge returned no usable verdict for check")),
        "the staged freeze gives the same finding"
    );
}

/// D2: a judge that refuses to judge the candidate is the judge's own
/// outcome about it. A staged freeze reports it as an authored judge reply,
/// which the staged command carries in its envelope back to the author loop
/// (`report_authored_judge_incomplete`), never as the host's resumable exit
/// that pauses the run.
#[tokio::test]
async fn a_judge_refusal_under_a_staged_freeze_is_an_authored_judge_reply() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, original) = seed(&temp);
    let client = Arc::new(FinishReasonJudge {
        calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        content: String::new(),
        stop_reason: Some("refusal".into()),
    });
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
        .expect("a judge that refuses is no failure of the run");
    assert!(
        incomplete.is_authored_judge_reply(),
        "a refusal is the judge's own reply, never the host's: {}",
        incomplete.report()
    );
    assert!(
        incomplete.report().contains("refused to judge"),
        "{}",
        incomplete.report()
    );
    assert_eq!(
        std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap(),
        original
    );
}
