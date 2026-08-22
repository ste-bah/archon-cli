//! Tests for the per-tool call budget enforced in [`execute_within_budget`].
//!
//! These drive the real dispatch path (`ToolRegistry::dispatch` →
//! `execute_tool_attempt`) rather than calling the helper directly, so a
//! regression that reconnects the choke point to a bare `tool.execute(..)`
//! fails here.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use archon_observability::{AgentActivityKind, AgentActivityStatus, InMemoryActivitySink};
use archon_tools::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

use crate::dispatch::{ToolRegistry, create_default_registry};

/// A tool whose call takes `runtime` and which declares `budget`.
///
/// `dropped` flips when the body's guard is destroyed, which only happens if
/// the future was cancelled part-way through — that is the property the whole
/// design rests on and the reason opting in is a per-tool decision.
struct SlowTool {
    name: &'static str,
    budget: Option<Duration>,
    runtime: Duration,
    executions: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
}

struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl Tool for SlowTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "tool-call budget test"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    /// This double stands in for the tools a budget is declared on — the
    /// network- and IPC-bound ones — every one of which runs somewhere. The
    /// method is required rather than defaulted so a tool cannot reach dispatch
    /// without having answered which world it belongs to (#201).
    fn capability(&self) -> archon_permissions::ToolCapability {
        archon_permissions::ToolCapability::EXECUTION
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        self.executions.fetch_add(1, Ordering::SeqCst);
        let guard = DropFlag(Arc::clone(&self.dropped));
        tokio::time::sleep(self.runtime).await;
        drop(guard);
        ToolResult::success("slow tool finished")
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }

    fn timeout(&self) -> Option<Duration> {
        self.budget
    }
}

fn slow_tool(
    name: &'static str,
    budget: Option<Duration>,
    runtime: Duration,
) -> (Box<SlowTool>, Arc<AtomicUsize>, Arc<AtomicBool>) {
    let executions = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicBool::new(false));
    let tool = Box::new(SlowTool {
        name,
        budget,
        runtime,
        executions: Arc::clone(&executions),
        dropped: Arc::clone(&dropped),
    });
    (tool, executions, dropped)
}

fn ctx_with_sink(sink: Arc<InMemoryActivitySink>) -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "tool-budget-test".to_string(),
        activity_sink: Some(sink),
        ..Default::default()
    }
}

#[tokio::test]
async fn declared_budget_bounds_the_call_and_reports_an_ordinary_error() {
    let (tool, executions, dropped) = slow_tool(
        "SlowBudgeted",
        Some(Duration::from_millis(50)),
        Duration::from_secs(30),
    );
    let mut registry = ToolRegistry::new();
    registry.register(tool);

    let started = std::time::Instant::now();
    let result = registry
        .dispatch(
            "SlowBudgeted",
            serde_json::json!({}),
            &ToolContext::default(),
        )
        .await;
    let elapsed = started.elapsed();

    assert!(result.is_error, "expected a timeout error, got {result:?}");
    assert!(
        result.content.contains("SlowBudgeted timed out after 50ms"),
        "timeout message must name the tool and the budget: {}",
        result.content
    );
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    assert!(
        dropped.load(Ordering::SeqCst),
        "the overrunning call must have been dropped at the deadline"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "dispatch waited {elapsed:?} on a 50ms budget"
    );
}

