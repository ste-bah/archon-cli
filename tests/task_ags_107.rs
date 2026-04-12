//! TASK-AGS-107: Ctrl+C → CancellationToken End-to-End Wiring
//!
//! Tests for the Ctrl+C cancellation path:
//!   1. STRUCTURAL: TUI Ctrl+C handler sends `__cancel__` when is_generating
//!   2. STRUCTURAL: main.rs input handler recognizes `__cancel__` and fires token
//!   3. STRUCTURAL: ToolContext has `cancel_parent` field
//!   4. FUNCTIONAL: parent CancellationToken.cancel() propagates to child_token()
//!   5. FUNCTIONAL: cancel fires within the shared slot pattern from AGS-106
//!   6. FUNCTIONAL: BACKGROUND_AGENTS.cancel() flips status to Cancelled

use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

/// Shared slot type matching AGS-106's `current_agent_task` shape.
type CurrentAgentTask = Arc<Mutex<Option<(CancellationToken, tokio::task::JoinHandle<()>)>>>;

// ---------------------------------------------------------------------------
// Structural: TUI Ctrl+C sends __cancel__ control message
// ---------------------------------------------------------------------------

/// Verify that TUI's Ctrl+C handler sends `__cancel__` via input_tx
/// when `is_generating` is true.
#[test]
fn tui_ctrl_c_sends_cancel_control_message() {
    let src = std::fs::read_to_string("crates/archon-tui/src/app.rs")
        .expect("cannot read app.rs");

    // Find the Ctrl+C handler block
    let has_ctrl_c_cancel = src.contains("__cancel__");
    assert!(
        has_ctrl_c_cancel,
        "TASK-AGS-107: TUI Ctrl+C handler must send '__cancel__' control message \
         via input_tx when is_generating is true"
    );

    // Verify it's inside the Ctrl+C key handler (the one that also
    // checks KeyModifiers::CONTROL on a nearby line).
    let lines: Vec<&str> = src.lines().collect();
    // Find the Ctrl+C key handler: look for "Ctrl+C = interrupt" comment
    let ctrl_c_line = lines
        .iter()
        .position(|l| l.contains("Ctrl+C = interrupt"))
        .expect("cannot find Ctrl+C handler comment");

    // __cancel__ should appear within 25 lines of the Ctrl+C handler
    let window_end = (ctrl_c_line + 25).min(lines.len());
    let has_cancel_in_handler = lines[ctrl_c_line..window_end]
        .iter()
        .any(|l| l.contains("__cancel__"));

    assert!(
        has_cancel_in_handler,
        "TASK-AGS-107: '__cancel__' must be sent from within the Ctrl+C key handler, \
         not elsewhere in app.rs"
    );
}

// ---------------------------------------------------------------------------
// Structural: main.rs recognizes __cancel__ and fires token
// ---------------------------------------------------------------------------

/// Verify that the input handler in main.rs recognizes the `__cancel__`
/// control message and calls `.cancel()` on the stored CancellationToken.
#[test]
fn handler_recognizes_cancel_control_message() {
    let src = std::fs::read_to_string("src/main.rs")
        .expect("cannot read src/main.rs");
    let lines: Vec<&str> = src.lines().collect();

    // Find the input handler loop
    let handler_start = lines
        .iter()
        .position(|l| l.contains("Spawn agent input processor"))
        .expect("cannot find input handler");

    // __cancel__ recognition must exist inside the handler
    let has_cancel_check = lines[handler_start..]
        .iter()
        .any(|l| l.contains("__cancel__"));

    assert!(
        has_cancel_check,
        "TASK-AGS-107: input handler must check for '__cancel__' control message"
    );

    // The cancel block should call .cancel() on the token
    let cancel_line = lines[handler_start..]
        .iter()
        .position(|l| l.contains("__cancel__"))
        .map(|i| i + handler_start)
        .expect("cannot find __cancel__ handler");

    // Within 15 lines of __cancel__ check, should call .cancel()
    let window_end = (cancel_line + 15).min(lines.len());
    let fires_token = lines[cancel_line..window_end]
        .iter()
        .any(|l| l.contains(".cancel()"));

    assert!(
        fires_token,
        "TASK-AGS-107: handler must call .cancel() on the CancellationToken \
         within the __cancel__ block"
    );
}

