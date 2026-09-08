//! Enforcement of a call's DECLARED output shape, exercised through the real
//! host dispatch path.
//!
//! These drive `run_single_v2_agent_call` — the function every live V2 agent
//! call funnels through (single calls, read-only fan-out branches, and
//! write-capable branches via `live_agent_dispatch`) — against a scripted LLM,
//! rather than calling the validator directly. The point is to prove the gate
//! is REACHABLE: a validator that only ever runs from its own unit test leaves
//! the defect in place, and counting the scripted LLM's invocations is the only
//! way to show the bounded re-ask actually happened rather than being assumed.

use super::*;
use archon_workflow::{WorkflowAgentCall, WorkflowAgentOutcome};
use serde_json::json;
use std::sync::Mutex;

#[tokio::test]
async fn declared_output_that_is_satisfied_passes_untouched() {
    let llm = Arc::new(ScriptedLlm::new(vec![accepted_with_items()]));
    let outcome = dispatch(
        &llm,
        declaring_call("collect-work-items", Some(json!(["items"]))),
    )
    .await;

    let result = outcome.expect("a satisfied declaration must not be rejected");
    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert_eq!(
        result.data["items"][0]["id"], "unit-a",
        "the result must reach the caller unaltered: {result:#?}"
    );
    assert_eq!(
        llm.prompt_count(),
        1,
        "a satisfied declaration must not cost a re-ask"
    );
}

/// The defect this whole module exists for: an ACCEPTED result whose declared
/// output is empty used to be admitted, and the fan-out reading `data.items`
/// downstream received nothing while the run reported success.
#[tokio::test]
async fn violated_declared_output_triggers_exactly_one_re_ask_quoting_the_violation() {
    let llm = Arc::new(ScriptedLlm::new(vec![
        accepted_with_empty_items(),
        accepted_with_items(),
    ]));
    let outcome = dispatch(
        &llm,
        declaring_call("collect-work-items", Some(json!(["items"]))),
    )
    .await;

    let result = outcome.expect("the re-asked result satisfies the declaration");
    assert_eq!(result.data["items"][0]["id"], "unit-a");

    let prompts = llm.prompts();
    assert_eq!(
        prompts.len(),
        2,
        "exactly one bounded re-ask: first attempt plus one repair, no more"
    );
    assert!(
        prompts[1].contains("data.items is an empty array"),
        "the re-ask must quote the violation verbatim, not just say the output was wrong: {}",
        prompts[1]
    );
    assert!(
        prompts[1].contains("call declared outputs [items]"),
        "the re-ask must restate the whole declaration it breached: {}",
        prompts[1]
    );
}

/// Exhausting the bound is terminal and says why. There is no third attempt,
/// because a repeat of the same violation shares the `Contract` repair class
/// and so cannot earn one from `differs_from`.
#[tokio::test]
async fn exhausting_the_bound_fails_terminally_with_a_readable_reason() {
    let llm = Arc::new(ScriptedLlm::new(vec![accepted_with_empty_items()]));
    let outcome = dispatch(
        &llm,
        declaring_call("collect-work-items", Some(json!(["items"]))),
    )
    .await;

    let error = outcome
        .expect_err("an unrepaired declared-output violation must not be accepted")
        .to_string();
    assert!(
        error.contains("schema repair failed after bounded retries"),
        "the terminal state must name the exhausted bound: {error}"
    );
    assert!(
        error.contains("data.items is an empty array"),
        "the terminal reason must stay readable, naming the violated output: {error}"
    );
    assert_eq!(
        llm.prompt_count(),
        2,
        "the bound is one re-ask; a repeated identical violation must not buy a third attempt"
    );
}

/// A call that declares nothing has no shape to enforce. Inventing a default
/// required shape would fail every workflow whose calls simply return a summary.
#[tokio::test]
async fn call_declaring_no_outputs_is_untouched() {
    let llm = Arc::new(ScriptedLlm::new(vec![accepted_with_no_data()]));
    let outcome = dispatch(&llm, declaring_call("summarise-findings", None)).await;

    let result = outcome.expect("a call that declared nothing must not be held to a shape");
    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert_eq!(
        llm.prompt_count(),
        1,
        "an undeclared call must not be re-asked for data it never promised"
    );
}

/// An honest non-outcome is not a shape violation. Demanding the declared data
/// from a blocked result would only teach agents to invent what they could not
/// produce, which is worse than a visible failure.
#[tokio::test]
async fn blocked_result_is_not_forced_to_produce_the_declared_output() {
    let llm = Arc::new(ScriptedLlm::new(vec![blocked_without_items()]));
    let outcome = dispatch(
        &llm,
        declaring_call("collect-work-items", Some(json!(["items"]))),
    )
    .await;

    let result = outcome.expect("an honest block must survive the declared-output gate");
    assert_eq!(result.status, WorkflowV2Status::Blocked);
    assert_eq!(
        llm.prompt_count(),
        1,
        "a blocked result claims nothing about the declared output, so it earns no re-ask"
    );
}