#[tokio::test]
async fn the_turn_continues_after_a_timeout() {
    let (slow, _, _) = slow_tool(
        "SlowBudgeted",
        Some(Duration::from_millis(50)),
        Duration::from_secs(30),
    );
    let (fast, fast_executions, _) = slow_tool("FastUnbudgeted", None, Duration::from_millis(1));
    let mut registry = ToolRegistry::new();
    registry.register(slow);
    registry.register(fast);
    let ctx = ToolContext::default();

    let timed_out = registry
        .dispatch("SlowBudgeted", serde_json::json!({}), &ctx)
        .await;
    let after = registry
        .dispatch("FastUnbudgeted", serde_json::json!({}), &ctx)
        .await;

    assert!(timed_out.is_error);
    assert!(!after.is_error, "dispatch after a timeout: {after:?}");
    assert_eq!(after.content, "slow tool finished");
    assert_eq!(fast_executions.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_tool_declaring_no_budget_is_left_alone() {
    // 400ms is well past any budget the opted-in tools declare and past the
    // 50ms used above; if the wrapper ever applied to `None` this would fail.
    let (tool, executions, dropped) = slow_tool("Unbudgeted", None, Duration::from_millis(400));
    let mut registry = ToolRegistry::new();
    registry.register(tool);

    let result = registry
        .dispatch("Unbudgeted", serde_json::json!({}), &ToolContext::default())
        .await;

    assert!(!result.is_error, "{result:?}");
    assert_eq!(result.content, "slow tool finished");
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    assert!(
        dropped.load(Ordering::SeqCst),
        "the guard is dropped on the success path too"
    );
}

#[tokio::test]
async fn a_budgeted_tool_finishing_in_time_returns_its_own_result() {
    let (tool, _, _) = slow_tool(
        "FitsInBudget",
        Some(Duration::from_secs(30)),
        Duration::from_millis(1),
    );
    let mut registry = ToolRegistry::new();
    registry.register(tool);

    let result = registry
        .dispatch(
            "FitsInBudget",
            serde_json::json!({}),
            &ToolContext::default(),
        )
        .await;

    assert!(!result.is_error, "{result:?}");
    assert_eq!(result.content, "slow tool finished");
}

#[tokio::test]
async fn the_timeout_path_emits_the_same_activity_events_as_a_completion() {
    let (slow, _, _) = slow_tool(
        "SlowBudgeted",
        Some(Duration::from_millis(50)),
        Duration::from_secs(30),
    );
    let mut registry = ToolRegistry::new();
    registry.register(slow);
    let sink = Arc::new(InMemoryActivitySink::new());

    let result = registry
        .dispatch(
            "SlowBudgeted",
            serde_json::json!({}),
            &ctx_with_sink(Arc::clone(&sink)),
        )
        .await;

    assert!(result.is_error);
    let events = sink.events();
    let kinds: Vec<_> = events.iter().map(|event| event.kind).collect();
    assert_eq!(
        kinds,
        vec![
            AgentActivityKind::ToolStarted,
            AgentActivityKind::ToolFailed
        ],
        "a timeout is reported through the ordinary tool-result activity pair"
    );
    assert_eq!(events[1].status, AgentActivityStatus::Failed);
    assert!(
        events[1].message.contains("SlowBudgeted elapsed="),
        "elapsed time must be recorded on the timeout path: {}",
        events[1].message
    );
}

/// `Bash` enforces its own deadline in `bash_process.rs`, where it can kill the
/// child it spawned and where the caller can shorten the limit per command. A
/// budget declared here would give one command two clocks, the shorter of which
/// the model cannot see or adjust.
#[test]
fn bash_declares_no_budget_so_it_is_not_double_wrapped() {
    let registry = create_default_registry(std::env::temp_dir(), None);
    let bash = registry.get("Bash").expect("Bash is registered");
    assert!(
        bash.timeout().is_none(),
        "Bash must keep its own deadline as the only one"
    );
}

/// The long-running-by-design tools must stay unbounded: a budget on them would
/// kill work that is behaving correctly.
#[test]
fn long_running_tools_declare_no_budget() {
    let registry = create_default_registry(std::env::temp_dir(), None);
    for name in ["Agent", "TaskCreate"] {
        let Some(tool) = registry.get(name) else {
            continue;
        };
        assert!(
            tool.timeout().is_none(),
            "{name} is long-running by design and must not declare a budget"
        );
    }
}

#[test]
fn network_bound_tools_declare_a_budget() {
    let registry = create_default_registry(std::env::temp_dir(), None);
    for name in ["WebFetch", "WebSearch", "lsp"] {
        let tool = registry.get(name).unwrap_or_else(|| {
            panic!("{name} is expected in the default registry");
        });
        assert!(
            tool.timeout().is_some(),
            "{name} is network- or IPC-bound and must declare a budget"
        );
    }
}
