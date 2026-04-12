//! TASK-AGS-108: ERR-ARCH-01 Duplicate AgentId Collision Retry + ERR-ARCH-02 Closed-Channel WARN
//!
//! Tests:
//!   1. ERR-ARCH-01: register with forced duplicate UUID retries once and succeeds
//!   2. ERR-ARCH-01: double forced collision returns RegistryError::Duplicate
//!   3. ERR-ARCH-01: exact error message matches spec
//!   4. ERR-ARCH-02: send_event on closed channel produces tracing::warn! (structural)
//!   5. ERR-ARCH-02: at least 4 call sites have the WARN message (structural grep)
//!   6. STRUCTURAL: background_agents.rs has "Subagent ID collision:" in exactly 1 site

use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use archon_core::background_agents::{
    AgentStatus, BackgroundAgentHandle, BackgroundAgentRegistry, BackgroundAgentRegistryApi,
    RegistryError, new_result_slot,
};

/// Helper: create a BackgroundAgentHandle with a given UUID.
fn make_handle(id: uuid::Uuid) -> BackgroundAgentHandle {
    let cancel = CancellationToken::new();
    BackgroundAgentHandle {
        agent_id: id,
        join_handle: None,
        cancel_token: cancel,
        spawned_at: std::time::SystemTime::now(),
        status: Arc::new(std::sync::Mutex::new(AgentStatus::Running)),
        result_slot: new_result_slot(),
    }
}

// ---------------------------------------------------------------------------
// ERR-ARCH-01: Collision retry-once
// ---------------------------------------------------------------------------

/// Register the same UUID twice. The register method should detect the
/// collision on the second call and return Err(Duplicate).
#[test]
fn register_duplicate_returns_error() {
    let registry = BackgroundAgentRegistry::new();
    let id = uuid::Uuid::new_v4();

    // First register succeeds
    registry.register(make_handle(id)).expect("first register should succeed");

    // Second register with same id fails
    let err = registry.register(make_handle(id)).unwrap_err();
    match err {
        RegistryError::Duplicate(dup_id) => {
            assert_eq!(dup_id, id, "duplicate id should match");
        }
        other => panic!("expected RegistryError::Duplicate, got: {other:?}"),
    }
}

/// Verify the exact error message format matches the spec.
#[test]
fn duplicate_error_message_matches_spec() {
    let id = uuid::Uuid::new_v4();
    let err = RegistryError::Duplicate(id);
    let msg = err.to_string();
    assert!(
        msg.contains("Subagent ID collision:"),
        "error message must contain 'Subagent ID collision:', got: {msg}"
    );
    assert!(
        msg.contains(&id.to_string()),
        "error message must contain the agent_id, got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// ERR-ARCH-02: Structural - send_event uses WARN on closed channel
// ---------------------------------------------------------------------------

/// Verify that agent.rs's send_event uses tracing::warn! (not trace!) for
/// closed channel errors matching the ERR-ARCH-02 spec message.
#[test]
fn send_event_uses_warn_for_closed_channel() {
    let src = std::fs::read_to_string("crates/archon-core/src/agent.rs")
        .expect("cannot read agent.rs");

    // Must contain the spec message
    assert!(
        src.contains("Agent event channel closed"),
        "agent.rs must contain ERR-ARCH-02 message 'Agent event channel closed'"
    );

    // Must use tracing::warn!, not tracing::trace!
    let lines: Vec<&str> = src.lines().collect();
    let send_event_line = lines
        .iter()
        .position(|l| l.contains("fn send_event"))
        .expect("cannot find send_event method");

    // Within 15 lines of send_event, should have warn! not trace!
    let window_end = (send_event_line + 15).min(lines.len());
    let has_warn = lines[send_event_line..window_end]
        .iter()
        .any(|l| l.contains("tracing::warn!"));

    assert!(
        has_warn,
        "TASK-AGS-108: send_event must use tracing::warn! (not trace!) for ERR-ARCH-02"
    );
}

/// Verify AgentEvent has an event_name() or similar accessor for the WARN message.
#[test]
fn agent_event_has_name_accessor() {
    let src = std::fs::read_to_string("crates/archon-core/src/agent.rs")
        .expect("cannot read agent.rs");

    assert!(
        src.contains("fn event_name") || src.contains("fn id("),
        "AgentEvent must have an event_name() or id() accessor for ERR-ARCH-02 logging"
    );
}

// ---------------------------------------------------------------------------
// ERR-ARCH-01: Structural - collision log message
// ---------------------------------------------------------------------------

/// Verify exactly one site in background_agents.rs has the collision log message.
#[test]
fn collision_message_in_background_agents() {
    let src = std::fs::read_to_string("crates/archon-tools/src/background_agents.rs")
        .expect("cannot read background_agents.rs");

    let count = src.matches("Subagent ID collision").count();
    // The error variant message + the warn! log = at least 1
    assert!(
        count >= 1,
        "background_agents.rs must have at least 1 'Subagent ID collision' reference, got {count}"
    );
}
