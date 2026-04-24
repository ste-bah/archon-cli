//! TASK-AGS-101: BackgroundAgentRegistry scaffold (REQ-FOR-D2 [1/5]).
//!
//! Global DashMap-backed registry that owns the `JoinHandle` +
//! `CancellationToken` for every background subagent spawned by
//! `AgentTool::execute` (TASK-AGS-104/105 replace the legacy
//! `agent.rs:2939-2977` spawn site with a redirect to this
//! registry). This module is intentionally a scaffold: it compiles
//! clean, provides the complete public API contract from the
//! TECH-AGS-ARCH-FIXES technical spec (data_models +
//! component_contracts), and backs the operations with an in-memory
//! DashMap. It is NOT yet wired into any spawn site — that is
//! deferred to TASK-AGS-104 and TASK-AGS-105.
//!
//! Rule 3 of the D10 philosophy
//! (`docs/architecture/spawn-everything-philosophy.md`) —
//! *"tools own task lifecycle"* — requires every spawned
//! subagent to register its handle in `BACKGROUND_AGENTS`
//! synchronously so that upper layers can poll status, trigger
//! cancellation, and reap terminal handles without holding locks
//! on the agent loop.
//!
//! The `RegistryError::Duplicate` variant exists for TASK-AGS-108
//! (ERR-ARCH-01) which adds the collision retry policy. For now,
//! `register` simply surfaces the duplicate as an error.

use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use dashmap::DashMap;
use once_cell::sync::Lazy;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Stable alias for the agent UUID used throughout the registry.
pub type AgentId = Uuid;

/// Lifecycle state of a tracked background agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    /// Task is still executing.
    Running,
    /// Task completed successfully (result available in `result_slot`).
    Finished,
    /// Task failed with an error (error string in `result_slot`).
    Failed,
    /// Task was cancelled via `CancellationToken`.
    Cancelled,
}

impl AgentStatus {
    /// `true` iff the status is terminal (Finished, Failed, Cancelled).
    pub fn is_terminal(self) -> bool {
        !matches!(self, AgentStatus::Running)
    }
}

/// Shared result slot. The spawned task writes exactly once; the
/// registry owns the only clone other than the task itself. Uses
/// `std::sync::Mutex` (not `tokio::sync::Mutex`) so `iter_running`
/// and `reap_finished` stay cheap and non-async.
pub type ResultSlot = Arc<Mutex<Option<Result<String, String>>>>;

/// Factory for a fresh empty result slot (convenience for call sites
/// and tests — avoids leaking the `Mutex` wrapper type).
pub fn new_result_slot() -> ResultSlot {
    Arc::new(Mutex::new(None))
}

/// Per-subagent handle stored in the registry. Fields match
/// TECH-AGS-ARCH-FIXES `data_models` section exactly.
pub struct BackgroundAgentHandle {
    pub agent_id: AgentId,
    /// `None` once the handle has been taken for awaiting; otherwise
    /// the live `JoinHandle` for the spawned task.
    pub join_handle: Option<JoinHandle<()>>,
    pub cancel_token: CancellationToken,
    pub spawned_at: SystemTime,
    pub status: Arc<Mutex<AgentStatus>>,
    pub result_slot: ResultSlot,
}

impl BackgroundAgentHandle {
    /// Snapshot the current status without holding the DashMap lock.
    pub fn current_status(&self) -> AgentStatus {
        *self.status.lock().expect("status mutex poisoned")
    }
}

/// Observability events emitted by the registry. Wired to a metrics
/// channel by `BackgroundAgentRegistry::with_metrics`; `None` for
/// the default singleton used at boot.
#[derive(Debug, Clone)]
pub enum RegistryEvent {
    Registered(AgentId),
    Cancelled(AgentId),
    Reaped(AgentId, AgentStatus),
}

