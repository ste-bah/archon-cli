//! A replayed remediation verdict vouches for the fix it followed, and only
//! for that fix. A verifier record may answer a verify call only when this
//! session's fix of the same unit and round was itself replayed from the fix
//! that verdict judged; a fix that ran again (a fresh coder, a different
//! lineage) is judged afresh.

use super::workflow_live_v2_reuse_content_key_tests::reuse_test_store;
use super::*;

pub(super) struct CountingAcceptedLlm {
    pub(super) calls: AtomicUsize,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for CountingAcceptedLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut result = WorkflowV2Result::accepted("counted accepted");
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            "counted host call completed",
        ));
        Ok(WorkflowAgentOutcome {
            content: serde_json::to_string(&result).expect("result json"),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
            stop_reason: None,
        })
    }
}

fn runner(
    llm: Arc<dyn WorkflowLlmClient>,
    workflow_store: &WorkflowStore,
    run: &archon_workflow::WorkflowRun,
    v2_store: &WorkflowV2ResultStore,
    fix: u64,
) -> WorkflowV2ScriptRunner {
    let (ui_sink, tui_rx) = default_workflow_ui_sink();
    std::mem::forget(tui_rx);
    let client = LiveV2AgentClient::new(llm, ui_sink, Vec::new(), run.id.clone(), None, None);
    WorkflowV2ScriptRunner::new(
        "verify lineage".to_string(),
        test_runtime(&test_spec()),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store.clone(),
        run.id.clone(),
        true,
        None,
        Some(serde_json::json!({ "fix": fix })),
    )
    .with_frontier_resume(true)
}

/// The prelude's shape: one fix, then its verifier at the next ordinal.
const SCRIPT: &str = r#"
async function workflow(w) {
  const unit = { version: 1, taskId: "TASK-A", round: 1, maxRounds: 2, sourceReduceCallIds: ["r"] };
  await w.agent("review-remediate-task-a-1-" + args.fix, {
    task: "Fix TASK-A: [f1]",
    remediationContract: Object.assign({ stage: "remediate" }, unit),
  });
  await w.agent("review-verify-task-a-1-" + (args.fix + 1), {
    task: "Verify TASK-A: [f1]",
    remediationContract: Object.assign({ stage: "verify" }, unit),
  });
  return "done";
}
"#;

/// A first session records fix 31 and its verdict 32; `restart_fix`
/// invalidates the fix as `restart-stage` would.
async fn recorded(
    restart_fix: bool,
) -> (
    tempfile::TempDir,
    WorkflowStore,
    archon_workflow::WorkflowRun,
    std::path::PathBuf,
) {
    let temp = tempfile::tempdir().expect("tempdir");
    let (workflow_store, run) = reuse_test_store(&temp);
    let root = workflow_store.run_dir(&run.id).join("v2");
    let first_store = WorkflowV2ResultStore::new(root.clone());
    let accepted: Arc<dyn WorkflowLlmClient> = Arc::new(SlowAcceptedLlm {
        delay: Duration::ZERO,
    });
    let first = runner(accepted, &workflow_store, &run, &first_store, 31)
        .run(SCRIPT)
        .await
        .expect("first session");
    assert_eq!(first.executed, 2, "{first:?}");
    if restart_fix {
        let mut fix = first_store
            .load_call_record("review-remediate-task-a-1-31")
            .expect("lookup")
            .expect("fix recorded");
        fix.invalidated_by = Some("review-remediate-task-a-1-31".to_string());
        first_store.save_call_record(&fix).expect("invalidate");
    }
    (temp, workflow_store, run, root)
}

#[tokio::test]
async fn a_verdict_replays_with_the_fix_it_judged() {
    let (_temp, workflow_store, run, root) = recorded(false).await;
    let resumed = runner(
        Arc::new(PanicLlm),
        &workflow_store,
        &run,
        &WorkflowV2ResultStore::new(root),
        29,
    )
    .run(SCRIPT)
    .await
    .expect("resumed session");
    assert_eq!(
        resumed.reused, 2,
        "fix 29<-31 and verdict 30<-32: {resumed:?}"
    );
    assert_eq!(resumed.executed, 0);
}

#[tokio::test]
async fn a_fix_that_ran_again_is_judged_again() {
    for fix in [29, 31] {
        let (_temp, workflow_store, run, root) = recorded(true).await;
        let llm = Arc::new(CountingAcceptedLlm {
            calls: AtomicUsize::new(0),
        });
        let resumed = runner(
            llm.clone(),
            &workflow_store,
            &run,
            &WorkflowV2ResultStore::new(root),
            fix,
        )
        .run(SCRIPT)
        .await
        .expect("resumed session");
        assert_eq!(
            resumed.executed, 2,
            "fix at {fix} ran fresh, so its verdict must be asked again: {resumed:?}"
        );
        assert_eq!(llm.calls.load(Ordering::SeqCst), 2);
    }
}
