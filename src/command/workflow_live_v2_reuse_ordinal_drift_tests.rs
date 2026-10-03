//! Issue 265: the ordinal-drift reuse of a completed task's record holds a
//! remediation call to the same freshness checks as every other reuse path.
//! It answers only the question the record answered, and never answers a
//! remediation call with an implement record.

use super::workflow_live_v2_reuse_content_key_tests::reuse_test_store;
use super::*;

const SCRIPT: &str = r#"
async function workflow(w) {
  const options = { task: "Post-review remediation for TASK-A-001. Findings (verbatim): " + args.findings };
  if (args.remediation) {
    options.remediationContract = { version: 1, stage: args.stage || "remediate", taskId: "TASK-A-001",
      round: 1, maxRounds: 1, sourceReduceCallIds: [args.source || "r"] };
  }
  await w.agent(args.label, options);
  return "done";
}
"#;

fn remediation(label: &str, findings: &str) -> serde_json::Value {
    serde_json::json!({ "label": label, "findings": findings, "remediation": true })
}

async fn session(
    llm: Arc<dyn WorkflowLlmClient>,
    workflow_store: &WorkflowStore,
    run: &archon_workflow::WorkflowRun,
    root: &std::path::Path,
    args: serde_json::Value,
    completed: &[&str],
) -> WorkflowV2ScriptSummary {
    let v2 = WorkflowV2ResultStore::new(root.to_path_buf());
    if let Some(id) = args.get("replayedFix").and_then(serde_json::Value::as_str) {
        let fix = v2.load_call_record(id).unwrap().unwrap();
        let key =
            archon_workflow::v2::script::resume_verdict::remediation_round_key(&fix.call).unwrap();
        v2.note_fix_lineage(
            &key,
            Some(archon_workflow::v2::result_store::ReplayedFix {
                call_id: fix.call.id,
                finished_at: fix.finished_at,
            }),
        );
    }
    // A changed scaffold prevents the strict content-keyed fallback from
    // masking a defect in the completed-task ordinal-drift waiver.
    let script = format!(
        "{SCRIPT}\n// revision {}",
        args.get("revision")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("initial")
    );
    let (ui_sink, tui_rx) = default_workflow_ui_sink();
    std::mem::forget(tui_rx);
    let client = LiveV2AgentClient::new(llm, ui_sink, Vec::new(), run.id.clone(), None, None);
    WorkflowV2ScriptRunner::new(
        "ordinal drift".to_string(),
        test_runtime(&test_spec()),
        WorkflowV2AgentAdapter::new(),
        client,
        v2,
        workflow_store.clone(),
        run.id.clone(),
        true,
        None,
        Some(args),
    )
    .with_frontier_resume(true)
    .with_resume_completed_ids(completed.iter().map(|id| id.to_string()).collect())
    .run(&script)
    .await
    .expect("session")
}

fn accepted() -> Arc<dyn WorkflowLlmClient> {
    Arc::new(SlowAcceptedLlm {
        delay: Duration::ZERO,
    })
}

#[tokio::test]
async fn a_shifted_remediation_call_with_other_findings_runs_again() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (workflow_store, run) = reuse_test_store(&temp);
    let root = workflow_store.run_dir(&run.id).join("v2");
    let first = session(
        accepted(),
        &workflow_store,
        &run,
        &root,
        remediation("remediate-task-a-001-r1-5", "[F1]"),
        &[],
    )
    .await;
    assert_eq!(first.executed, 1, "{first:?}");
    // The same question under a shifted ordinal is answered from the record.
    let same = session(
        Arc::new(PanicLlm),
        &workflow_store,
        &run,
        &root,
        remediation("remediate-task-a-001-r1-7", "[F1]"),
        &["TASK-A-001"],
    )
    .await;
    assert_eq!(same.reused, 1, "{same:?}");
    // Other findings are another question: the call runs.
    let other = session(
        accepted(),
        &workflow_store,
        &run,
        &root,
        remediation("remediate-task-a-001-r1-9", "[F2]"),
        &["TASK-A-001"],
    )
    .await;
    assert_eq!((other.executed, other.reused), (1, 0), "{other:?}");
}

