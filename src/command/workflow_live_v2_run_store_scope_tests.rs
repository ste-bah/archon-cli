//! Every live agent call runs under the run-store boundary, read-only calls
//! included, and a call working at the project root is not exempt from it.
//!
//! Driven through `run_single_v2_agent_call`, the entry read-only fan-out
//! branches and single calls use, with a port that records the scope in force
//! while the agent runs — the scope the session's guard and tool context copy.

use super::*;
use archon_tools::workflow_read_guard::RunStoreScope;
use archon_workflow::{WorkflowAgentCall, WorkflowAgentOutcome};
use serde_json::json;
use std::sync::Mutex;

#[derive(Default)]
struct ScopeCapture(Mutex<Vec<Option<RunStoreScope>>>);

#[async_trait::async_trait]
impl WorkflowLlmClient for ScopeCapture {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("v2 dispatch uses run_agent")
    }

    async fn run_agent(
        &self,
        _: WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.0
            .lock()
            .unwrap()
            .push(archon_tools::workflow_read_guard::current_run_store());
        Ok(WorkflowAgentOutcome {
            content: json!({
                "status": "accepted",
                "summary": "inspected the repository",
                "evidence": [{"kind": "review", "summary": "read the sources"}],
            })
            .to_string(),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
            stop_reason: None,
        })
    }
}

fn read_only_call(id: &str) -> WorkflowV2CallExecution {
    WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: id.to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: archon_workflow::WorkflowV2HostOptions::default(),
        },
        input: json!({}),
        depends_on: Vec::new(),
    }
}

/// The scope a read-only call at the project root ran under, and the paths
/// it is judged against.
async fn scope_seen_by_a_read_only_call(
    run_id: &str,
) -> (tempfile::TempDir, std::path::PathBuf, RunStoreScope) {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let run = project.join(".archon/workflows").join(run_id);
    std::fs::create_dir_all(run.join("v2/branches")).unwrap();
    std::fs::create_dir_all(project.join("src")).unwrap();
    let store = WorkflowV2ResultStore::new(run.join("v2"));
    let llm = Arc::new(ScopeCapture::default());
    let (ui_sink, _rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client =
        LiveV2AgentClient::new(llm.clone(), ui_sink, Vec::new(), run_id.into(), None, None);

    run_single_v2_agent_call(
        "inspect",
        Some(project.display().to_string()),
        &read_only_call("inspect-sources"),
        &WorkflowV2AgentAdapter::new(),
        &client,
        Some(&store),
        None,
        false,
    )
    .await
    .expect("the scripted call is accepted");

    let seen = llm.0.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "one agent session");
    let scope = seen[0]
        .clone()
        .expect("a read-only call must run under the run-store scope");
    (temp, project, scope)
}

#[tokio::test]
async fn a_read_only_call_at_the_project_root_is_bounded_by_the_run_store() {
    let (_temp, project, scope) = scope_seen_by_a_read_only_call("run-1").await;
    let run = project.join(".archon/workflows/run-1");

    assert!(scope.holds_host_records(&run.join("v2/branches/b/record.json")));
    assert!(scope.holds_host_records(&project.join(".archon/workflows/run-0/state.json")));
    assert!(scope.prunes_walk(&project.join(".archon/workflows/run-0")));
    assert!(!scope.holds_host_records(&run.join("artifacts/report.md")));
    assert!(!scope.holds_host_records(&project.join("src/lib.rs")));
}

/// The completion check's admission rule and the write guard agree: the live
/// run-prefixed report shape is writable, the records beside it are not.
#[tokio::test]
async fn the_scope_admits_exactly_the_run_prefixed_deliverable_of_wf139() {
    let fixture: serde_json::Value = serde_json::from_str(
        archon_test_support::fixtures::WF139_PROJECT_ARTIFACT_WRITE_FALSE_SAFETY,
    )
    .unwrap();
    let run_id = fixture["run_id"].as_str().unwrap();
    let reported = fixture["reported_changed_file"].as_str().unwrap();
    let (_temp, project, scope) = scope_seen_by_a_read_only_call(run_id).await;

    assert!(!scope.holds_host_records(&project.join(reported)));
    assert!(
        scope.holds_host_records(&project.join(format!(".archon/workflows/{run_id}-x/y.json")))
    );
    assert!(scope.holds_host_records(
        &project.join(format!(".archon/workflows/{run_id}/v2/branches/b.json"))
    ));
}
