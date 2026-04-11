//! TASK-AGS-101: BackgroundAgentRegistry scaffold — integration tests.
//!
//! Exercises the public `BACKGROUND_AGENTS` singleton and the
//! `BackgroundAgentRegistryApi` contract. Written BEFORE the impl
//! (Gate 1) so this file will fail to compile until
//! `crates/archon-core/src/background_agents.rs` is created and
//! wired through `crates/archon-core/src/lib.rs`.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use archon_tools::background_agents::{
    self, AgentStatus, BackgroundAgentHandle, BackgroundAgentRegistry,
    BackgroundAgentRegistryApi, RegistryError, BACKGROUND_AGENTS,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Build a minimal handle whose spawned task simply waits on the
/// cancel token so the tests can drive state transitions.
fn make_handle(status: AgentStatus) -> BackgroundAgentHandle {
    let cancel = CancellationToken::new();
    let cancel_child = cancel.clone();
    let result_slot = background_agents::new_result_slot();
    let result_slot_child = Arc::clone(&result_slot);
    let status_cell = Arc::new(std::sync::Mutex::new(status));
    let status_child = Arc::clone(&status_cell);

    let join = tokio::spawn(async move {
        cancel_child.cancelled().await;
        *status_child.lock().unwrap() = AgentStatus::Cancelled;
        *result_slot_child.lock().unwrap() = Some(Err("cancelled".into()));
    });

    BackgroundAgentHandle {
        agent_id: Uuid::new_v4(),
        join_handle: Some(join),
        cancel_token: cancel,
        spawned_at: SystemTime::now(),
        status: status_cell,
        result_slot,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn register_then_get_round_trips() {
    let registry = BackgroundAgentRegistry::new();
    let handle = make_handle(AgentStatus::Running);
    let id = handle.agent_id;

    registry.register(handle).expect("register must succeed");
    assert_eq!(registry.get(&id), Some(AgentStatus::Running));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_register_returns_duplicate_error() {
    let registry = BackgroundAgentRegistry::new();
    let handle1 = make_handle(AgentStatus::Running);
    let id = handle1.agent_id;
    registry.register(handle1).expect("first register ok");

    // Fabricate a second handle reusing the same id.
    let mut handle2 = make_handle(AgentStatus::Running);
    handle2.agent_id = id;
    let err = registry
        .register(handle2)
        .expect_err("duplicate register must fail");
    assert!(matches!(err, RegistryError::Duplicate(dup) if dup == id));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_fires_cancel_token_and_transitions_status() {
    let registry = BackgroundAgentRegistry::new();
    let handle = make_handle(AgentStatus::Running);
    let id = handle.agent_id;
    let token_clone = handle.cancel_token.clone();
    registry.register(handle).expect("register ok");

    registry.cancel(&id).expect("cancel ok");
    assert!(token_clone.is_cancelled(), "cancel_token must be fired");

    // Allow the spawned task a moment to settle.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // poll_status should show Cancelled (task flipped its own status).
    let status = registry.poll_status(&id);
    assert_eq!(status, Some(AgentStatus::Cancelled));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn iter_running_excludes_finished_handles() {
    let registry = BackgroundAgentRegistry::new();
    let running = make_handle(AgentStatus::Running);
    let finished = make_handle(AgentStatus::Finished);
    let running_id = running.agent_id;
    let finished_id = finished.agent_id;
    registry.register(running).expect("register running");
    registry.register(finished).expect("register finished");

    let live = registry.iter_running();
    assert!(live.contains(&running_id), "iter_running must include Running handle");
    assert!(
        !live.contains(&finished_id),
        "iter_running must exclude Finished handle"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reap_finished_removes_terminal_handles() {
    let registry = BackgroundAgentRegistry::new();
    let running = make_handle(AgentStatus::Running);
    let finished = make_handle(AgentStatus::Finished);
    let failed = make_handle(AgentStatus::Failed);

    let running_id = running.agent_id;
    let finished_id = finished.agent_id;
    let failed_id = failed.agent_id;

    registry.register(running).unwrap();
    registry.register(finished).unwrap();
    registry.register(failed).unwrap();

    let reaped = registry.reap_finished();
    assert!(reaped.contains(&finished_id));
    assert!(reaped.contains(&failed_id));
    assert!(!reaped.contains(&running_id));

    // Reaped handles are gone; running one remains.
    assert_eq!(registry.get(&running_id), Some(AgentStatus::Running));
    assert_eq!(registry.get(&finished_id), None);
    assert_eq!(registry.get(&failed_id), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_unknown_id_returns_not_found() {
    let registry = BackgroundAgentRegistry::new();
    let err = registry.cancel(&Uuid::new_v4()).expect_err("unknown id");
    assert!(matches!(err, RegistryError::NotFound(_)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_singleton_is_usable_across_references() {
    // BACKGROUND_AGENTS is the global Lazy<Arc<dyn ...>>. Clone-and-use.
    let global: Arc<dyn BackgroundAgentRegistryApi> = Arc::clone(&*BACKGROUND_AGENTS);
    let handle = make_handle(AgentStatus::Running);
    let id = handle.agent_id;
    global.register(handle).expect("global register ok");
    assert_eq!(global.get(&id), Some(AgentStatus::Running));
    // Cleanup so other tests on the singleton don't see stale handles.
    let _ = global.cancel(&id);
}