#[tokio::test]
async fn an_implement_record_never_answers_a_remediation_call() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (workflow_store, run) = reuse_test_store(&temp);
    let root = workflow_store.run_dir(&run.id).join("v2");
    let implement = serde_json::json!({
        "label": "implement-task-a-001-3", "findings": "[F1]", "remediation": false,
    });
    let first = session(accepted(), &workflow_store, &run, &root, implement, &[]).await;
    assert_eq!(first.executed, 1, "{first:?}");
    let fix = session(
        accepted(),
        &workflow_store,
        &run,
        &root,
        remediation("remediate-task-a-001-r1-5", "[F1]"),
        &["TASK-A-001"],
    )
    .await;
    assert_eq!((fix.executed, fix.reused), (1, 0), "{fix:?}");
}

fn record_source(v2: &WorkflowV2ResultStore, at: &str) {
    let call = WorkflowV2HostCall {
        id: "recorded-review".into(),
        method: WorkflowV2HostMethod::Agent,
        write_mode: None,
        options: Default::default(),
    };
    let mut record = WorkflowV2CallRecord::new(
        "fixture",
        call,
        1,
        "input".into(),
        WorkflowV2Result::accepted("review findings"),
        vec![],
    );
    record.started_at = at.into();
    record.finished_at = at.into();
    v2.save_call_record(&record).unwrap();
}

fn drift_args(label: &str, stage: &str, revision: &str) -> serde_json::Value {
    serde_json::json!({ "label": label, "findings": "[F1]", "remediation": true,
        "stage": stage, "source": "recorded-review", "revision": revision })
}

#[tokio::test]
async fn ordinal_drift_rejects_an_answer_older_than_its_recorded_source() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = reuse_test_store(&temp);
    let root = store.run_dir(&run.id).join("v2");
    let v2 = WorkflowV2ResultStore::new(root.clone());
    record_source(&v2, "2026-01-01T00:00:00Z");
    let first = session(
        accepted(),
        &store,
        &run,
        &root,
        drift_args("remediate-task-a-001-r1-5", "remediate", "first"),
        &[],
    )
    .await;
    assert_eq!(first.executed, 1, "{first:?}");
    let same = session(
        Arc::new(PanicLlm),
        &store,
        &run,
        &root,
        drift_args("remediate-task-a-001-r1-7", "remediate", "second"),
        &["TASK-A-001"],
    )
    .await;
    assert_eq!(same.reused, 1, "a fresh answer is reusable: {same:?}");
    // Deterministic ordering, without sleeping for filesystem timestamps.
    record_source(&v2, "9999-01-01T00:00:00Z");
    let stale = session(
        accepted(),
        &store,
        &run,
        &root,
        drift_args("remediate-task-a-001-r1-9", "remediate", "third"),
        &["TASK-A-001"],
    )
    .await;
    assert_eq!(
        (stale.executed, stale.reused),
        (1, 0),
        "a newer observation requires execution: {stale:?}"
    );
}

#[tokio::test]
async fn ordinal_drift_rejects_a_verdict_without_its_replayed_fix() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = reuse_test_store(&temp);
    let root = store.run_dir(&run.id).join("v2");
    let v2 = WorkflowV2ResultStore::new(root.clone());
    record_source(&v2, "2026-01-01T00:00:00Z");
    let id = "verification-wave-verify-task-a-001-r1-6";
    let first = session(
        accepted(),
        &store,
        &run,
        &root,
        drift_args(id, "verify", "first"),
        &[],
    )
    .await;
    assert_eq!(first.executed, 1, "{first:?}");
    let mut fix = v2.load_call_record(id).unwrap().unwrap();
    fix.call.id = "remediate-task-a-001-r1-5".into();
    fix.call
        .options
        .extra
        .get_mut("remediationContract")
        .unwrap()["stage"] = serde_json::json!("remediate");
    fix.started_at = "2026-01-02T00:00:00Z".into();
    fix.finished_at = fix.started_at.clone();
    v2.save_call_record(&fix).unwrap();
    let mut replay = drift_args(
        "verification-wave-verify-task-a-001-r1-8",
        "verify",
        "second",
    );
    replay["replayedFix"] = serde_json::json!(fix.call.id);
    let vouched = session(
        Arc::new(PanicLlm),
        &store,
        &run,
        &root,
        replay,
        &["TASK-A-001"],
    )
    .await;
    assert_eq!(
        vouched.reused, 1,
        "a verdict vouches for the replayed fix it judged: {vouched:?}"
    );
    let fresh = session(
        accepted(),
        &store,
        &run,
        &root,
        drift_args(
            "verification-wave-verify-task-a-001-r1-10",
            "verify",
            "third",
        ),
        &["TASK-A-001"],
    )
    .await;
    assert_eq!(
        (fresh.executed, fresh.reused),
        (1, 0),
        "a fix without proven replay lineage needs a fresh verdict: {fresh:?}"
    );
}
