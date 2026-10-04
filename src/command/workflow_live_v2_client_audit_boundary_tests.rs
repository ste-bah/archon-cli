//! #236: the repository-audit assessor is confined to its sealed snapshot, and
//! its call names no read roots. This test builds the assessor's call through
//! the real audit client. It then checks that every path the call's text
//! names lies in the snapshot, so the empty list loses nothing. It also
//! checks that the live repository the client was built for never appears
//! in the call's text.

use super::super::*;
use archon_workflow::{
    WorkflowAgentOutcome, WorkflowV2CallExecution, WorkflowV2HostCall, WorkflowV2HostMethod,
};
use std::sync::Mutex;

#[derive(Default)]
struct Recorder(Mutex<Vec<WorkflowAgentCall>>);

#[async_trait::async_trait]
impl WorkflowLlmClient for Recorder {
    /// Scripted replies stand for one continued session (#241).
    async fn continue_agent(
        &self,
        call: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        self.run_agent(call).await
    }

    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("the assessor call is all this test makes")
    }

    async fn run_agent(
        &self,
        call: WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.0.lock().unwrap().push(call);
        Ok(WorkflowAgentOutcome {
            content: "recorded".into(),
            stop_reason: Some("end_turn".into()),
            ..WorkflowAgentOutcome::default()
        })
    }
}

/// Repeatedly turns a doubled backslash into one.
fn collapse(text: &str) -> String {
    let mut text = text.to_string();
    loop {
        let collapsed = text.replace("\\\\", "\\");
        if collapsed == text {
            return text;
        }
        text = collapsed;
    }
}

#[tokio::test]
async fn the_assessor_call_names_only_paths_inside_its_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let live = root.join("live-repository");
    let snapshot = root.join("run/v2/repository-audit/snapshots/s1");
    std::fs::create_dir_all(&live).unwrap();
    std::fs::create_dir_all(&snapshot).unwrap();

    let recorder = Arc::new(Recorder::default());
    let (ui_sink, _rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        recorder.clone(),
        ui_sink,
        Vec::new(),
        "wf-test".into(),
        Some(live.display().to_string()),
        None,
    )
    .for_audit();
    let options = archon_workflow::WorkflowV2HostOptions {
        task: Some(
            "Read-only semantic repository audit of the sealed repository_root. Return data."
                .into(),
        ),
        ..Default::default()
    };
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "repository-audit-1".into(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options,
        },
        input: serde_json::json!({
            "snapshot": "tree",
            "audit_contract": {"schema_version": 1, "snapshot": "tree", "declared_paths": ["src/lib.rs"]}
        }),
        depends_on: vec![],
    };
    // As `AuditDispatch::run_call` builds it.
    let mut request = archon_workflow::v2::call_data::v2_agent_request(
        "semantic repository audit",
        Some(snapshot.display().to_string()),
        &execution,
        None,
    );
    request.role = "critic".into();
    let prompt = archon_workflow::WorkflowV2AgentAdapter::new().build_prompt_parts(&request);
    client
        .run_agent_request(&request, prompt.invocation)
        .await
        .expect("recorded");

    let call = recorder.0.lock().unwrap().pop().expect("one call");
    assert!(
        call.allowed_tools
            .iter()
            .any(|t| t == EXACT_TOOL_POLICY_MARKER)
            && !call.allowed_tools.iter().any(|t| t == "Bash"),
        "the assessor is no longer a workspace-bounded call: {:?}",
        call.allowed_tools
    );
    assert_eq!(call.cwd.as_deref(), Some(snapshot.as_path()));
    assert!(call.read_roots.is_empty(), "{:?}", call.read_roots);

    // JSON escaping is undone at every depth, so that a Windows path reads
    // as itself. The roots are compared after the same change.
    let text = collapse(&format!(
        "{} {} {}",
        call.task,
        serde_json::to_string(&call.messages).unwrap(),
        serde_json::to_string(&call.system).unwrap()
    ));
    let root_text = collapse(&root.display().to_string());
    let snapshot_text = collapse(&snapshot.display().to_string());
    let mut named = 0;
    let mut rest = text.as_str();
    while let Some(at) = rest.find(&root_text) {
        let tail = &rest[at..];
        // Scanned after the root, so a separator inside it never ends the
        // path. An escaped newline ends it, as whitespace and quotes do.
        let after = &tail[root_text.len()..];
        let end = root_text.len()
            + after
                .find(|c: char| c.is_whitespace() || "\"'`,;)]}".contains(c))
                .into_iter()
                .chain(after.find("\\n"))
                .min()
                .unwrap_or(after.len());
        let path = tail[..end].trim_end_matches('.');
        assert!(
            path == snapshot_text
                || path.starts_with(&format!("{snapshot_text}{}", std::path::MAIN_SEPARATOR)),
            "the assessor call names {path:?}, outside its snapshot"
        );
        named += 1;
        rest = &tail[end..];
    }
    assert!(named > 0, "the call names its snapshot root");
}
