//! The repeat-tool chain as observed through the real dispatch path (#200 Phase 2).
//!
//! These drive `ToolRegistry::dispatch` → `execute_tool_attempt` rather than
//! calling the guard directly, so they fail if the observation is ever
//! disconnected from a call site — including the two that never reach the tool.
//!
//! Each test uses its own `session_id`, because the chains are process-global
//! and these run in parallel.

use std::sync::Arc;

use archon_permissions::{
    SandboxBackend, SandboxScope, SandboxScopeSupport, SandboxTerminal, SandboxTerminalRequest,
};
use archon_tools::repeat_tool_guard::{ChainKey, REPEAT_TOOL_CHAINS, RepeatToolConfig};
use archon_tools::tool::{
    PermissionLevel, Tool, ToolCapability, ToolContext, ToolResult, ToolRunAdmission,
};

use crate::dispatch::ToolRegistry;

const TOOL_OUTPUT: &str = "crates/archon-core/src/agent.rs:141: struct PendingToolCall";

/// A tool whose result is a fixed string, so "byte-identical" is checkable.
struct FixedTool {
    name: &'static str,
    level: PermissionLevel,
}

#[async_trait::async_trait]
impl Tool for FixedTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "repeat-tool guard test"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        ToolResult::success(TOOL_OUTPUT)
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        self.level
    }

    fn capability(&self) -> ToolCapability {
        ToolCapability::HostLocal
    }
}

/// A backend that refuses everything, to reach the sandbox-precheck exit.
#[derive(Debug)]
struct RefusingSandbox;

impl SandboxBackend for RefusingSandbox {
    fn check(
        &self,
        tool: &str,
        _capability: ToolCapability,
        _input: &serde_json::Value,
    ) -> Result<(), String> {
        Err(format!("sandbox refused {tool}"))
    }

    fn terminal(&self, _request: &SandboxTerminalRequest) -> SandboxTerminal {
        SandboxTerminal::Refused("no terminals in this fake".to_string())
    }

    fn scope_support(&self, _scope: SandboxScope) -> SandboxScopeSupport {
        SandboxScopeSupport::Held
    }
}

fn registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(FixedTool {
        name: "Grep",
        level: PermissionLevel::Safe,
    }));
    registry.register(Box::new(FixedTool {
        name: "TodoWrite",
        level: PermissionLevel::Safe,
    }));
    registry.register(Box::new(FixedTool {
        name: "Bash",
        level: PermissionLevel::Risky,
    }));
    registry
}

fn ctx(session_id: &str) -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: session_id.to_string(),
        ..Default::default()
    }
}

fn grep_input() -> serde_json::Value {
    serde_json::json!({"pattern": "PendingToolCall", "path": "crates"})
}

#[tokio::test]
async fn three_identical_calls_through_dispatch_earn_one_reminder() {
    let registry = registry();
    let ctx = ctx("dispatch-run");

    for _ in 0..3 {
        registry.dispatch("Grep", grep_input(), &ctx).await;
    }

    let reminders = REPEAT_TOOL_CHAINS.take_reminders(&ChainKey::of(&ctx));
    assert_eq!(reminders.len(), 1, "got {reminders:?}");
    assert!(reminders[0].contains("called Grep 3 times in a row"));
}

/// The tool's own output is what the model reads back. A reminder folded into
/// it would make the audit record show the tool returning text it never
/// produced, and would be charged against the result's byte budget.
#[tokio::test]
async fn the_tool_result_is_byte_identical_and_carries_no_reminder() {
    let registry = registry();
    let ctx = ctx("dispatch-byte-identical");

    let mut last = ToolResult::success("unset");
    for _ in 0..3 {
        last = registry.dispatch("Grep", grep_input(), &ctx).await;
    }

    assert_eq!(
        last.content.as_bytes(),
        TOOL_OUTPUT.as_bytes(),
        "the tool result must be exactly what the tool returned"
    );
    assert!(!last.is_error);
    assert_eq!(
        REPEAT_TOOL_CHAINS.take_reminders(&ChainKey::of(&ctx)).len(),
        1,
        "the reminder must exist — it is just delivered somewhere else"
    );
}