/// The declaration is read as the script wrote it, not from a fixed list of
/// names. Any name a workflow declares is enforced the same way, in any
/// language and against any repository.
#[tokio::test]
async fn a_declared_name_other_than_items_is_enforced_the_same_way() {
    let llm = Arc::new(ScriptedLlm::new(vec![accepted_with_items()]));
    let outcome = dispatch(
        &llm,
        declaring_call("collect-findings", Some(json!(["findings"]))),
    )
    .await;

    let error = outcome
        .expect_err("a declared name the result does not carry must be rejected")
        .to_string();
    assert!(
        error.contains("data.findings is absent"),
        "the violation must name the script's own declared output: {error}"
    );
}

async fn dispatch(
    llm: &Arc<ScriptedLlm>,
    execution: WorkflowV2CallExecution,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    let temp = tempfile::tempdir().expect("tempdir");
    let v2_store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    // Held for the duration: the client emits activity onto the sink and a
    // dropped receiver would fail the call for a reason unrelated to the gate.
    let (ui_sink, _tui_rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        llm.clone(),
        ui_sink,
        Vec::new(),
        "declared-output-run".to_string(),
        None,
        None,
    );
    run_single_v2_agent_call(
        "enforce declared outputs",
        None,
        &execution,
        &WorkflowV2AgentAdapter::new(),
        &client,
        Some(&v2_store),
        None,
        false,
    )
    .await
}

fn declaring_call(id: &str, outputs: Option<serde_json::Value>) -> WorkflowV2CallExecution {
    let mut options = archon_workflow::WorkflowV2HostOptions::default();
    if let Some(outputs) = outputs {
        options.extra.insert("outputs".to_string(), outputs);
    }
    WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: id.to_string(),
            method: WorkflowV2HostMethod::Reduce,
            write_mode: None,
            options,
        },
        input: json!({}),
        depends_on: Vec::new(),
    }
}

fn accepted_with_items() -> String {
    json!({
        "status": "accepted",
        "summary": "collected the outstanding work items",
        "evidence": [{"kind": "review", "summary": "read the declared source inventory"}],
        "data": {"items": [{"id": "unit-a", "task": "do the thing"}]},
    })
    .to_string()
}

fn accepted_with_empty_items() -> String {
    json!({
        "status": "accepted",
        "summary": "collected the outstanding work items",
        "evidence": [{"kind": "review", "summary": "read the declared source inventory"}],
        "data": {"items": []},
    })
    .to_string()
}

fn accepted_with_no_data() -> String {
    json!({
        "status": "accepted",
        "summary": "summarised the findings",
        "evidence": [{"kind": "review", "summary": "read the declared source inventory"}],
    })
    .to_string()
}

fn blocked_without_items() -> String {
    json!({
        "status": "blocked",
        "summary": "the source inventory is unreachable",
        "evidence": [{"kind": "blocker", "summary": "declared source path does not exist"}],
    })
    .to_string()
}

/// Replays canned agent bodies and records every prompt it was sent.
///
/// The recorded prompts are the observable this module asserts on: the count is
/// the retry budget actually spent, and the second entry is the re-ask whose
/// text has to carry the violation. Once the script runs out it repeats its
/// last body, so a test that expects exhaustion cannot pass merely because the
/// fake ran dry.
struct ScriptedLlm {
    bodies: Vec<String>,
    prompts: Mutex<Vec<String>>,
}

impl ScriptedLlm {
    fn new(bodies: Vec<String>) -> Self {
        Self {
            bodies,
            prompts: Mutex::new(Vec::new()),
        }
    }

    fn prompts(&self) -> Vec<String> {
        self.prompts.lock().expect("prompts lock").clone()
    }

    fn prompt_count(&self) -> usize {
        self.prompts.lock().expect("prompts lock").len()
    }
}

