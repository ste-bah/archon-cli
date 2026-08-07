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
//! `register` surfaces a collision as `RegistryError::Duplicate`. Spawn paths
//! do not use it — they use `register_run`, which has a defined answer for an
//! id the registry has seen before rather than an error, because a subagent id
//! is not always new: `AgentTool` registers the same agent from the parent task
//! as well, and `SendMessage` resumes an agent under its original id. See
//! [`RunRegistration`].
//!
//! This registry is also the *only* thing agent liveness is derived from
//! (`board::leases::holder_liveness`), which is why it is keyed by runtime
//! subagent id rather than by UUID: not every spawn path mints a UUID.

use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
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
/// TECH-AGS-ARCH-FIXES `data_models` section exactly, plus
/// [`BackgroundAgentHandle::subagent_id`].
pub struct BackgroundAgentHandle {
    pub agent_id: AgentId,
    /// The id the *runtime* gave this agent — what reaches the subagent's
    /// `ToolContext`, what the board records as a claim holder, and what the
    /// hooks report. It is the registry's key.
    ///
    /// It is not always `agent_id.to_string()`: `AgentTool` and `TaskCreate`
    /// mint UUID subagent ids, but `archon-pipeline` mints
    /// `{session}-{ordinal}-{agent}`. Keying on the UUID would have left every
    /// pipeline agent unaskable, which is the whole reason liveness used to be
    /// derived by asking several registries in turn.
    pub subagent_id: String,
    /// `None` once the handle has been taken for awaiting, or when the agent
    /// runs in the foreground and there is no spawned task to join; otherwise
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

/// What starting a run under a given subagent id did to the registry.
///
/// The three arms exist because a subagent id is not always new.
/// `SendMessage` resumes an agent under its *original* id
/// (`archon-core/src/agent/message_delivery.rs`), so which arm a resume takes
/// depends only on whether `spawn_gc_task`'s 60s reaper happened to run in
/// between — a race, and one that must not change whether the agent is
/// reported alive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunRegistration {
    /// The id was unregistered; the handle is now the registry's.
    Registered,
    /// A run under this id is still executing, so the existing entry stands.
    /// The runtime does not admit two concurrent runs of one subagent id
    /// (`SubagentManager::register_with_id` refuses a running duplicate), so
    /// this means the same run reached the choke point twice — `AgentTool`
    /// registers on the parent task as well, to keep `execute`'s spawn marker
    /// truthful.
    AlreadyRunning,
    /// A terminal entry was left under this id and has been replaced. This is
    /// the resume case, and the reason `register` alone was not enough: a
    /// resumed agent inheriting its predecessor's terminal status would read
    /// as dead for the whole of its second life.
    Restarted,
}

/// Public contract for the background-agents registry. Defined as
/// a trait so the global singleton can be replaced with a mock in
/// tests, and so upper layers take `Arc<dyn BackgroundAgentRegistryApi>`
/// instead of a concrete type.
pub trait BackgroundAgentRegistryApi: Send + Sync {
    /// Insert a handle. Returns `Duplicate(id)` if the id already exists.
    fn register(&self, handle: BackgroundAgentHandle) -> Result<(), RegistryError>;

    /// Record that a run is starting under `handle.subagent_id`, whatever the
    /// registry currently says about that id. Total, not fallible: see
    /// [`RunRegistration`] for the three outcomes and why each one is a defined
    /// result rather than an error.
    fn register_run(&self, handle: BackgroundAgentHandle) -> RunRegistration;

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

    /// Status of the handle registered under a *runtime* subagent id.
    ///
    /// [`Self::get`] can only answer for the spawn paths that mint UUID
    /// subagent ids. Anything that knows an agent by the id the runtime handed
    /// it — the board's claim leases, above all — has to ask this way or it
    /// cannot ask about a pipeline agent at all.
    fn status_of(&self, subagent_id: &str) -> Option<AgentStatus>;

    /// Record a terminal status for a registered handle without removing it:
    /// `reap_finished` still owns removal, and a poller that has not looked
    /// since the agent finished should see `Complete`, not `Unknown`.
    ///
    /// Returns `false` if the id is unregistered or was already terminal, so
    /// the caller can be as idempotent as it likes.
    fn mark_terminal(&self, subagent_id: &str, status: AgentStatus) -> bool;

