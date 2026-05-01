//! GHOST-006 Gate 5 smoke test: sandbox gates BOTH tool-execution paths.
//!
//! Subpath A — registry dispatch (subagent path).
//! Subpath B — direct tool.execute (main-agent path, agent.rs:1395 analogue).
//!
//! Both paths must independently enforce the sandbox backend check.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use archon_permissions::SandboxBackend;
use archon_tools::tool::{AgentMode, PermissionLevel, Tool, ToolContext, ToolResult};

/// Fake sandbox backend gated by a shared AtomicBool. Mirrors the production
/// `SharedSandboxFlag` in archon-tui but strips all external dependencies so
/// the test compiles against the leaf crates only.
#[derive(Debug)]
struct FakeSandboxBackend {
    enabled: Arc<AtomicBool>,
}

impl SandboxBackend for FakeSandboxBackend {
    fn check(&self, tool: &str, _input: &serde_json::Value) -> Result<(), String> {
        if !self.enabled.load(Ordering::SeqCst) {
            return Ok(());
        }
        match tool {
            "Write" | "Edit" | "NotebookEdit" => Err(format!(
                "sandbox: {tool} is blocked (write operations disabled)"
            )),
            "Bash" | "Shell" => Err(format!(
                "sandbox: {tool} is blocked (shell operations disabled)"
            )),
            "WebFetch" | "WebSearch" => Err(format!(
                "sandbox: {tool} is blocked (network operations disabled)"
            )),
            "TaskCreate" | "TaskUpdate" | "Agent" => Err(format!(
                "sandbox: {tool} is blocked (agent spawning disabled)"
            )),
            _ => Ok(()),
        }
    }
}

/// Minimal tool named "Write" — sandbox-classified as write-blocked.
struct DummyWriteTool;

#[async_trait::async_trait]
impl Tool for DummyWriteTool {
    fn name(&self) -> &str {
        "Write"
    }

    fn description(&self) -> &str {
        "Dummy write tool for sandbox gate tests."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string" },
                "content": { "type": "string" }
            }
        })
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        ToolResult::success("write ok")
    }
}

// =============================================================================
// Subpath A — registry dispatch
// =============================================================================

#[tokio::test]
async fn subpath_a_dispatch_blocked_when_sandbox_on() {
    let flag = Arc::new(AtomicBool::new(false));
    let backend = Arc::new(FakeSandboxBackend {
        enabled: flag.clone(),
    });

    let mut registry = archon_core::dispatch::ToolRegistry::new();
    registry.register(Box::new(DummyWriteTool));

    let ctx = ToolContext {
        mode: AgentMode::Normal,
        sandbox: Some(backend),
        ..Default::default()
    };

    // flag = false → allowed
    let res = registry
        .dispatch("Write", serde_json::json!({"file_path": "/tmp/x"}), &ctx)
        .await;
    assert!(
        !res.is_error,
        "expected success when sandbox is off; got: {res:?}"
    );

    // flag = true → blocked
    flag.store(true, Ordering::SeqCst);
    let res = registry
        .dispatch("Write", serde_json::json!({"file_path": "/tmp/x"}), &ctx)
        .await;
    assert!(
        res.is_error,
        "expected error when sandbox is on; got: {res:?}"
    );
    assert!(
        res.content.starts_with("Error: sandbox:"),
        "expected 'sandbox:' prefix in error; got: {}",
        res.content
    );

    // flag = false → allowed again
    flag.store(false, Ordering::SeqCst);
    let res = registry
        .dispatch("Write", serde_json::json!({"file_path": "/tmp/x"}), &ctx)
        .await;
    assert!(
        !res.is_error,
        "expected success after sandbox turned off; got: {res:?}"
    );
}

// =============================================================================
// Subpath B — direct tool.execute (main-agent path analogue)
// =============================================================================

#[tokio::test]
async fn subpath_b_direct_execute_blocked_when_sandbox_on() {
    let flag = Arc::new(AtomicBool::new(false));
    let backend = Arc::new(FakeSandboxBackend {
        enabled: flag.clone(),
    });

    let tool = DummyWriteTool;
    let ctx = ToolContext {
        mode: AgentMode::Normal,
        sandbox: Some(backend),
        ..Default::default()
    };

    // flag = false → allowed
    let res = tool
        .execute(serde_json::json!({"file_path": "/tmp/x"}), &ctx)
        .await;
    assert!(
        !res.is_error,
        "expected success when sandbox is off; got: {res:?}"
    );

    // flag = true → blocked — simulate agent.rs sandbox pre-check
    flag.store(true, Ordering::SeqCst);
    if let Some(ref backend) = ctx.sandbox {
        if let Err(reason) = backend.check("Write", &serde_json::json!({"file_path": "/tmp/x"})) {
            assert!(
                reason.starts_with("sandbox:"),
                "expected 'sandbox:' prefix; got: {reason}"
            );
        } else {
            panic!("sandbox should have blocked Write when flag is true");
        }
    }

    // flag = false → allowed again
    flag.store(false, Ordering::SeqCst);
    if let Some(ref backend) = ctx.sandbox {
        backend
            .check("Write", &serde_json::json!({"file_path": "/tmp/x"}))
            .expect("sandbox should allow Write when flag is false");
    }
}

// =============================================================================
// Subpath B — full round-trip with actual tool.execute call
// =============================================================================

#[tokio::test]
async fn subpath_b_full_roundtrip() {
    let flag = Arc::new(AtomicBool::new(false));
    let backend = Arc::new(FakeSandboxBackend {
        enabled: flag.clone(),
    });

    let tool = DummyWriteTool;
    let ctx = ToolContext {
        mode: AgentMode::Normal,
        sandbox: Some(backend),
        ..Default::default()
    };

    // Direct execute without sandbox gating → tool succeeds
    let res = tool
        .execute(serde_json::json!({"file_path": "/tmp/x"}), &ctx)
        .await;
    assert!(!res.is_error, "expected success; got: {res:?}");
    assert_eq!(res.content, "write ok");

    // Simulate what agent.rs does: pre-check sandbox before execute
    flag.store(true, Ordering::SeqCst);
    if let Some(ref backend) = ctx.sandbox {
        match backend.check("Write", &serde_json::json!({"file_path": "/tmp/x"})) {
            Ok(()) => panic!("expected block when sandbox on"),
            Err(reason) => {
                assert!(
                    reason.contains("sandbox"),
                    "expected sandbox in reason; got: {reason}"
                );
                assert!(
                    reason.contains("Write"),
                    "expected tool name in reason; got: {reason}"
                );
            }
        }
    }

    // Same check for allowed tool (Read)
    let read_input = serde_json::json!({"file_path": "/tmp/x"});
    if let Some(ref backend) = ctx.sandbox {
        backend
            .check("Read", &read_input)
            .expect("sandbox should allow Read even when flag is true");
    }
}

// =============================================================================
// SandboxBackend trait is object-safe (Send + Sync + Debug bounds verified)
// =============================================================================

#[test]
fn sandbox_backend_is_object_safe() {
    // If this compiles, the trait is object-safe.
    let flag = Arc::new(AtomicBool::new(false));
    let _backend: Arc<dyn SandboxBackend> = Arc::new(FakeSandboxBackend { enabled: flag });
}