#[async_trait::async_trait]
impl WorkflowLlmClient for ScriptedLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("v2 dispatch uses run_agent")
    }

    async fn run_agent(
        &self,
        request: WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        let prompt = request
            .messages
            .first()
            .and_then(|message| message.get("content"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let index = {
            let mut prompts = self.prompts.lock().expect("prompts lock");
            prompts.push(prompt);
            prompts.len() - 1
        };
        let content = self.bodies[index.min(self.bodies.len() - 1)].clone();
        Ok(WorkflowAgentOutcome {
            content,
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
            stop_reason: None,
        })
    }
}

#[tokio::test]
async fn empty_reply_is_not_schema_repair() {
    let llm = Arc::new(ScriptedLlm::new(vec![String::new()]));
    let error = dispatch(&llm, declaring_call("empty-provider", None))
        .await.expect_err("empty reply must fail").to_string();
    assert!(!error.contains("schema repair"), "{error}");
    assert!(error.contains("empty reply"), "{error}");
}

#[tokio::test]
async fn empty_reply_persists_run_owned_transport_record() {
    let llm = Arc::new(ScriptedLlm::new(vec![String::new()]));
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let (ui, _rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(llm, ui, vec![], "run".into(), None, None);
    let result = run_single_v2_agent_call("respond", None,
        &declaring_call("empty-provider", None), &WorkflowV2AgentAdapter::new(),
        &client, Some(&store), None, false).await;
    assert!(result.is_err());
    let raw = std::fs::read_to_string(store.root().join("transport.jsonl"))
        .expect("dead run must carry its own transport evidence");
    let rows: Vec<serde_json::Value> = raw.lines().map(|s| serde_json::from_str(s).unwrap()).collect();
    assert!(rows.iter().any(|r| r["kind"] == "agent_call_failed" && r["call_id"] == "empty-provider"));
}

#[tokio::test]
async fn transport_evidence_survives_spawned_http_agent_and_repair() {
    use archon_llm::{anthropic::{AnthropicClient, MessageRequest}, auth::AuthProvider,
        identity::{IdentityMode, IdentityProvider}, types::Secret};
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    struct HttpAgent(String);
    #[async_trait::async_trait]
    impl WorkflowLlmClient for HttpAgent {
        async fn send_message(&self, _: Vec<serde_json::Value>, _: Vec<serde_json::Value>,
            _: Vec<serde_json::Value>, _: &str) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> { unreachable!() }
        async fn run_agent(&self, _: WorkflowAgentCall) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
            let url = self.0.clone();
            archon_observability::spawn_named("http-agent-fixture", async move {
                let client = AnthropicClient::new(AuthProvider::ApiKey(Secret::new("fixture-secret".into())),
                    IdentityProvider::new(IdentityMode::Clean, "session".into(), "device".into(), String::new()), Some(url));
                let mut rx = client.stream_message(MessageRequest::default()).await.unwrap();
                while rx.recv().await.is_some() {}
            }).await.unwrap();
            Ok(WorkflowAgentOutcome { content: String::new(), tool_uses: vec![], tokens_in: 0, tokens_out: 0, stop_reason: None })
        }
    }
    let server = MockServer::start().await;
    let body = "event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"max_tokens\"}}\n\nevent: message_stop\ndata: {}\n\n";
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_string(body)).mount(&server).await;
    let root = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(root.path().join("v2"));
    let (ui, _rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(Arc::new(HttpAgent(server.uri())), ui, vec![], "run".into(), None, None);
    let error = run_single_v2_agent_call("respond", None, &declaring_call("http-empty", None),
        &WorkflowV2AgentAdapter::new(), &client, Some(&store), None, false).await.unwrap_err().to_string();
    assert!(!error.contains("schema repair"), "{error}");
    let raw = std::fs::read_to_string(store.root().join("transport.jsonl")).unwrap();
    let rows: Vec<serde_json::Value> = raw.lines().map(|s| serde_json::from_str(s).unwrap()).collect();
    let captures: Vec<_> = rows.iter().filter(|r| r["kind"] == "http_response").collect();
    assert!(!captures.is_empty(), "HTTP instrumentation or task inheritance is disconnected: {raw}");
    for record in captures {
        assert_eq!(record["call_id"], "http-empty");
        assert_eq!(record["http_status"], 200);
        assert_eq!(record["finish_reason"], "max_tokens");
        assert_eq!(record["body_bytes"], body.len());
    }
}

#[tokio::test]
async fn repository_audit_dispatch_uses_selected_timeout_and_read_only_tools() {
    struct Capture(Mutex<Option<WorkflowAgentCall>>);
    #[async_trait::async_trait]
    impl WorkflowLlmClient for Capture {
        async fn send_message(&self,_:Vec<serde_json::Value>,_:Vec<serde_json::Value>,_:Vec<serde_json::Value>,_:&str)->archon_workflow::WorkflowResult<WorkflowAgentOutcome>{unreachable!()}
        async fn run_agent(&self,call:WorkflowAgentCall)->archon_workflow::WorkflowResult<WorkflowAgentOutcome>{
            *self.0.lock().unwrap()=Some(call);
            Ok(WorkflowAgentOutcome{content:accepted_with_items(),tool_uses:vec![],tokens_in:0,tokens_out:0,stop_reason:Some("end_turn".into())})
        }
    }
    use archon_workflow::WorkflowAgentDispatch;
    let capture=Arc::new(Capture(Mutex::new(None)));
    struct Progress(Mutex<String>);
    #[async_trait::async_trait]
    impl archon_workflow::ui_sink_port::WorkflowUiSink for Progress {
        async fn emit(&self, event: archon_workflow::WorkflowUiEvent) -> archon_workflow::ui_sink_port::WorkflowUiResult {
            if let archon_workflow::WorkflowUiEvent::Text(text) = event { self.0.lock().unwrap().push_str(&text); }
            Ok(())
        }
    }
    let progress = Arc::new(Progress(Mutex::new(String::new())));
    let ui = progress.clone();
    let client=LiveV2AgentClient::new(capture.clone(),ui,vec![],"run".into(),None,Some(17));
    let mut execution=declaring_call("audit-timeout",None);
    execution.call.options.extra.insert("audit_timeout_secs".into(),json!(7200));
    super::workflow_live_v2_script::AuditDispatch(client.for_audit()).run_call("audit",None,&execution,&WorkflowV2AgentAdapter::new(),None,None).await.unwrap();
    let call=capture.0.lock().unwrap().take().unwrap();
    assert_eq!(call.timeout_secs,Some(7200));
    let text = progress.0.lock().unwrap();
    assert!(text.contains("Repository audit waiting") && text.contains("7200"), "audit wait and allowance not visible: {text}");
    assert!(!call.allowed_tools.iter().any(|t|matches!(t.as_str(),"Bash"|"Write"|"Edit"|"Agent")));
}

#[tokio::test]
async fn repository_audit_direct_implementation_requires_sealed_dispatch() {
    use archon_workflow::repository_audit::{runtime::AuditRuntime, budget::{AuditPolicy, Limit}};
    struct InspectRoot(Mutex<Vec<std::path::PathBuf>>);
    #[async_trait::async_trait]
    impl WorkflowLlmClient for InspectRoot {
        async fn send_message(&self,_:Vec<serde_json::Value>,_:Vec<serde_json::Value>,_:Vec<serde_json::Value>,_:&str)->archon_workflow::WorkflowResult<WorkflowAgentOutcome>{unreachable!()}
        async fn run_agent(&self, call: WorkflowAgentCall)->archon_workflow::WorkflowResult<WorkflowAgentOutcome>{
            if let Some(root) = call.cwd { self.0.lock().unwrap().push(root.into()); }
            Ok(WorkflowAgentOutcome{content:blocked_without_items(),tool_uses:vec![],tokens_in:0,tokens_out:0,stop_reason:Some("end_turn".into())})
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    for args in [vec!["init", "-q"], vec!["-c", "user.name=fixture", "-c", "user.email=fixture@example.invalid", "commit", "--allow-empty", "-qm", "base"]] {
        assert!(std::process::Command::new("git").current_dir(&repo).args(args).status().unwrap().success());
    }
    let store = WorkflowStore::project(&temp.path().join("project"));
    let spec = archon_workflow::WorkflowSpec{schema:archon_workflow::spec::WORKFLOW_SCHEMA.into(),name:"direct-write".into(),task:"deliver".into(),
        target_repository_root:Some(repo.display().to_string()),max_agents:1,max_parallelism:1,stages:vec![],permissions:Default::default(),learning_hooks:vec![]};
    let run = store.create_run(spec).unwrap();
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let audit = AuditRuntime::initialize(store.clone(),run.id.clone(),AuditPolicy{
        attempt_timeout_secs:Limit::Unlimited,total_time_secs:Limit::Unlimited,unexpected_change_refreshes:Limit::Unlimited}).unwrap();
    let llm = Arc::new(InspectRoot(Mutex::new(vec![])));
    let (ui,_rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(llm.clone(),ui,vec![],run.id.clone(),Some(repo.display().to_string()),None).with_audit(audit.clone());
    let mut execution = declaring_call("direct-write",None);
    execution.call.method = WorkflowV2HostMethod::Implementation;
    execution.call.write_mode = Some(archon_workflow::WorkflowV2WriteMode::Serial);
    execution.call.options.target_files = vec!["new.txt".into()];
    let runtime = WorkflowV2ScriptRuntime{target_repository_root:Some(repo.display().to_string()),..Default::default()};
    execute_v2_live_call("deliver",&runtime,execution,WorkflowV2AgentAdapter::new(),&client,&v2,&store,&run.id,true,None,None,false).await.unwrap();
    let roots = llm.0.lock().unwrap();
    assert!(!roots.is_empty());
    assert!(roots.iter().all(|root| root.join(".git").is_file()), "direct writer was sent to canonical repository: {roots:?}");
    assert!(audit.state().unwrap().declared_paths.contains("new.txt"));
}
