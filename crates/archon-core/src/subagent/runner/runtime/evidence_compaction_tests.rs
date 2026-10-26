use super::*;
use crate::subagent::runner::tests::{MockProvider, make_runner, text_response};
use archon_tools::tool::{Tool, ToolContext};
use serde_json::json;
use std::sync::Arc;
struct Landed;
impl archon_tools::audit_landing::LandingHost for Landed {
    fn land(&self, _: serde_json::Value) -> Result<String, String> {
        unreachable!()
    }
    fn hint(&self) -> Result<String, String> {
        Ok(
            "25 of 41 landed; remaining file-25 through file-40; do not re-gather landed paths"
                .into(),
        )
    }
    fn complete(&self, _: &serde_json::Value) -> Result<(), String> {
        Err("16 missing".into())
    }
}
#[tokio::test]
async fn compaction_reinjects_host_landed_records_and_read_orientation() {
    let provider = Arc::new(MockProvider::new(vec![
        text_response("Summary of prior inspection."),
        text_response("Summary of prior inspection."),
    ]));
    let mut runner = make_runner(provider.clone(), 2);
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("data.txt");
    std::fs::write(&path, "alpha".repeat(800)).unwrap();
    let guard = Arc::new(archon_tools::workflow_read_guard::WorkflowReadGuard::new(
        40, 20, false, false,
    ));
    runner.tool_context = ToolContext {
        working_dir: root.path().to_path_buf(),
        session_id: "AC-X-001".into(),
        read_result_replay_limit_bytes: archon_tools::READ_REPLAY_CONTEXT_BYTES,
        workflow_read_guard: Some(guard.clone()),
        ..ToolContext::default()
    };
    let input = json!({"file_path":path,"limit":4});
    assert!(
        guard
            .before_tool_for(&runner.tool_context, "Read", &input)
            .is_none()
    );
    let first = archon_tools::file_read::ReadTool
        .execute(input.clone(), &runner.tool_context)
        .await;
    guard.after_tool("Read", &input, !first.is_error, "ok");
    let read_content = first.content;
    runner.tool_context.audit_landing = Some(Arc::new(
        archon_tools::audit_landing::AuditLanding::new(Arc::new(Landed), None),
    ));
    let mut history = vec![
        json!({"role":"assistant","content":[{"type":"tool_use","id":"r1","name":"Read","input":input}]}),
        json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"r1","content":read_content,"is_error":false}]}),
    ];
    history.extend((0..16).map(|i|json!({"role":if i%2==0{"user"}else{"assistant"},"content":"old evidence ".repeat(2000)})));
    let mut messages = MessageHistory::new(history);
    let mut state = crate::agent::AutoCompactState::default();
    let telemetry = crate::agent::autocompact::CompactionTelemetry {
        provider_family: "fixture",
        wire_shape: "fixture",
        native_context_window: 200_000,
        runtime_context_budget: 200_000,
        context_source: "fixture",
        compaction_backend: "fixture",
    };
    let mut known = 100_000;
    assert!(
        compact_proactively(
            &runner,
            &mut messages,
            &mut state,
            &mut known,
            &telemetry,
            crate::agent::CompactAction::Full,
            "fixture compaction failed"
        )
        .await
    );
    let last = messages.as_slice().last().unwrap()["content"]
        .as_str()
        .unwrap();
    assert!(last.contains("25 of 41"));
    assert!(last.contains("Historical read-set"));
    assert!(last.contains("full contracted artifact"));
    assert!(
        guard
            .before_tool_for(&runner.tool_context, "Read", &input)
            .is_none()
    );
    let second = archon_tools::file_read::ReadTool
        .execute(input.clone(), &runner.tool_context)
        .await;
    assert_eq!(second.content, read_content);
}
