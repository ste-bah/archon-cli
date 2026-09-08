//! What a script's tool call has to survive besides the permission checker.
//!
//! Each of these fails if `run` goes back to calling `Tool::execute`: the
//! layers asserted here live between [`ToolRegistry::dispatch`] and the tool,
//! and none of them is reachable from the tool itself. They assert on the
//! *effect* — a refusal, an unrun tool, a chain that grew — rather than on the
//! host's fields, because a host that stored the right context and dispatched
//! past it would satisfy any field assertion.
//!
//! Session ids are unique per test: the repeat-tool chains are process-global
//! and these run in parallel.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use archon_tools::repeat_tool_guard::{ChainKey, REPEAT_TOOL_CHAINS};
use archon_tools::tool::{PermissionLevel, Tool, ToolContext, ToolResult, ToolRunAdmission};

use super::*;

/// Counts its own executions, so "was refused" can be distinguished from
/// "ran and returned an error" — which is the distinction the whole bypass is
/// about.
#[derive(Debug)]
struct CountingTool {
    runs: Arc<AtomicUsize>,
    level: PermissionLevel,
}

#[async_trait::async_trait]
impl Tool for CountingTool {
    fn name(&self) -> &str {
        "ScriptProbe"
    }

    fn description(&self) -> &str {
        "Counts how many times it actually ran."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }

    fn capability(&self) -> archon_permissions::ToolCapability {
        archon_permissions::ToolCapability::EXECUTION
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        self.level
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        self.runs.fetch_add(1, Ordering::SeqCst);
        ToolResult::success("ran")
    }
}

/// A world that refuses every capability, to reach the sandbox-precheck exit.
#[derive(Debug)]
struct RefusingWorld;

impl archon_permissions::SandboxBackend for RefusingWorld {
    fn check(
        &self,
        tool: &str,
        _capability: archon_permissions::ToolCapability,
        _input: &serde_json::Value,
    ) -> Result<(), String> {
        Err(format!("this world does not admit {tool}"))
    }

    fn terminal(
        &self,
        _request: &archon_permissions::SandboxTerminalRequest,
    ) -> archon_permissions::SandboxTerminal {
        archon_permissions::SandboxTerminal::Refused("this world hosts no shell".into())
    }

    fn scope_support(
        &self,
        _scope: archon_permissions::SandboxScope,
    ) -> archon_permissions::SandboxScopeSupport {
        archon_permissions::SandboxScopeSupport::Held
    }
}

/// A checker that permits the probe, so every refusal below comes from a layer
/// *after* the permission gate — which is the only interesting kind here.
fn permissive_checker() -> PermissionChecker {
    PermissionChecker::new(
        archon_permissions::mode::PermissionMode::default(),
        archon_permissions::rules::RuleSet {
            always_allow: vec![archon_permissions::rules::ToolRule {
                tool: "ScriptProbe".to_string(),
                pattern: "*".to_string(),
            }],
            always_deny: Vec::new(),
            always_ask: Vec::new(),
        },
    )
}

fn host_with(context: ToolContext, level: PermissionLevel) -> (ScriptToolHost, Arc<AtomicUsize>) {
    let runs = Arc::new(AtomicUsize::new(0));
    let mut registry = archon_core::dispatch::ToolRegistry::new();
    registry.register(Box::new(CountingTool {
        runs: Arc::clone(&runs),
        level,
    }));
    (
        ScriptToolHost {
            audited_writes: false,
            registry,
            checker: permissive_checker(),
            context,
        },
        runs,
    )
}

fn probe() -> RunToolRequest {
    RunToolRequest {
        name: "ScriptProbe".to_string(),
        input: serde_json::json!({"target": "the same thing every time"}),
    }
}

/// The bypass itself. An operator's blocked-tool decision reaches a
/// model-issued call through the admission callback; a script used to run the
/// same tool from the same registry without ever consulting it.
#[tokio::test]
async fn a_blocked_admission_refuses_a_scripts_tool_call() {
    let (host, runs) = host_with(
        ToolContext {
            tool_run_admission: Some(Arc::new(|_request| ToolRunAdmission::Blocked {
                reason: "the operator blocked this tool".to_string(),
            })),
            ..super::tests::context("script-admission-blocked")
        },
        // Admission is consulted only for non-`Safe` tools. That filter is
        // policy and predates this change; a `Safe` tool here would make the
        // test pass for the wrong reason.
        PermissionLevel::Risky,
    );

    let response = host
        .run("blocked#1", &probe())
        .await
        .expect("a blocked call is a failed result, not a failed run");

    assert!(response.is_error, "{response:?}");
    assert!(
        response.content.contains("the operator blocked this tool"),
        "the reason must reach the script verbatim: {response:?}"
    );
    assert_eq!(
        runs.load(Ordering::SeqCst),
        0,
        "the tool must not have run at all"
    );
}

