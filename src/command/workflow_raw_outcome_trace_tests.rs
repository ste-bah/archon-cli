//! Issue 276: a raw-outcome agent result records what the session's tool
//! trace shows it read and ran, or says plainly that no trace was recorded.

use super::*;

struct TracedRawLlm {
    tool_uses: Vec<archon_workflow::WorkflowAgentToolUse>,
    /// Calls that fail with a transient error before one succeeds.
    transient_failures: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for TracedRawLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        panic!("fixed raw outcome must use run_agent")
    }

    async fn run_agent(
        &self,
        _request: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        let left = &self.transient_failures;
        if left.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            left.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            return Err(archon_workflow::WorkflowError::StageFailed(
                "connection reset by peer".to_string(),
            ));
        }
        Ok(WorkflowAgentOutcome {
            content: "opaque candidate bytes".to_string(),
            tool_uses: self.tool_uses.clone(),
            tokens_in: 1,
            tokens_out: 1,
            stop_reason: Some("end_turn".to_string()),
        })
    }
}

fn tool_use(
    name: &str,
    input: serde_json::Value,
    is_error: bool,
) -> archon_workflow::WorkflowAgentToolUse {
    archon_workflow::WorkflowAgentToolUse {
        tool_name: name.to_string(),
        input,
        output: serde_json::json!({ "is_error": is_error }),
    }
}

async fn run_raw_author(
    tool_uses: Vec<archon_workflow::WorkflowAgentToolUse>,
) -> archon_workflow::WorkflowV2Result {
    run_raw_author_with_files(tool_uses).await.0
}

async fn run_raw_author_with_files(
    tool_uses: Vec<archon_workflow::WorkflowAgentToolUse>,
) -> (archon_workflow::WorkflowV2Result, String) {
    run_raw_author_after_failures(tool_uses, 0).await
}

/// The stored author result and the text of every file the run wrote, the
/// provider failing transiently `failures` times first.
async fn run_raw_author_after_failures(
    tool_uses: Vec<archon_workflow::WorkflowAgentToolUse>,
    failures: usize,
) -> (archon_workflow::WorkflowV2Result, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(TracedRawLlm {
            tool_uses,
            transient_failures: failures.into(),
        }),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        Some(600),
    )
    .with_fixed_raw_tool_policy(vec!["Read".into(), "Grep".into(), "Bash".into()]);
    let runner = WorkflowV2ScriptRunner::new(
        "raw author".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store,
        run.id,
        true,
        None,
        None,
    )
    .with_raw_outcomes(true);
    runner
        .run(
            r#"async function workflow(w) {
  return await w.agent("acceptance-author-1", { task: "Author", tier: "planner", resultMode: "rawOutcome" });
}"#,
        )
        .await
        .expect("raw outcome run");
    let records = v2_store.load_call_records().expect("call records");
    let result = records
        .into_iter()
        .find(|record| record.call.id.contains("acceptance-author-1"))
        .expect("author record")
        .result;
    let written = walkdir::WalkDir::new(temp.path())
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .collect::<Vec<_>>()
        .join("\n");
    (result, written)
}

#[tokio::test]
async fn raw_outcome_result_records_files_read_and_commands_run_from_the_tool_trace() {
    let result = run_raw_author(vec![
        tool_use("Read", serde_json::json!({"file_path": "src/a.rs"}), false),
        tool_use("Read", serde_json::json!({"file_path": "src/b.rs"}), false),
        tool_use("Read", serde_json::json!({"file_path": "src/a.rs"}), false),
        tool_use(
            "Grep",
            serde_json::json!({"pattern": "fn main", "path": "src"}),
            false,
        ),
        tool_use(
            "Bash",
            serde_json::json!({"command": "cargo build --offline"}),
            true,
        ),
    ])
    .await;

    let read: Vec<&str> = result.files_read.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(read, ["src/a.rs", "src/b.rs"], "{result:?}");
    let commands: Vec<&str> = result
        .commands_run
        .iter()
        .map(|c| c.command.as_str())
        .collect();
    assert!(
        commands
            .iter()
            .any(|c| c.contains("Grep") && c.contains("fn main")),
        "{commands:?}"
    );
    let bash = result
        .commands_run
        .iter()
        .find(|c| c.command == "cargo build (2 args)")
        .expect("the Bash call is recorded");
    assert_eq!(
        bash.status,
        archon_workflow::WorkflowV2CommandStatus::Failed
    );
    assert_eq!(
        result.data["toolTrace"]["recorded"], true,
        "{}",
        result.data
    );
    assert_eq!(result.data["toolTrace"]["toolCalls"], 5, "{}", result.data);
}

#[tokio::test]
async fn raw_outcome_without_a_tool_trace_says_not_recorded_instead_of_none() {
    let result = run_raw_author(Vec::new()).await;

    assert!(result.files_read.is_empty());
    assert!(result.commands_run.is_empty());
    assert_eq!(
        result.data["toolTrace"]["recorded"], false,
        "{}",
        result.data
    );
    assert_eq!(
        result.data["toolTrace"]["filesRead"], "not_recorded",
        "{}",
        result.data
    );
    assert_eq!(
        result.data["toolTrace"]["commandsRun"], "not_recorded",
        "{}",
        result.data
    );
}

