#[tokio::test]
async fn v3_dialect_scripts_receive_claude_code_primitives() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (tui_tx, _tui_rx) = bounded_tui_event_channel();
    let client = LiveV2AgentClient::new(
        Arc::new(PanicLlm),
        tui_tx,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "v3 primitives".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store,
        run.id.clone(),
        true,
        None,
        None,
    );

    let summary = runner
        .run(
            r#"
export const meta = {
  name: 'primitives-demo',
  phases: [{ title: 'One' }],
}
export default async function workflow({ agent, phase, log, pipeline, w }) {
  if (typeof agent !== "function" || typeof pipeline !== "function" || typeof w !== "object") {
    throw new Error("v3 primitives missing");
  }
  await phase("Write Things");
  await log("first step done");
  const doubled = await pipeline([1, 2], [async (n) => n * 2]);
  return { doubled };
}
"#,
        )
        .await
        .expect("script summary");

    // phase()/log() persist as deterministic checkpoint records — the
    // journal — and no LLM call is ever needed for them (PanicLlm).
    assert!(
        v2_store
            .load_call_record("phase-1-write-things")
            .expect("phase record")
            .is_some()
    );
    assert!(
        v2_store
            .load_call_record("log-1")
            .expect("log record")
            .is_some()
    );
    assert_eq!(summary.executed, 2);
}

#[tokio::test]
async fn legacy_scripts_still_receive_raw_host_api() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (tui_tx, _tui_rx) = bounded_tui_event_channel();
    let client = LiveV2AgentClient::new(
        Arc::new(PanicLlm),
        tui_tx,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "legacy".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store,
        run.id.clone(),
        true,
        None,
        None,
    );

    let summary = runner
        .run(
            r#"
async function workflow(w) {
  if (typeof w.checkpoint !== "function") {
    throw new Error("legacy host API missing");
  }
  await w.checkpoint("legacy-checkpoint", { task: "legacy" });
  return {};
}
"#,
        )
        .await
        .expect("script summary");

    assert!(
        v2_store
            .load_call_record("legacy-checkpoint")
            .expect("legacy record")
            .is_some()
    );
    assert_eq!(summary.executed, 1);
}

struct CannedAuthorLlm {
    script: String,
}

#[async_trait::async_trait]
impl LlmClient for CannedAuthorLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        let envelope = serde_json::json!({
            "status": "accepted",
            "summary": "authored the workflow script",
            "evidence": [{ "kind": "implementation", "summary": "script authored from the task universe" }],
            "data": { "workflow_js": self.script },
        });
        Ok(LlmResponse {
            content: envelope.to_string(),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }
}