// ---------------------------------------------------------------------------
// Structural: ToolContext has cancel_parent field
// ---------------------------------------------------------------------------

/// Verify that ToolContext has a `cancel_parent` field.
#[test]
fn tool_context_has_cancel_parent_field() {
    let src = std::fs::read_to_string("crates/archon-tools/src/tool.rs")
        .expect("cannot read tool.rs");

    assert!(
        src.contains("cancel_parent"),
        "TASK-AGS-107: ToolContext must have a cancel_parent field \
         for propagating CancellationToken through the tool chain"
    );
}

// ---------------------------------------------------------------------------
// Structural: agent_tool.rs propagates cancel_parent through ToolContext
// ---------------------------------------------------------------------------

/// Verify that AgentTool::execute reads cancel_parent from ctx and
/// uses it (via child_token or clone) when creating the CancellationToken
/// for run_subagent.
#[test]
fn agent_tool_propagates_cancel_parent() {
    let src = std::fs::read_to_string("crates/archon-tools/src/agent_tool.rs")
        .expect("cannot read agent_tool.rs");

    // cancel_parent must be referenced in agent_tool.rs
    assert!(
        src.contains("cancel_parent"),
        "TASK-AGS-107: agent_tool.rs must reference cancel_parent from ToolContext \
         to propagate cancellation into run_subagent"
    );

    // child_token() should be used for cascading cancellation
    assert!(
        src.contains("child_token()"),
        "TASK-AGS-107: agent_tool.rs must use child_token() to create a \
         cascading CancellationToken for the subagent"
    );
}

// ---------------------------------------------------------------------------
// Functional: parent cancel propagates to child tokens
// ---------------------------------------------------------------------------

/// Verify that cancelling a parent CancellationToken cascades to
/// all child tokens created via child_token().
#[tokio::test]
async fn parent_cancel_cascades_to_children() {
    let parent = CancellationToken::new();
    let child1 = parent.child_token();
    let child2 = parent.child_token();
    let grandchild = child1.child_token();

    // None should be cancelled yet
    assert!(!parent.is_cancelled());
    assert!(!child1.is_cancelled());
    assert!(!child2.is_cancelled());
    assert!(!grandchild.is_cancelled());

    // Cancel parent
    parent.cancel();

    // All descendants must be cancelled
    assert!(child1.is_cancelled(), "child1 must be cancelled");
    assert!(child2.is_cancelled(), "child2 must be cancelled");
    assert!(grandchild.is_cancelled(), "grandchild must be cancelled");
}

// ---------------------------------------------------------------------------
// Functional: __cancel__ fires token in shared slot
// ---------------------------------------------------------------------------

/// Simulate the __cancel__ → token.cancel() path:
/// 1. Store a CancellationToken in the shared slot (AGS-106 pattern)
/// 2. Simulate receiving "__cancel__" by firing the token
/// 3. Verify the spawned work observes cancellation
#[tokio::test]
async fn cancel_control_message_fires_slot_token() {
    let slot: CurrentAgentTask = Arc::new(Mutex::new(None));
    let was_cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));

    // Spawn work that waits for cancellation
    let cancel = CancellationToken::new();
    let cancel_inner = cancel.clone();
    let was_cancelled_inner = Arc::clone(&was_cancelled);
    let handle = tokio::spawn(async move {
        cancel_inner.cancelled().await;
        was_cancelled_inner.store(true, std::sync::atomic::Ordering::SeqCst);
    });

    // Store in slot (mimics AGS-106 post-spawn)
    *slot.lock().await = Some((cancel, handle));

    // Simulate __cancel__ handler: read slot, fire token
    {
        let guard = slot.lock().await;
        if let Some((ref token, _)) = *guard {
            token.cancel();
        }
    }

    // Wait for the spawned work to observe cancellation
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    assert!(
        was_cancelled.load(std::sync::atomic::Ordering::SeqCst),
        "spawned work should have observed cancellation after __cancel__"
    );
}