/// Registry-level errors. `Duplicate` is the ERR-ARCH-01 variant
/// surfaced by `register`; TASK-AGS-108 wraps it with a retry policy.
/// `Closed` is reserved for the metrics-channel-dropped case.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("Subagent ID collision: {0} already registered")]
    Duplicate(AgentId),
    #[error("Subagent not found: {0}")]
    NotFound(AgentId),
    #[error("registry metrics channel closed")]
    Closed,
}

/// Public contract for the background-agents registry. Defined as
/// a trait so the global singleton can be replaced with a mock in
/// tests, and so upper layers take `Arc<dyn BackgroundAgentRegistryApi>`
/// instead of a concrete type.
pub trait BackgroundAgentRegistryApi: Send + Sync {
    /// Insert a handle. Returns `Duplicate(id)` if the id already exists.
    fn register(&self, handle: BackgroundAgentHandle) -> Result<(), RegistryError>;

    /// Return the current status of a registered handle, or `None`
    /// if the id is not (or no longer) in the registry.
    fn get(&self, id: &AgentId) -> Option<AgentStatus>;

    /// Fire the `CancellationToken` for a registered handle and flag
    /// its status as `Cancelled`. Does NOT remove the entry — that is
    /// `reap_finished`'s job once the spawned task actually exits.
    fn cancel(&self, id: &AgentId) -> Result<(), RegistryError>;

    /// Same semantics as `get`, kept as a separate method to match
    /// the six-method contract in TECH-AGS-ARCH-FIXES component_contracts.
    fn poll_status(&self, id: &AgentId) -> Option<AgentStatus>;

    /// Remove every handle whose status is terminal (Finished, Failed,
    /// Cancelled) and return the ids that were removed.
    fn reap_finished(&self) -> Vec<AgentId>;

    /// Return the ids of every handle whose status is still `Running`.
    fn iter_running(&self) -> Vec<AgentId>;
}

/// DashMap-backed implementation of the registry contract.
pub struct BackgroundAgentRegistry {
    inner: Arc<DashMap<AgentId, BackgroundAgentHandle>>,
    metrics_tx: Option<UnboundedSender<RegistryEvent>>,
}

impl BackgroundAgentRegistry {
    /// Construct a registry with no metrics sink.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
            metrics_tx: None,
        }
    }

    /// Construct a registry that forwards lifecycle events to the
    /// supplied metrics channel. Used by the observability layer in
    /// TECH-AGS-NFR (deferred).
    pub fn with_metrics(tx: UnboundedSender<RegistryEvent>) -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
            metrics_tx: Some(tx),
        }
    }

    /// Best-effort emit. A dropped receiver is not fatal to the
    /// registry — the caller keeps running and the error is ignored
    /// (ERR-ARCH-02 handles the equivalent case for the agent-event
    /// channel in TASK-AGS-108).
    fn emit(&self, event: RegistryEvent) {
        if let Some(tx) = self.metrics_tx.as_ref() {
            let _ = tx.send(event);
        }
    }
}

impl Default for BackgroundAgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl BackgroundAgentRegistryApi for BackgroundAgentRegistry {
    fn register(&self, handle: BackgroundAgentHandle) -> Result<(), RegistryError> {
        let id = handle.agent_id;
        if self.inner.contains_key(&id) {
            return Err(RegistryError::Duplicate(id));
        }
        self.inner.insert(id, handle);
        self.emit(RegistryEvent::Registered(id));
        Ok(())
    }

    fn get(&self, id: &AgentId) -> Option<AgentStatus> {
        self.inner.get(id).map(|h| h.current_status())
    }

    fn cancel(&self, id: &AgentId) -> Result<(), RegistryError> {
        match self.inner.get(id) {
            Some(handle) => {
                handle.cancel_token.cancel();
                *handle.status.lock().expect("status mutex poisoned") = AgentStatus::Cancelled;
                self.emit(RegistryEvent::Cancelled(*id));
                Ok(())
            }
            None => Err(RegistryError::NotFound(*id)),
        }
    }