    /// Runtime ids of every handle whose status is still `Running`. The
    /// counterpart to [`Self::iter_running`] for callers that want every live
    /// agent rather than only the UUID-shaped ones.
    fn iter_running_ids(&self) -> Vec<String>;
}

/// DashMap-backed implementation of the registry contract.
///
/// Keyed by `BackgroundAgentHandle::subagent_id` rather than by `agent_id`, so
/// one registry answers for every spawn path. The `AgentId`-typed methods stay
/// on the contract and resolve through `id.to_string()`, which is exactly the
/// key for every agent that was given a UUID subagent id.
pub struct BackgroundAgentRegistry {
    inner: Arc<DashMap<String, BackgroundAgentHandle>>,
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
        let key = handle.subagent_id.clone();
        if self.inner.contains_key(&key) {
            return Err(RegistryError::Duplicate(id));
        }
        self.inner.insert(key, handle);
        self.emit(RegistryEvent::Registered(id));
        Ok(())
    }

    fn register_run(&self, handle: BackgroundAgentHandle) -> RunRegistration {
        let id = handle.agent_id;
        // One `entry` rather than a read then a write: two runners resuming the
        // same id at once must not both see a terminal entry and both replace
        // it, leaving one of them holding a handle the registry dropped.
        match self.inner.entry(handle.subagent_id.clone()) {
            Entry::Occupied(mut occupied) => {
                if occupied.get().current_status() == AgentStatus::Running {
                    return RunRegistration::AlreadyRunning;
                }
                occupied.insert(handle);
                self.emit(RegistryEvent::Registered(id));
                RunRegistration::Restarted
            }
            Entry::Vacant(vacant) => {
                vacant.insert(handle);
                self.emit(RegistryEvent::Registered(id));
                RunRegistration::Registered
            }
        }
    }

    fn get(&self, id: &AgentId) -> Option<AgentStatus> {
        self.status_of(&id.to_string())
    }

    fn cancel(&self, id: &AgentId) -> Result<(), RegistryError> {
        match self.inner.get(&id.to_string()) {
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
        let terminal: Vec<String> = self
            .inner
            .iter()
            .filter(|entry| entry.current_status().is_terminal())
            .map(|entry| entry.key().clone())
            .collect();

        let mut reaped = Vec::with_capacity(terminal.len());
        for key in &terminal {
            if let Some((_, handle)) = self.inner.remove(key) {
                self.emit(RegistryEvent::Reaped(
                    handle.agent_id,
                    handle.current_status(),
                ));
                reaped.push(handle.agent_id);
            }
        }

        reaped
    }

    fn iter_running(&self) -> Vec<AgentId> {
        self.inner
            .iter()
            .filter(|entry| entry.current_status() == AgentStatus::Running)
            .map(|entry| entry.agent_id)
            .collect()
    }

    fn status_of(&self, subagent_id: &str) -> Option<AgentStatus> {
        self.inner.get(subagent_id).map(|h| h.current_status())
    }

    fn mark_terminal(&self, subagent_id: &str, status: AgentStatus) -> bool {
        // A `Running` argument would be a caller bug, and silently storing it
        // would resurrect an agent the runtime has already given up on.
        if !status.is_terminal() {
            return false;
        }
        let Some(handle) = self.inner.get(subagent_id) else {
            return false;
        };
        let mut current = handle.status.lock().expect("status mutex poisoned");
        if current.is_terminal() {
            return false;
        }
        *current = status;
        true
    }

    fn iter_running_ids(&self) -> Vec<String> {
        self.inner
            .iter()
            .filter(|entry| entry.current_status() == AgentStatus::Running)
            .map(|entry| entry.key().clone())
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
    poll_subagent(&id.to_string())
}

/// Non-blocking poll by *runtime* subagent id — the id the board, the hooks and
/// the executor all know an agent by. Use this rather than
/// [`poll_background_agent`] unless you are holding a UUID minted by
/// `AgentTool` or `TaskCreate`; pipeline agents have no such UUID.
pub fn poll_subagent(subagent_id: &str) -> PollOutcome {
    match BACKGROUND_AGENTS.status_of(subagent_id) {
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
    archon_observability::spawn_named("background-agent-gc", async {
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

#[cfg(test)]
mod tests;