#[tokio::test]
async fn authored_script_lifecycle_authors_persists_and_executes() {
    let authored_script = r#"export const meta = {
  name: 'authored-demo',
  description: 'authored by the canned planner',
  phases: [{ title: 'Only', detail: 'one phase with real agent work' }],
}
export default async function workflow({ agent, phase, log, w }) {
  await phase("Authored Phase");
  const review = await agent("Review the plan and reply with a short confirmation.", {
    label: "demo-review",
  });
  const tasks = [];
  await w.parallel("adversarial-review-map", tasks, {
    tier: "critic", itemKind: "review_map",
    reviewContract: { version: 1, kind: "adversarial_findings", stage: "map", findingsPath: "data.findings", maxFindingsPerItem: 25 }
  });
  const adversarial = await w.reduce("adversarial-review-reduce", { findings: [] }, {
    tier: "critic",
    reviewContract: { version: 1, kind: "adversarial_findings", stage: "reduce_final", sourceMapCallIds: ["adversarial-review-map"], preserveMapFindings: true, accountingField: "adversarial_findings", maxInputBytes: 48000 }
  });
  await w.parallel("coverage-audit-map", tasks, {
    tier: "critic", itemKind: "review_map",
    reviewContract: { version: 1, kind: "uncovered_requirements", stage: "map", findingsPath: "data.findings", maxFindingsPerItem: 25 }
  });
  const coverage = await w.reduce("coverage-audit-reduce", { findings: [] }, {
    tier: "critic",
    reviewContract: { version: 1, kind: "uncovered_requirements", stage: "reduce_final", sourceMapCallIds: ["coverage-audit-map"], preserveMapFindings: true, accountingField: "uncovered_requirements", maxInputBytes: 48000 }
  });
  await log(`authored ran: ${review && review.status}`);
  return {
    accepted: [],
    blocked: [],
    adversarial_findings: adversarial.findings || [],
    uncovered_requirements: coverage.findings || [],
    notes: "authored demo complete",
  };
}
"#;
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (tui_tx, _tui_rx) = bounded_tui_event_channel();
    let client = LiveV2AgentClient::new(
        Arc::new(CannedAuthorLlm {
            script: authored_script.to_string(),
        }),
        tui_tx,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "author and execute".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store.clone(),
        run.id.clone(),
        true,
        None,
        None,
    );
    let authored_path = workflow_store.run_dir(&run.id).join("authored-workflow.js");

    let summary = runner
        .run_authored_script_lifecycle(authored_path.clone(), serde_json::Value::Null)
        .await
        .expect("authored lifecycle summary");

    // The authored script was persisted and carries the dialect marker.
    let persisted = std::fs::read_to_string(&authored_path).expect("authored file");
    assert!(persisted.contains("export const meta"));
    // The author call is journaled like any other call.
    assert!(
        v2_store
            .load_call_record("author-workflow-script")
            .expect("author record")
            .is_some()
    );
    // The authored script executed with the primitives: phase + log records.
    assert!(
        v2_store
            .load_call_record("phase-1-authored-phase")
            .expect("phase record")
            .is_some()
    );
    // The execution's own result surfaced through the script channel.
    let result = summary.script_result.expect("script result");
    assert!(result.contains("authored demo complete"));

    let events = std::fs::read_to_string(workflow_store.run_dir(&run.id).join("events.jsonl"))
        .expect("events");
    let author_finished = events
        .find(r#""call_id":"author-workflow-script","#)
        .expect("author completion event");
    let authored_phase_started = events
        .find(r#""call_id":"phase-1-authored-phase","#)
        .expect("authored phase event");
    let terminal_status = events
        .find(r#""event":"terminal_status""#)
        .expect("terminal status event");
    assert!(
        authored_phase_started < terminal_status,
        "author bootstrap must not emit terminal status before authored execution"
    );
    assert!(
        author_finished < authored_phase_started,
        "authored execution should start after the author bootstrap call"
    );
}

#[test]
fn authored_source_validation_applies_to_persisted_scripts() {
    let source = r#"
// export const meta = { name: 'comment-only' }
export default async function workflow() {
  return { accepted: [], blocked: [], notes: 'missing declaration' };
}
"#;
    assert!(validate_authored_workflow_source(source).is_err());

    let compact_meta = r#"
export const meta={name:'compact',description:'valid compact declaration',phases:[]}
export default async function workflow(){return {accepted:[],blocked:[],notes:'ok'}}
"#;
    assert!(validate_authored_workflow_source(compact_meta).is_ok());
    assert!(normalize_workflow_export(compact_meta).contains("globalThis.__workflowMeta = true"));
}

#[test]
fn authored_task_accounting_is_complete_and_disjoint() {
    let expected = ["TASK-001".to_string(), "TASK-002".to_string()]
        .into_iter()
        .collect();
    assert!(
            validate_authored_task_accounting(
                Some(
                    r#"{"accepted":["TASK-001"],"blocked":[{"taskId":"TASK-002","reason":"provider entitlement denied"}],"adversarial_findings":[],"uncovered_requirements":[]}"#,
                ),
                &expected,
            )
            .is_ok()
        );
    assert!(
        validate_authored_task_accounting(
            Some(r#"{"accepted":["TASK-001"],"blocked":[]}"#),
            &expected,
        )
        .is_err(),
        "missing task accounting must fail closed"
    );
    assert!(
            validate_authored_task_accounting(
                Some(
                    r#"{"accepted":["TASK-001","TASK-002"],"blocked":[{"taskId":"TASK-002","reason":"duplicate"}]}"#,
                ),
                &expected,
            )
            .is_err(),
            "accepted and blocked sets must be disjoint"
        );
}

#[tokio::test]
async fn workflow_returning_with_pending_calls_fails_with_dropped_call_error() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (tui_tx, _tui_rx) = bounded_tui_event_channel();
    let client = LiveV2AgentClient::new(
        Arc::new(SlowAcceptedLlm {
            delay: Duration::from_secs(30),
        }),
        tui_tx,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "fire and forget".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store,
        run.id.clone(),
        true,
        None,
        None,
    );

    let summary = runner
        .run(
            r#"
export const meta = { name: 'dropped-call-demo', phases: [{ title: 'One' }] }
export default async function workflow({ agent }) {
  // Real work MUST be awaited: this slow agent call is dropped mid-flight.
  agent("Do something important that is never awaited.", { label: "orphaned-work" });
  return { accepted: [], blocked: [], notes: "returned early" };
}
"#,
        )
        .await
        .expect("summary");

    assert_eq!(summary.status, WorkflowV2Status::Failed);
    let next_action = summary.next_action.unwrap_or_default();
    assert!(
        summary.failed_call.as_deref() == Some("workflow.js") || !next_action.is_empty(),
        "run must be marked a script failure"
    );
}

#[tokio::test]
async fn phase_with_body_callback_runs_and_awaits_the_body() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (tui_tx, _tui_rx) = bounded_tui_event_channel();
    let client = LiveV2AgentClient::new(
        Arc::new(PanicLlm),
        tui_tx,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "phase body".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store,
        run.id.clone(),
        true,
        None,
        None,
    );

    let summary = runner
        .run(
            r#"
export const meta = { name: 'phase-body-demo', phases: [{ title: 'One' }] }
export default async function workflow({ phase, log }) {
  const value = await phase("With Body", async () => {
    await log("inside the body");
    return 41 + 1;
  });
  if (value !== 42) {
    throw new Error(`phase body result not returned: ${value}`);
  }
  return { accepted: [], blocked: [], notes: "body ran" };
}
"#,
        )
        .await
        .expect("summary");

    // The body's log call proves the callback executed and was awaited.
    assert!(
        v2_store
            .load_call_record("log-1")
            .expect("log record")
            .is_some(),
        "phase body must run"
    );
    let result = summary.script_result.expect("script result");
    assert!(result.contains("body ran"));
}
