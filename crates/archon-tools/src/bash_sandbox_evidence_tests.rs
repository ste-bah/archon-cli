use std::path::PathBuf;
use std::sync::Arc;

use archon_permissions::sandbox::{SandboxBackend, SandboxCommandRequest, SandboxCommandResult};
use serde_json::json;

use super::*;
use crate::tool::ToolContext;

#[derive(Debug)]
struct FixedSandbox {
    result: SandboxCommandResult,
}

impl SandboxBackend for FixedSandbox {
    fn check(
        &self,
        _tool: &str,
        _capability: archon_permissions::ToolCapability,
        _input: &serde_json::Value,
    ) -> Result<(), String> {
        Ok(())
    }

    fn execute_bash<'a>(
        &'a self,
        _request: SandboxCommandRequest,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Option<SandboxCommandResult>> + Send + 'a>,
    > {
        let result = self.result.clone();
        Box::pin(async move { Some(result) })
    }
}

fn sandbox_ctx(result: SandboxCommandResult) -> ToolContext {
    ToolContext {
        working_dir: PathBuf::from("."),
        session_id: "sandbox-evidence-session".into(),
        tool_run_tool_use_id: Some("sandbox-evidence-tool".into()),
        sandbox: Some(Arc::new(FixedSandbox { result })),
        ..ToolContext::default()
    }
}

#[tokio::test]
async fn completed_sandbox_command_mints_authoritative_execution() {
    let result = BashTool::default()
        .execute(
            json!({"command": "cargo test --lib"}),
            &sandbox_ctx(SandboxCommandResult {
                content: "test result: ok. 1 passed; 0 failed".into(),
                is_error: false,
                exit_code: Some(0),
            }),
        )
        .await;

    let execution = result
        .authoritative_bash_execution()
        .expect("completed sandbox process must carry execution authority");
    assert_eq!(execution.session_id(), "sandbox-evidence-session");
    assert_eq!(execution.tool_use_id(), "sandbox-evidence-tool");
    assert_eq!(execution.command(), "cargo test --lib");
    assert_eq!(execution.exit_code(), 0);
}

#[tokio::test]
async fn failed_sandbox_command_preserves_exact_exit_code() {
    let result = BashTool::default()
        .execute(
            json!({"command": "cargo test --lib"}),
            &sandbox_ctx(SandboxCommandResult {
                content: "Exit code 17\ntest failed".into(),
                is_error: true,
                exit_code: Some(17),
            }),
        )
        .await;

    let execution = result
        .authoritative_bash_execution()
        .expect("completed failing process must carry execution authority");
    assert!(result.is_error);
    assert_eq!(execution.exit_code(), 17);
}

#[tokio::test]
async fn sandbox_transport_failure_cannot_mint_authoritative_execution() {
    let result = BashTool::default()
        .execute(
            json!({"command": "cargo test --lib"}),
            &sandbox_ctx(SandboxCommandResult {
                content: "Error: transport failed".into(),
                is_error: true,
                exit_code: None,
            }),
        )
        .await;

    assert!(result.is_error);
    assert!(result.authoritative_bash_execution().is_none());
}
