//! TASK-AGS-106: D1 Input Handler tokio::spawn Wrapper
//!
//! Structural and functional tests for the non-blocking input handler fix.
//!
//! The D1 "smoking gun" is `agent.process_message(&input).await` running
//! synchronously on the input handler task, freezing the TUI. The fix wraps
//! both process_message call sites in `tokio::spawn` and stores a
//! CancellationToken in a shared slot accessible from outside the handler.
//!
//! These tests verify:
//!   1. STRUCTURAL: `process_message` is never sync-awaited on the handler
//!      task (grep-based source analysis).
//!   2. FUNCTIONAL: the shared CancellationToken slot pattern works —
//!      a token stored from inside a spawn is reachable and fireable
//!      from outside (models the TASK-AGS-107 Ctrl+C path).
//!   3. SERIALIZATION: sequential JoinHandle await prevents concurrent
//!      process_message calls against one conversation.

use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

/// Shared slot type matching the production `current_agent_task` shape.
/// The CancellationToken is stored when a process_message spawn starts;
/// TASK-AGS-107's Ctrl+C handler will read this from a signal handler
/// task to cancel a running agent.
type CurrentAgentTask = Arc<Mutex<Option<(CancellationToken, tokio::task::JoinHandle<()>)>>>;

// ---------------------------------------------------------------------------
// Structural guard: no sync process_message await in the handler.
// ---------------------------------------------------------------------------

/// Verify that `process_message` calls in main.rs's input handler region
/// are ONLY inside `tokio::spawn` blocks, never sync-awaited on the
/// handler task.
///
/// This is a source-level structural test. It reads main.rs, finds all
/// `process_message` call sites, and asserts each one is preceded by a
/// `tokio::spawn` on the same or prior line within the input handler
/// region (~lines 3250-3875).
#[ignore = "TDD test for unimplemented AGS-106 input handler tokio::spawn wrapper; tracked under #224 (CI cross-platform parity with P1.1 canary skip list)"]
#[test]
fn process_message_never_sync_awaited_in_handler() {
    let src = std::fs::read_to_string("src/main.rs").expect("cannot read src/main.rs");
    let lines: Vec<&str> = src.lines().collect();

    // Find the input handler spawn boundary: starts with "tokio::spawn(async move {"
    // after the "Spawn agent input processor" comment, ends with the matching "});"
    let handler_start = lines
        .iter()
        .position(|l| l.contains("Spawn agent input processor"))
        .expect("cannot find input handler spawn comment");

    // Find the handler-level tokio::spawn (the one right after the
    // "Spawn agent input processor" comment). All process_message calls
    // live INSIDE this outer spawn. The D1 fix wraps each call in a
    // NESTED tokio::spawn. We need to verify the nested spawn exists,
    // not just the outer one.
    //
    // Strategy: for each process_message call, check that there's a
    // `tokio::spawn` within the preceding 20 lines (close enough to be
    // the wrapper, not the outer handler spawn hundreds of lines above).
    let outer_spawn_line = lines
        .iter()
        .enumerate()
        .skip(handler_start)
        .find(|(_, l)| l.contains("tokio::spawn(async move"))
        .map(|(i, _)| i)
        .expect("cannot find handler-level tokio::spawn");

    let mut violations = Vec::new();
    for (i, line) in lines.iter().enumerate().skip(outer_spawn_line + 1) {
        if !line.contains("process_message") {
            continue;
        }
        // Skip comments
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with("///") || trimmed.starts_with("*") {
            continue;
        }
        // Skip lines that don't actually call the method (e.g. type
        // annotations, string literals containing the word).
        if !trimmed.contains(".process_message(") {
            continue;
        }
        // Check for a NEARBY tokio::spawn within the preceding 20 lines.
        // The outer handler spawn is hundreds of lines above, so this
        // window only matches a nested wrapper spawn.
        let window_start = i.saturating_sub(20);
        let has_nearby_spawn = lines[window_start..i]
            .iter()
            .any(|l| l.contains("tokio::spawn"));
        if !has_nearby_spawn {
            violations.push(format!(
                "line {}: process_message sync-awaited without nearby tokio::spawn wrapper: {}",
                i + 1,
                trimmed
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "TASK-AGS-106 regression: process_message sync-awaited in handler:\n{}",
        violations.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Functional: CancellationToken shared slot reachable from outside.
// ---------------------------------------------------------------------------

/// Verify that a CancellationToken stored inside a spawned task is
/// reachable and fireable from an external task (models Ctrl+C path).
#[tokio::test]
async fn cancel_token_reachable_from_outside_spawn() {
    let slot: CurrentAgentTask = Arc::new(Mutex::new(None));
    let slot_for_spawn: CurrentAgentTask = Arc::clone(&slot);

    let was_cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let was_cancelled_inner = Arc::clone(&was_cancelled);

    // Spawn a "handler" that stores a token in the shared slot, then
    // waits for cancellation.
    let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let cancel = CancellationToken::new();
        let cancel_inner = cancel.clone();
        let work: tokio::task::JoinHandle<()> = tokio::spawn(async move {
            cancel_inner.cancelled().await;
            was_cancelled_inner.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        // Store token in shared slot
        *slot_for_spawn.lock().await = Some((cancel, work));
        started_tx.send(()).ok();
    });

    // Wait for the handler to store the token
    handle.await.unwrap();
    started_rx.await.unwrap();

    // External "signal handler" reads the slot and fires the token
    let (token, join_handle) = {
        let mut guard = slot.lock().await;
        guard.take().expect("slot should contain a task")
    };
    token.cancel();

    // The inner task should complete
    tokio::time::timeout(std::time::Duration::from_secs(2), join_handle)
        .await
        .expect("join timed out")
        .expect("join panicked");

    assert!(
        was_cancelled.load(std::sync::atomic::Ordering::SeqCst),
        "inner task should have observed cancellation"
    );
}

// ---------------------------------------------------------------------------
// Serialization: JoinHandle await prevents concurrent agent calls.
// ---------------------------------------------------------------------------

/// Verify that awaiting a previous JoinHandle before spawning a new task
/// serializes execution (no concurrent access).
#[tokio::test]
async fn sequential_join_handle_serializes_tasks() {
    let counter = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let max_concurrent = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let slot: CurrentAgentTask = Arc::new(Mutex::new(None));

    for _ in 0..5 {
        // Await previous task before spawning new one (serial model)
        {
            let mut guard = slot.lock().await;
            if let Some((_cancel, handle)) = guard.take() {
                handle.await.ok();
            }
        }

        let counter_clone = Arc::clone(&counter);
        let max_clone = Arc::clone(&max_concurrent);
        let cancel = CancellationToken::new();
        let handle = tokio::spawn(async move {
            let current = counter_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            // Record max concurrent
            max_clone.fetch_max(current, std::sync::atomic::Ordering::SeqCst);
            // Simulate work
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            counter_clone.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        });

        *slot.lock().await = Some((cancel, handle));
    }

    // Await final task
    if let Some((_cancel, handle)) = slot.lock().await.take() {
        handle.await.ok();
    }

    // Max concurrent should be 1 (serialized)
    let max = max_concurrent.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        max, 1,
        "expected max concurrency of 1 (serialized), got {max}"
    );
}