    fn poll_status(&self, id: &AgentId) -> Option<AgentStatus> {
        self.get(id)
    }

    fn reap_finished(&self) -> Vec<AgentId> {
        let reaped: Vec<AgentId> = self
            .inner
            .iter()
            .filter(|entry| entry.current_status().is_terminal())
            .map(|entry| *entry.key())
            .collect();

        for id in &reaped {
            if let Some((_, handle)) = self.inner.remove(id) {
                self.emit(RegistryEvent::Reaped(*id, handle.current_status()));
            }
        }

        reaped
    }

    fn iter_running(&self) -> Vec<AgentId> {
        self.inner
            .iter()
            .filter(|entry| entry.current_status() == AgentStatus::Running)
            .map(|entry| *entry.key())
            .collect()
    }
}

/// Global singleton used by the spawn sites (TASK-AGS-104/105/106).
/// Stored behind `Arc<dyn _>` so the concrete type is replaceable in
/// tests via `Arc::clone(&*BACKGROUND_AGENTS)`.
pub static BACKGROUND_AGENTS: Lazy<Arc<dyn BackgroundAgentRegistryApi>> =
    Lazy::new(|| Arc::new(BackgroundAgentRegistry::new()));

// ---------------------------------------------------------------------------
// TASK-TUI-402 / TASK-TUI-409: Thin shim API for TUI layer (Option A per
// Phase B drift-reconcile). The original spec (TASK-TUI-402) used pre-AGS-101
// primitives (oneshot receiver, started_at, &str keys, SubagentOutcome
// payload). AGS-101 replaced those with a snapshot-based AgentStatus model.
// This shim wraps the shipped registry with the minimum API the TUI needs.
//
// 5 spec→shipped reconciliations (Phase C spec-edit work):
//   R1: agent id is &AgentId (Uuid) — not &str (AGS-101 typing)
//   R2: PollOutcome::Running carries no `elapsed` field — trait doesn't
//       expose spawned_at (would require trait surgery touching AGS-104/105/107)
//   R3: PollOutcome::Complete(AgentStatus) — not Complete(SubagentOutcome).
//       AgentStatus::{Finished, Failed, Cancelled} is the reconciled
//       discriminant; result_slot payload not exposed on trait.
//   R4: sync (non-async) preserved — matches spec EC-TUI-010
//   R5: snapshot-idempotent — caller can re-poll without consumption
//       side-effects (oneshot-drain semantics do not apply)

/// Non-blocking poll outcome for a background subagent. Reconciles the
/// pre-AGS-101 spec contract to the shipped snapshot-based registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollOutcome {
    /// The id is not (or no longer) in the registry.
    Unknown,
    /// The subagent is still executing.
    Running,
    /// The subagent has reached a terminal state. Payload is the
    /// specific terminal `AgentStatus` (Finished, Failed, or Cancelled).
    Complete(AgentStatus),
}

/// Non-blocking poll. Callers may invoke this from sync contexts (e.g.
/// TUI refresh loop). Snapshot-idempotent: repeated calls with the same
/// id return the same outcome until the registry state changes.
pub fn poll_background_agent(id: &AgentId) -> PollOutcome {
    match BACKGROUND_AGENTS.get(id) {
        None => PollOutcome::Unknown,
        Some(AgentStatus::Running) => PollOutcome::Running,
        Some(terminal) => PollOutcome::Complete(terminal),
    }
}

/// Fire the registered cancellation token. Idempotent at the shim layer —
/// re-cancelling a cancelled agent returns Ok(()) from the registry impl
/// because the token is already cancelled (verify by Gate 3 probe).
/// Propagates RegistryError::NotFound for unknown ids.
pub fn cancel_background_agent(id: &AgentId) -> Result<(), RegistryError> {
    BACKGROUND_AGENTS.cancel(id)
}