#[tokio::test]
async fn raw_outcome_with_a_host_trace_of_zero_calls_stores_empty_lists_as_fact() {
    let summary = archon_workflow::WorkflowAgentToolUse {
        tool_name: archon_tools::subagent_session::TOOL_TRACE_SUMMARY_NAME.to_string(),
        input: serde_json::json!({"calls": 0, "kept": 0, "dropped": 0, "inputs_truncated": 0}),
        output: serde_json::Value::Null,
    };
    let result = run_raw_author(vec![summary]).await;

    assert!(result.files_read.is_empty());
    assert!(result.commands_run.is_empty());
    let trace = &result.data["toolTrace"];
    assert_eq!(trace["recorded"], true, "{trace}");
    assert_eq!(trace["toolCalls"], 0, "{trace}");
    assert!(!trace.to_string().contains("not_recorded"), "{trace}");
}

#[tokio::test]
async fn raw_outcome_trace_never_persists_a_credential_from_a_tool_input() {
    // Token-shaped values built at run time, so no literal sits in the source.
    let github = format!("ghp_{}", "Q7w8E9r0T1".repeat(4).get(..36).unwrap());
    let anthropic = format!("sk-ant-api03-{}", "Yy8".repeat(10));
    let (result, written) = run_raw_author_with_files(vec![
        tool_use(
            "Grep",
            serde_json::json!({"pattern": github.clone()}),
            false,
        ),
        tool_use(
            "Bash",
            serde_json::json!({"command": format!("deploy --key {anthropic}")}),
            false,
        ),
    ])
    .await;

    assert_eq!(result.commands_run.len(), 2, "{result:?}");
    assert!(written.contains("toolTrace"), "the run wrote its result");
    for secret in [&github, &anthropic] {
        assert!(
            !written.contains(secret.as_str()),
            "a run file holds {secret}"
        );
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains(secret.as_str())
        );
    }
}

/// A PEM private key block, built at run time so no literal key sits in the
/// source.
fn pem_block() -> String {
    let kind = "PRIVATE KEY";
    format!(
        "-----BEGIN {kind}-----\n{}\n-----END {kind}-----",
        "MIIEvQIBADANBgkqhkiG9w0BAQEFAASC".repeat(40)
    )
}

#[tokio::test]
async fn no_secret_from_any_tool_input_reaches_a_run_file() {
    let key_id = "f00dfacecafe0123beef4567";
    let account = serde_json::json!({"type": "service_account", "private_key_id": key_id,
        "private_key": pem_block()})
    .to_string();
    let big = format!("{}\n{}", "x".repeat(200_000), pem_block());
    let (result, written) = run_raw_author_with_files(vec![
        tool_use("Write", serde_json::json!({"file_path": "keys/a.pem", "content": big}), false),
        tool_use("Write", serde_json::json!({"file_path": "sa.json", "content": account}), false),
        tool_use(
            "Bash",
            serde_json::json!({"command": "DB_PASSWORD=hunter2xyz FOO_TOKEN=abc123xyz ./deploy"}),
            false,
        ),
        tool_use(
            "WebFetch",
            serde_json::json!({"url": "https://example.invalid/api?key=AIzaQ1W2E3R4", "prompt": "p"}),
            false,
        ),
    ])
    .await;

    assert_eq!(result.commands_run.len(), 4, "{result:?}");
    assert!(written.contains("toolTrace") && written.contains("sa.json"));
    for secret in [
        "MIIEvQIBADAN",
        key_id,
        "hunter2xyz",
        "abc123xyz",
        "AIzaQ1W2E3R4",
    ] {
        assert!(!written.contains(secret), "a run file holds {secret}");
    }
}

#[tokio::test]
async fn a_retried_attempt_marks_the_trace_incomplete() {
    let summary = archon_workflow::WorkflowAgentToolUse {
        tool_name: archon_tools::subagent_session::TOOL_TRACE_SUMMARY_NAME.to_string(),
        input: serde_json::json!({"calls": 0, "kept": 0, "dropped": 0, "inputs_truncated": 0}),
        output: serde_json::Value::Null,
    };
    let (result, _) = run_raw_author_after_failures(vec![summary], 1).await;

    let trace = &result.data["toolTrace"];
    assert_eq!(trace["recorded"], true, "{trace}");
    assert_eq!(trace["complete"], false, "{trace}");
    assert!(
        trace["incompleteReasons"][0]
            .as_str()
            .is_some_and(|reason| reason.contains("retry")),
        "{trace}"
    );
}

#[tokio::test]
async fn no_bash_credential_form_reaches_a_run_file() {
    let commands = [
        "mysql -uroot -phunter2 app",
        "psql --password hunter2 -h db",
        "sshpass -p hunter2 ssh deploy@host",
        "curl -u admin:hunter2 https://api.invalid/x",
        r#"curl -d "{\"password\":\"hunter2\"}" https://api.invalid/login"#,
        "curl -H 'Authorization: Basic aHVudGVyMg==' https://api.invalid/x",
        "redis-cli AUTH hunter2",
        "vault login hunter2Token",
        "echo hunter2 | docker login --password-stdin",
        "htpasswd f user hunter2",
    ];
    let uses = commands
        .iter()
        .map(|command| tool_use("Bash", serde_json::json!({"command": command}), false))
        .collect();
    let (result, written) = run_raw_author_with_files(uses).await;

    assert_eq!(result.commands_run.len(), commands.len(), "{result:?}");
    assert!(written.contains("toolTrace") && written.contains("sshpass"));
    for secret in ["hunter2", "aHVudGVyMg"] {
        assert!(!written.contains(secret), "a run file holds {secret}");
    }
}