/// A bookkeeping tool interleaved into a loop must not launder it.
#[tokio::test]
async fn an_excluded_tool_between_identical_calls_does_not_reset_the_chain() {
    let registry = registry();
    let ctx = ctx("dispatch-excluded");
    assert!(
        ctx.repeat_tool.exclude.contains(&"TodoWrite".to_string()),
        "this test relies on the documented default exclusion"
    );

    registry.dispatch("Grep", grep_input(), &ctx).await;
    registry
        .dispatch("TodoWrite", serde_json::json!({"todos": []}), &ctx)
        .await;
    registry.dispatch("Grep", grep_input(), &ctx).await;
    registry
        .dispatch("TodoWrite", serde_json::json!({"todos": []}), &ctx)
        .await;
    registry.dispatch("Grep", grep_input(), &ctx).await;

    let reminders = REPEAT_TOOL_CHAINS.take_reminders(&ChainKey::of(&ctx));
    assert_eq!(
        reminders.len(),
        1,
        "three Greps separated by an excluded tool are still a run of three"
    );
    assert!(reminders[0].contains("called Grep 3 times in a row"));
}

/// A call permissions keep rejecting is the loop most worth breaking, and it
/// never reaches the tool — so the observation cannot live only on the
/// execution path.
#[tokio::test]
async fn three_admission_blocked_attempts_earn_a_reminder() {
    let registry = registry();
    let mut ctx = ctx("dispatch-blocked");
    ctx.tool_run_admission = Some(Arc::new(|_request| ToolRunAdmission::Blocked {
        reason: "policy".to_string(),
    }));

    for _ in 0..3 {
        let result = registry
            .dispatch("Bash", serde_json::json!({"command": "ls"}), &ctx)
            .await;
        assert!(result.is_error, "the call must still be blocked");
    }

    let reminders = REPEAT_TOOL_CHAINS.take_reminders(&ChainKey::of(&ctx));
    assert_eq!(reminders.len(), 1, "got {reminders:?}");
    assert!(
        reminders[0].contains("Every one of those 3 calls was refused"),
        "got: {}",
        reminders[0]
    );
}

#[tokio::test]
async fn three_sandbox_refusals_earn_a_reminder() {
    let registry = registry();
    let mut ctx = ctx("dispatch-sandbox-refused");
    ctx.sandbox = Some(Arc::new(RefusingSandbox));

    for _ in 0..3 {
        let result = registry.dispatch("Grep", grep_input(), &ctx).await;
        assert!(result.is_error);
    }

    let reminders = REPEAT_TOOL_CHAINS.take_reminders(&ChainKey::of(&ctx));
    assert_eq!(reminders.len(), 1, "got {reminders:?}");
    assert!(reminders[0].contains("Every one of those 3 calls was refused"));
}

/// `session_id` is copied verbatim from parent to child, so it cannot be the
/// whole key.
#[tokio::test]
async fn a_subagents_repetition_does_not_trip_the_parents_counter() {
    let registry = registry();
    let parent = ctx("dispatch-shared-session");
    let child = ToolContext {
        subagent_id: Some("child-1".to_string()),
        ..parent.clone()
    };

    for _ in 0..3 {
        registry.dispatch("Grep", grep_input(), &child).await;
    }

    assert!(
        REPEAT_TOOL_CHAINS
            .take_reminders(&ChainKey::of(&parent))
            .is_empty(),
        "the child's loop must not be delivered to the parent"
    );
    assert_eq!(
        REPEAT_TOOL_CHAINS
            .take_reminders(&ChainKey::of(&child))
            .len(),
        1
    );
}

/// Turning the guard off has to leave nothing behind — not a quieter guard, no
/// guard.
#[tokio::test]
async fn a_disabled_guard_records_nothing() {
    let registry = registry();
    let mut ctx = ctx("dispatch-disabled");
    ctx.repeat_tool = RepeatToolConfig {
        enabled: false,
        ..RepeatToolConfig::default()
    };

    for _ in 0..8 {
        let result = registry.dispatch("Grep", grep_input(), &ctx).await;
        assert_eq!(result.content, TOOL_OUTPUT);
    }

    assert_eq!(REPEAT_TOOL_CHAINS.run_length(&ChainKey::of(&ctx)), 0);
    assert!(
        REPEAT_TOOL_CHAINS
            .take_reminders(&ChainKey::of(&ctx))
            .is_empty()
    );
}
