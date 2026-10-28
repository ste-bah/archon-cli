use super::*;

#[derive(Default)]
struct Replies(std::sync::atomic::AtomicUsize);

#[async_trait::async_trait]
impl WorkflowLlmClient for Replies {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        unreachable!()
    }

    async fn run_agent(
        &self,
        _: archon_workflow::WorkflowAgentCall,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        let n = self.0.fetch_add(1, Ordering::SeqCst);
        Ok(WorkflowAgentOutcome {
            content: "{}".into(),
            stop_reason: Some(if n < 4 { "max_tokens" } else { "end_turn" }.into()),
            ..Default::default()
        })
    }
}

#[tokio::test]
async fn workflow_decompose_rung_transition_uses_real_script_host_checkpoint_without_console() {
    let (_temp, store, run_id) = new_run();
    let replies = Arc::new(Replies::default());
    let (runner, _rx) = runner(
        &store,
        &run_id,
        replies.clone(),
        Some(Arc::new(Judge::default())),
        Some(serde_json::json!({"gateMode":"enforce"})),
    );
    let source = std::env::var("ARCHON_TEST_SCRIPT_ROOT").map_or_else(
        |_| FIXED_SCRIPT_SOURCE.to_owned(),
        |root| {
            [
                "workflow_decompose_v1.js",
                "workflow_decompose_reply_blocks.js",
                "workflow_decompose_v1_acceptance.js",
                "workflow_decompose_v1_set_gate.js",
                "workflow_decompose_v1_frozen.js",
                "workflow_decompose_v1_progress.js",
                "workflow_decompose_v1_seed.js",
                "workflow_decompose_v1_context.js",
            ]
            .iter()
            .map(|name| std::fs::read_to_string(std::path::Path::new(&root).join(name)).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
        },
    );
    let script = format!(
        r#"{source}
async function workflow(w) {{
  if (typeof console !== "undefined") throw new Error("unexpected console");
  return authorCandidate(w, {{phase:"skeleton", capability:"freeze-skeleton",
    prompt:()=>"author", retryScopes:new Set(["candidate_artifact"])}});
}}
"#
    );
    let outcome = runner.run(&script).await.unwrap();
    assert_eq!(outcome.status, WorkflowV2Status::Accepted, "{outcome:?}");
    assert_eq!(replies.0.load(Ordering::SeqCst), 5);
    let records = WorkflowV2ResultStore::new(store.run_dir(&run_id).join("v2"))
        .load_call_records()
        .unwrap();
    assert!(
        records.iter().any(|record| {
            record
                .call
                .options
                .task
                .as_deref()
                .is_some_and(|task| task.contains("skeleton repair rung r2"))
        }),
        "{records:?}"
    );
    let events = std::fs::read_to_string(store.events_path(&run_id)).unwrap();
    let transitions: Vec<serde_json::Value> = events
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .filter(|event: &serde_json::Value| {
            event["detail"]["event"] == "call_started"
                && event["detail"]["call_id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("repair-skeleton-r2-c2-"))
        })
        .collect();
    assert_eq!(
        transitions.len(),
        1,
        "one watch event per rung transition: {events}"
    );
    assert!(
        transitions[0]["detail"]["call_id"]
            .as_str()
            .unwrap()
            .contains("-a3")
    );
}
