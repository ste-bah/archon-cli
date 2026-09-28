//! Batch H through the host (`WorkflowScriptHost::execute`): a recorded
//! remediation answer replays only for the question it answered. An
//! acceptance round's fix names the round (`observedBy`); the round,
//! recorded again after the answer, is an observation the answer never saw,
//! so the call runs -- under its own id or a shifted ordinal. And the
//! completed-task waiver never stretches a record to other findings.

use super::workflow_live_v2_reuse_content_key_tests::reuse_test_store;
use super::*;

fn runner_with(
    llm: Arc<dyn WorkflowLlmClient>,
    workflow_store: &WorkflowStore,
    run: &archon_workflow::WorkflowRun,
    v2_store: &WorkflowV2ResultStore,
    args: serde_json::Value,
) -> WorkflowV2ScriptRunner {
    let (ui_sink, tui_rx) = default_workflow_ui_sink();
    std::mem::forget(tui_rx);
    let client = LiveV2AgentClient::new(llm, ui_sink, Vec::new(), run.id.clone(), None, None);
    WorkflowV2ScriptRunner::new(
        "observed remediation".to_string(),
        test_runtime(&test_spec()),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store.clone(),
        run.id.clone(),
        true,
        None,
        Some(args),
    )
    .with_frontier_resume(true)
}

/// An acceptance round's fix, named as the prelude names it.
const FIX_SCRIPT: &str = r#"
async function workflow(w) {
  const id = "review-remediate-task-a-1-" + args.ordinal;
  await w.agent(id, {
    task: "Post-review remediation for TASK-A. Findings (verbatim): " + args.findings,
    remediationContract: { version: 1, stage: "remediate", taskId: "TASK-A", round: 1, maxRounds: 1,
      sourceReduceCallIds: ["r"], observedBy: ["acceptance-contract-run-1"] },
  });
  return "done";
}
"#;

fn args(ordinal: u64, findings: &str) -> serde_json::Value {
    serde_json::json!({ "ordinal": ordinal, "findings": findings })
}

fn accepted() -> Arc<dyn WorkflowLlmClient> {
    Arc::new(SlowAcceptedLlm {
        delay: Duration::ZERO,
    })
}

/// The host runs and records the acceptance round now.
fn observe(run: &archon_workflow::WorkflowRun, v2_store: &WorkflowV2ResultStore) {
    let call = WorkflowV2HostCall {
        id: "acceptance-contract-run-1".into(),
        method: WorkflowV2HostMethod::Tool,
        write_mode: None,
        options: WorkflowV2HostOptions::default(),
    };
    let record = WorkflowV2CallRecord::new(
        run.id.clone(),
        call,
        1,
        "round".into(),
        WorkflowV2Result::accepted("round observed"),
        vec![],
    );
    v2_store
        .save_call_record(&record)
        .expect("acceptance record");
}

/// A session over the same run directory: a resume.
async fn session(
    llm: Arc<dyn WorkflowLlmClient>,
    workflow_store: &WorkflowStore,
    run: &archon_workflow::WorkflowRun,
    root: &std::path::Path,
    args: serde_json::Value,
) -> WorkflowV2ScriptSummary {
    let store = WorkflowV2ResultStore::new(root.to_path_buf());
    runner_with(llm, workflow_store, run, &store, args)
        .run(FIX_SCRIPT)
        .await
        .expect("session")
}

#[tokio::test]
async fn an_acceptance_fix_observed_again_after_it_answered_runs_again() {
    for (recorded, asked) in [(81, 81), (83, 81)] {
        let temp = tempfile::tempdir().expect("tempdir");
        let (workflow_store, run) = reuse_test_store(&temp);
        let root = workflow_store.run_dir(&run.id).join("v2");
        let v2_store = WorkflowV2ResultStore::new(root.clone());
        observe(&run, &v2_store);
        let first = session(
            accepted(),
            &workflow_store,
            &run,
            &root,
            args(recorded, "[f1]"),
        )
        .await;
        assert_eq!(first.executed, 1, "{first:?}");
        // Not observed since it answered: the same question, replayed.
        let replayed = session(
            Arc::new(PanicLlm),
            &workflow_store,
            &run,
            &root,
            args(asked, "[f1]"),
        )
        .await;
        assert_eq!(replayed.reused, 1, "-{recorded} for -{asked}: {replayed:?}");
        // The round ran again and saw the same failure: a new question.
        observe(&run, &WorkflowV2ResultStore::new(root.clone()));
        let again = session(
            accepted(),
            &workflow_store,
            &run,
            &root,
            args(asked, "[f1]"),
        )
        .await;
        assert_eq!(again.executed, 1, "-{recorded} for -{asked}: {again:?}");
        assert_eq!(again.reused, 0);
    }
}

/// The completed-task waiver skips the input hash on purpose (a re-authored
/// script); for remediation it never covers other findings.
#[tokio::test]
async fn the_completed_task_waiver_never_answers_other_findings() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (workflow_store, run) = reuse_test_store(&temp);
    let root = workflow_store.run_dir(&run.id).join("v2");
    let v2_store = WorkflowV2ResultStore::new(root.clone());
    observe(&run, &v2_store);
    session(accepted(), &workflow_store, &run, &root, args(81, "[f1]")).await;
    let record = v2_store
        .load_call_record("review-remediate-task-a-1-81")
        .unwrap()
        .expect("fix record");
    let credited = record.with_completion_evidence(vec![WorkflowV2TaskCompletionEvidence::new(
        "TASK-A",
        archon_workflow::WorkflowV2TaskCompletionEvidenceKind::ImplementationCandidate,
        "review-remediate-task-a-1-81",
        "item-0",
        WorkflowV2Status::Accepted,
    )]);
    v2_store
        .save_call_record(&credited)
        .expect("credited record");
    let completed = std::collections::BTreeSet::from(["TASK-A".to_string()]);
    let store = WorkflowV2ResultStore::new(root.clone());
    let other = runner_with(accepted(), &workflow_store, &run, &store, args(81, "[f2]"))
        .with_resume_completed_ids(completed.clone())
        .run(FIX_SCRIPT)
        .await
        .expect("session");
    assert_eq!(other.executed, 1, "{other:?}");
    assert_eq!(other.reused, 0);
}