// ---------------------------------------------------------------------------
// TASK-TUI-406: 60s janitor task for BACKGROUND_AGENTS registry
// (drift-reconcile from spec's gc_completed_agents + 1hr TTL)
//
// Reconciliations vs spec (TASK-TUI-406.md):
//   R1: spec calls for gc_completed_agents() + BACKGROUND_AGENTS.iter() +
//       JoinHandle::is_finished() check. Reconciled to reap_finished()
//       (line 216) which uses AgentStatus::is_terminal() — AGS-101
//       trait-encapsulated, stricter (covers Failed/Cancelled too).
//   R2: spec's 1-hour TTL reconciled to eager reap (TTL=0). STRICTER
//       memory bound; callers that need a grace window must poll before
//       the next 60s tick. Per NFR-TUI-SUB-002 this is safer, not weaker.
// ---------------------------------------------------------------------------

/// TASK-TUI-406: Spawn a 60s-interval janitor task that reaps terminal
/// entries from the global registry. Prevents unbounded growth under
/// sustained load (NFR-TUI-SUB-002).
///
/// Returns the JoinHandle so callers can abort the task on shutdown,
/// though tokio::spawn detaches — dropping the handle does not cancel
/// the task. The task runs for the lifetime of the tokio runtime.
pub fn spawn_gc_task() -> tokio::task::JoinHandle<()> {
    tokio::spawn(async {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        // First tick fires immediately; skip it so we don't reap before
        // any agent has had time to complete.
        interval.tick().await;
        loop {
            interval.tick().await;
            let _reaped = BACKGROUND_AGENTS.reap_finished();
        }
    })
}

// ---------------------------------------------------------------------------
// Module-local unit tests (smoke — full contract tests live in
// crates/archon-core/tests/task_ags_101.rs).
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_handle(status: AgentStatus) -> BackgroundAgentHandle {
        BackgroundAgentHandle {
            agent_id: Uuid::new_v4(),
            join_handle: None,
            cancel_token: CancellationToken::new(),
            spawned_at: SystemTime::now(),
            status: Arc::new(Mutex::new(status)),
            result_slot: new_result_slot(),
        }
    }

    #[test]
    fn register_get_and_duplicate() {
        let r = BackgroundAgentRegistry::new();
        let h = dummy_handle(AgentStatus::Running);
        let id = h.agent_id;
        r.register(h).unwrap();
        assert_eq!(r.get(&id), Some(AgentStatus::Running));

        let mut dup = dummy_handle(AgentStatus::Running);
        dup.agent_id = id;
        match r.register(dup) {
            Err(RegistryError::Duplicate(got)) => assert_eq!(got, id),
            other => panic!("expected Duplicate, got {other:?}"),
        }
    }

    #[test]
    fn status_is_terminal_helper() {
        assert!(!AgentStatus::Running.is_terminal());
        assert!(AgentStatus::Finished.is_terminal());
        assert!(AgentStatus::Failed.is_terminal());
        assert!(AgentStatus::Cancelled.is_terminal());
    }

    // TASK-TUI-402: shim unit test. Running-handle happy-path coverage is
    // deferred to TASK-TUI-409 integration tests to avoid contaminating
    // the global BACKGROUND_AGENTS singleton across unit-test runs.
    #[test]
    fn poll_unknown_id_returns_unknown() {
        assert_eq!(poll_background_agent(&Uuid::new_v4()), PollOutcome::Unknown);
    }

    #[test]
    fn reap_removes_terminal() {
        let r = BackgroundAgentRegistry::new();
        let running = dummy_handle(AgentStatus::Running);
        let finished = dummy_handle(AgentStatus::Finished);
        let running_id = running.agent_id;
        let finished_id = finished.agent_id;
        r.register(running).unwrap();
        r.register(finished).unwrap();

        let reaped = r.reap_finished();
        assert!(reaped.contains(&finished_id));
        assert!(!reaped.contains(&running_id));
        assert_eq!(r.get(&running_id), Some(AgentStatus::Running));
        assert_eq!(r.get(&finished_id), None);
    }
}