/// Permission and confinement are different questions. The checker said yes;
/// the world still gets to say no, and only `dispatch` asks it.
#[tokio::test]
async fn a_sandbox_capability_denial_refuses_a_scripts_tool_call() {
    let (host, runs) = host_with(
        ToolContext {
            sandbox: Some(Arc::new(RefusingWorld)),
            ..super::tests::context("script-sandbox-denied")
        },
        PermissionLevel::Safe,
    );

    let response = host
        .run("sandboxed#1", &probe())
        .await
        .expect("a refused call is a failed result, not a failed run");

    assert!(response.is_error, "{response:?}");
    assert!(
        response.content.contains("does not admit ScriptProbe"),
        "the backend's own reason must survive: {response:?}"
    );
    assert_eq!(
        runs.load(Ordering::SeqCst),
        0,
        "the tool must not have run in a world that refused it"
    );
}

/// A script is a loop with no model in it to get bored, so it is the caller
/// most able to repeat itself — and until now the only one whose repetitions
/// were invisible to the guard.
#[tokio::test]
async fn identical_script_tool_calls_extend_the_repeat_tool_chain() {
    let context = super::tests::context("script-repeat-chain");
    assert!(
        context.repeat_tool.enabled,
        "this test relies on the documented default policy"
    );
    let key = ChainKey::of(&context);
    let (host, runs) = host_with(context, PermissionLevel::Safe);

    for attempt in 0..3 {
        host.run(&format!("repeat#{attempt}"), &probe())
            .await
            .expect("the probe runs");
    }

    assert_eq!(runs.load(Ordering::SeqCst), 3, "all three calls executed");
    let reminders = REPEAT_TOOL_CHAINS.take_reminders(&key);
    assert_eq!(reminders.len(), 1, "got {reminders:?}");
    assert!(
        reminders[0].contains("called ScriptProbe 3 times in a row"),
        "got: {}",
        reminders[0]
    );
}

/// Varying the arguments is genuine progress and must not be reported as a
/// stall — asserted here because the chain is now fed from this path, so this
/// path can also poison it.
#[tokio::test]
async fn differing_script_tool_calls_do_not_earn_a_reminder() {
    let context = super::tests::context("script-repeat-varied");
    let key = ChainKey::of(&context);
    let (host, _runs) = host_with(context, PermissionLevel::Safe);

    for attempt in 0..3 {
        host.run(
            &format!("varied#{attempt}"),
            &RunToolRequest {
                name: "ScriptProbe".to_string(),
                input: serde_json::json!({"target": attempt}),
            },
        )
        .await
        .expect("the probe runs");
    }

    assert!(
        REPEAT_TOOL_CHAINS.take_reminders(&key).is_empty(),
        "three different questions are not a repetition"
    );
}

/// The gate has to be installed on the context a real run uses, not only on
/// the ones a test hands in. Without this the three tests above would pass
/// against a production path whose `tool_run_admission` is permanently `None`
/// — a gate that is present, compiled, and never consulted.
#[test]
fn a_host_built_from_config_carries_the_admission_gate_and_the_repeat_policy() {
    let config = archon_core::config::load_config().expect("the archon config loads");
    let host = ScriptToolHost::new(std::env::temp_dir(), "script-host-wiring".to_string())
        .expect("the tool host builds from config");

    assert!(
        host.context.tool_run_admission.is_some(),
        "the blocked-tool gate must be installed"
    );
    assert!(
        host.context.tool_run_outcome.is_some(),
        "the outcome tap must be installed"
    );
    assert_eq!(
        host.context.tool_run_parent_action_id.as_deref(),
        Some("script-host-wiring")
    );
    assert!(
        host.context.activity_sink.is_some(),
        "the lifecycle emitters are guarded on this; without it they land nowhere"
    );
    assert_eq!(
        host.context.repeat_tool, config.guard.repeat_tool,
        "[guard.repeat_tool] must reach the script path, not RepeatToolConfig::default()"
    );
}
