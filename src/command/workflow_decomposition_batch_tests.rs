use super::*;
struct BatchLlm { active: AtomicUsize, peak: AtomicUsize }
#[async_trait::async_trait]
impl WorkflowLlmClient for BatchLlm {
    async fn send_message(&self, _:Vec<serde_json::Value>, _:Vec<serde_json::Value>, _:Vec<serde_json::Value>, _: &str) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> { unreachable!() }
    async fn run_agent(&self, call: archon_workflow::WorkflowAgentCall) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        assert!(archon_tools::read_boundary::current().contains(&".archon".into()), "raw dispatch lost host exclusion policy");
        let n = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(n, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        let id = call.task.lines().find_map(|line| line.strip_prefix("Author ONLY entry "))
            .and_then(|line| line.split(':').next()).unwrap_or("unused");
        Ok(WorkflowAgentOutcome {content:serde_json::json!({"id":id}).to_string(),stop_reason:Some("end_turn".into()),..Default::default()})
    }
}
#[tokio::test]
async fn fixed_batches_overlap_through_real_quickjs_host_and_keep_read_policy() {
    let temp = tempfile::tempdir().unwrap();
    let spec = test_spec();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store.create_run(spec.clone()).unwrap();
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let (sink, _rx) = default_workflow_ui_sink();
    let llm = Arc::new(BatchLlm {active:AtomicUsize::new(0),peak:AtomicUsize::new(0)});
    let client = LiveV2AgentClient::new(llm.clone(),sink,vec![],run.id.clone(),None,Some(10))
        .with_fixed_raw_tool_policy(vec!["Read".into(),"Grep".into(),"Glob".into()]);
    let criteria = (1..=7).map(|n|(format!("AC-X-{n:03}"),format!("criterion {n}"))).collect::<std::collections::BTreeMap<_,_>>();
    let summary = WorkflowV2ScriptRunner::new("batch proof".into(),test_runtime(&spec),WorkflowV2AgentAdapter::new(),client,v2,store,run.id,true,None,
        Some(serde_json::json!({"projectRoot":temp.path(),"prdPath":temp.path().join("prd.md"),"prdDigest":"a".repeat(64),"taskRoot":temp.path().join("tasks"),"gateMode":"observe","acceptanceCriteria":criteria,"authorMaxParallelism":3})))
        .with_raw_outcomes(true).with_host_command_executor(Arc::new(PhaseHost {calls:Mutex::new(vec![])}))
        .run(crate::command::workflow_decompose::FIXED_SCRIPT_SOURCE).await.unwrap();
    assert_eq!(summary.status, WorkflowV2Status::Accepted);
    assert_eq!(llm.peak.load(Ordering::SeqCst),3);
    assert_eq!(llm.active.load(Ordering::SeqCst),0);
}
