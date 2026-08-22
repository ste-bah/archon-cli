//! Cancellation safety for the `lsp` call budget (#200 Phase 1).
//!
//! Opting a tool into the dispatcher's per-call budget means its `execute`
//! future is *dropped* at the deadline. For `lsp` the resource that must not
//! survive that drop is the `LspServerManager` mutex: a guard leaked there
//! would not just fail one call, it would wedge every later `lsp` call in the
//! session behind a lock nobody holds a handle to any more.

use std::sync::Arc;
use std::time::Duration;

use archon_tools::execution_deadline::ExecutionDeadline;
use archon_tools::lsp_manager::{LspConfig, LspServerConfig, LspServerManager};
use archon_tools::lsp_tool::LspTool;
use archon_tools::tool::{Tool, ToolContext};
use tokio::sync::Mutex;

/// A manager pointed at a binary that certainly is not installed, so
/// `ensure_connected` fails immediately on the `which` lookup. That leaves the
/// manager lock as the only thing a call can block on, which is exactly the
/// thing under test.
fn manager_with_missing_server(root: std::path::PathBuf) -> LspServerManager {
    LspServerManager::new(
        root,
        Some(LspConfig {
            servers: vec![LspServerConfig {
                command: Some("archon-no-such-language-server".to_string()),
                args: Some(vec![]),
                language_id: Some("rust".to_string()),
            }],
        }),
    )
}

fn hover_input() -> serde_json::Value {
    serde_json::json!({
        "operation": "hover",
        "file_path": "src/lib.rs",
        "line": 1,
        "character": 1
    })
}

#[test]
fn lsp_declares_a_call_budget() {
    let manager = manager_with_missing_server(std::env::temp_dir());
    let tool = LspTool::new(Arc::new(Mutex::new(manager)));
    assert_eq!(tool.timeout(), Some(Duration::from_secs(60)));
}

#[tokio::test]
async fn dropping_an_lsp_call_at_the_deadline_releases_the_manager_lock() {
    let root = tempfile::tempdir().expect("tempdir");
    let manager = Arc::new(Mutex::new(manager_with_missing_server(
        root.path().to_path_buf(),
    )));
    let tool = LspTool::new(Arc::clone(&manager));
    let ctx = ToolContext {
        working_dir: root.path().to_path_buf(),
        session_id: "lsp-timeout-test".to_string(),
        ..Default::default()
    };

    // Stand in for a wedged in-flight LSP request: the manager is locked, so
    // the call below can get no further than `manager.lock().await`.
    let wedged = Arc::clone(&manager).lock_owned().await;

    let cancelled = ExecutionDeadline::new(Duration::from_millis(200))
        .wait(tool.execute(hover_input(), &ctx))
        .await;
    assert!(
        cancelled.is_none(),
        "the call must not complete while the manager is locked"
    );

    drop(wedged);

    // If the cancelled future had leaked the guard, or stayed queued on the
    // mutex, this second call would never acquire it.
    let after = tokio::time::timeout(Duration::from_secs(5), tool.execute(hover_input(), &ctx))
        .await
        .expect("manager lock must be free once the cancelled call is dropped");

    assert!(after.is_error, "{after:?}");
    assert!(
        after.content.contains("LSP not available"),
        "expected the missing-server error, got: {}",
        after.content
    );
    assert!(
        manager.try_lock().is_ok(),
        "the manager lock must be free after both calls"
    );
}
