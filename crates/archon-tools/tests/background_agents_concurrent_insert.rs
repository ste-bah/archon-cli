//! TASK-TUI-407: BACKGROUND_AGENTS concurrent-insert regression
//! (TC-TUI-SUBAGENT-01, REQ-TUI-SUB-001 [5/5], EC-TUI-011,
//! NFR-TUI-SUB-002).
//!
//! functional-spec lines 570-572 (TC-TUI-SUBAGENT-01) and 532-534
//! (REQ-TUI-SUB-001) require a unit-level proof that the global
//! `BACKGROUND_AGENTS` registry accepts 100 concurrent inserts under
//! DashMap semantics, with every id retrievable post-insert. This is
//! the lowest-level correctness gate for the subagent fan-out path
//! and is intentionally narrower than
//! `task_ags_104.rs::spawn_100_yields_100_unique_agent_ids` — that
//! test drives the full `AgentTool::execute` pipeline, whereas spec
//! line 65 for TUI-407 mandates direct registry manipulation so the
//! assertion is about the registry *itself*, not its callers.
//!
//! ------------------------------------------------------------------
//! Spec-vs-shipped reconciliation (see stage-5 coverage matrix,
//! TUI-411 file-header pattern):
//! ------------------------------------------------------------------
//! 1. Insert API shape: the spec literal at lines 30-31 says
//!    `BACKGROUND_AGENTS.insert(format!("tc-01-{i}"), BackgroundAgent
//!    { handle, receiver, started_at, parent_agent_id, request })`.
//!    The shipped AGS-101 contract
//!    (`archon_tools::background_agents::BackgroundAgentRegistryApi`)
//!    exposes `register(handle: BackgroundAgentHandle)
//!    -> Result<(), RegistryError>` with the id living *inside*
//!    `handle.agent_id` as a `Uuid`. There is no `.insert(key, …)`
//!    method and there is no `BackgroundAgent` struct. We call
//!    `register` and collect `handle.agent_id` before moving the
//!    handle into the registry.
//!
//! 2. Key space: the spec wants `String` keys of the form
//!    `"tc-01-{i}"`. Shipped `AgentId` is a `uuid::Uuid` generated
//!    per handle. We cannot force string keys, so the test collects
//!    the 100 `Uuid`s returned from 100 `register` calls and filters
//!    by *that* set instead of by a prefix. The intent is preserved:
//!    100 distinct registry entries survive concurrent insertion and
//!    every id is retrievable afterwards.
//!
//! 3. Handle fields: the spec's `BackgroundAgent { handle, receiver,
//!    started_at, parent_agent_id, request }` does not exist. Shipped
//!    `BackgroundAgentHandle` has `agent_id`, `join_handle`,
//!    `cancel_token`, `spawned_at`, `status`, `result_slot`. The local
//!    `dummy_background_handle` helper mirrors the canonical pattern
//!    in `task_ags_101.rs::make_handle` (spawns an immediate-cancel
//!    future, wires status + result_slot) so the test handle is
//!    structurally identical to the ones used by AGS-101 contract
//!    tests.
//!
//! Cleanup: every id collected during the concurrent phase is
//! cancelled + reaped at the end of the test so the global singleton
//! does not leak entries into sibling integration tests (same pattern
//! as `task_ags_104.rs::spawn_100_yields_100_unique_agent_ids`).

use std::collections::HashSet;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::SystemTime;

use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use archon_tools::background_agents::{
    self, AgentStatus, BackgroundAgentHandle, BACKGROUND_AGENTS,
};

/// Construct a minimal `BackgroundAgentHandle` whose spawned task
/// simply waits on its cancel token. Mirrors the canonical helper in
/// `task_ags_101.rs::make_handle` so the test exercises the registry
/// with a handle shape identical to AGS-101 contract tests.
fn dummy_background_handle() -> BackgroundAgentHandle {
    let cancel = CancellationToken::new();
    let cancel_child = cancel.clone();
    let result_slot = background_agents::new_result_slot();
    let result_slot_child = Arc::clone(&result_slot);
    let status_cell = Arc::new(StdMutex::new(AgentStatus::Running));
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

/// TC-TUI-SUBAGENT-01 — 100 concurrent `register` calls against the
/// global `BACKGROUND_AGENTS` singleton must all succeed, produce 100
/// unique `AgentId`s, and every id must be retrievable afterwards.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tc_tui_subagent_01_concurrent_insert() {
    // Clone the global Arc so all spawned tasks share the same
    // registry without needing to capture the static `Lazy` directly.
    let registry = Arc::clone(&*BACKGROUND_AGENTS);

    let mut set: JoinSet<Uuid> = JoinSet::new();
    for _ in 0..100 {
        let reg = Arc::clone(&registry);
        set.spawn(async move {
            let handle = dummy_background_handle();
            let id = handle.agent_id;
            reg.register(handle).expect("concurrent register must succeed");
            id
        });
    }

    // Drain the JoinSet; panic if any spawn failed.
    let mut ids: Vec<Uuid> = Vec::with_capacity(100);
    while let Some(res) = set.join_next().await {
        ids.push(res.expect("spawned register task must not panic"));
    }

    // Assertion 1 — exactly 100 ids returned.
    assert_eq!(ids.len(), 100, "expected 100 concurrent register results");

    // Assertion 2 — all 100 ids are distinct (DashMap semantics +
    // uuid-v4 entropy guarantee no collision in practice; catching a
    // collision here would indicate a DashMap regression).
    let unique: HashSet<Uuid> = ids.iter().copied().collect();
    assert_eq!(unique.len(), 100, "every concurrent insert must produce a unique AgentId");

    // Assertion 3 — every id is retrievable via the registry API.
    // This is the key TC-TUI-SUBAGENT-01 invariant: DashMap did not
    // drop or overwrite any entry during concurrent insertion.
    for id in &ids {
        let status = registry.get(id);
        assert!(
            matches!(status, Some(AgentStatus::Running)),
            "registered handle {id} must be retrievable with Running status, got {status:?}"
        );
    }

    // Cleanup — cancel + reap every id we tracked so the global
    // singleton does not carry entries into sibling tests. Pattern
    // matches task_ags_104.rs::spawn_100_yields_100_unique_agent_ids
    // and task_ags_101.rs::global_singleton_is_usable_across_references.
    for id in &ids {
        let _ = registry.cancel(id);
    }
    // Give the spawned tasks a tick to transition to Cancelled before
    // reap so `reap_finished` actually clears them.
    tokio::task::yield_now().await;
    let _ = registry.reap_finished();
}