// ---------------------------------------------------------------------------
// Functional: cancel propagates through nested subagent tokens
// ---------------------------------------------------------------------------

/// Model the cancel propagation chain:
///   input_handler → current_agent_task token → child_token in ToolContext
///   → child_token in run_subagent → child_token per nested subagent
///
/// Cancelling the top-level token must cancel all 3 nested levels.
#[tokio::test]
async fn cancel_propagates_through_three_levels() {
    let top = CancellationToken::new();

    // Level 1: process_message spawn token (stored in current_agent_task)
    let level1 = top.child_token();

    // Level 2: AgentTool::execute creates child for run_subagent
    let level2 = level1.child_token();

    // Level 3: nested subagent inside run_subagent
    let level3 = level2.child_token();

    // Track cancellation at each level
    let l1_cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let l2_cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let l3_cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let l1c = Arc::clone(&l1_cancelled);
    let l2c = Arc::clone(&l2_cancelled);
    let l3c = Arc::clone(&l3_cancelled);

    let h1 = tokio::spawn(async move {
        level1.cancelled().await;
        l1c.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    let h2 = tokio::spawn(async move {
        level2.cancelled().await;
        l2c.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    let h3 = tokio::spawn(async move {
        level3.cancelled().await;
        l3c.store(true, std::sync::atomic::Ordering::SeqCst);
    });

    // Fire the top-level cancel (simulates Ctrl+C handler)
    top.cancel();

    // All handles should complete quickly
    let join_all = async {
        let _ = h1.await;
        let _ = h2.await;
        let _ = h3.await;
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), join_all)
        .await
        .expect("joins timed out — cancel did not propagate");

    assert!(l1_cancelled.load(std::sync::atomic::Ordering::SeqCst), "level 1 not cancelled");
    assert!(l2_cancelled.load(std::sync::atomic::Ordering::SeqCst), "level 2 not cancelled");
    assert!(l3_cancelled.load(std::sync::atomic::Ordering::SeqCst), "level 3 not cancelled");
}

// ---------------------------------------------------------------------------
// Functional: BACKGROUND_AGENTS.cancel() flips status
// ---------------------------------------------------------------------------

/// Verify that BackgroundAgentRegistry::cancel() flips status to Cancelled
/// and fires the CancellationToken.
#[tokio::test]
async fn background_agents_cancel_flips_status() {
    use archon_core::background_agents::{
        AgentStatus, BackgroundAgentHandle, BackgroundAgentRegistryApi,
        BackgroundAgentRegistry, new_result_slot,
    };

    let registry = BackgroundAgentRegistry::new();
    let cancel = CancellationToken::new();
    let cancel_check = cancel.clone();
    let agent_id = uuid::Uuid::new_v4();

    let handle = BackgroundAgentHandle {
        agent_id,
        join_handle: Some(tokio::runtime::Handle::current().spawn(async {
            // Long-running work that waits for cancel
            tokio::time::sleep(std::time::Duration::from_secs(300)).await;
        })),
        cancel_token: cancel,
        spawned_at: std::time::SystemTime::now(),
        status: Arc::new(std::sync::Mutex::new(AgentStatus::Running)),
        result_slot: new_result_slot(),
    };

    registry.register(handle).expect("register failed");

    // Cancel
    registry.cancel(&agent_id).expect("cancel failed");

    // Status must be Cancelled
    let status = registry.get(&agent_id).expect("agent not found");
    assert_eq!(
        status,
        AgentStatus::Cancelled,
        "status should be Cancelled after cancel()"
    );

    // Token must be fired
    assert!(
        cancel_check.is_cancelled(),
        "CancellationToken should be cancelled after registry.cancel()"
    );
}
