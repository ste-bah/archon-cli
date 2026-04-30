//! Session-lifecycle hook fires extracted from `session_loop/mod.rs`.
//!
//! TASK #219 SESSION-LOOP-SPLIT: pulls the Setup → SessionStart →
//! InstructionsLoaded sequence (CRIT-06) into a single helper. The
//! sequence is fire-and-await with a single side-effect (consuming
//! `watch_paths` from SessionStart's aggregated result) — extracting
//! it removes ~50 lines from the run_session_loop body without
//! changing semantics.

use std::sync::Arc;

use archon_core::agent::Agent;

/// Fire the three startup hooks in order: Setup, SessionStart,
/// InstructionsLoaded. Consumes any `watch_paths` returned by
/// SessionStart and registers them on the agent.
///
/// Reference: CRIT-06 (full hook lifecycle), REQ-HOOK-017 (watch_paths
/// consumption).
pub(super) async fn fire_session_startup_hooks(agent: &Arc<tokio::sync::Mutex<Agent>>) {
    // CRIT-06: Fire Setup hook once agent is fully configured
    {
        let guard = agent.lock().await;
        guard.fire_hook_detached(
            archon_core::hooks::HookType::Setup,
            serde_json::json!({
                "hook_event": "Setup",
            }),
        )
    }
    .await;

    // CRIT-06: Fire SessionStart hook at the beginning of the session
    let session_start_agg = {
        let guard = agent.lock().await;
        guard.fire_hook_detached(
            archon_core::hooks::HookType::SessionStart,
            serde_json::json!({
                "hook_event": "SessionStart",
                "reason": "new_session",
            }),
        )
    }
    .await;
    // Consume watch_paths from SessionStart hooks (REQ-HOOK-017)
    if !session_start_agg.watch_paths.is_empty() {
        tracing::info!(
            "SessionStart hook returned {} watch paths",
            session_start_agg.watch_paths.len()
        );
        agent
            .lock()
            .await
            .add_watch_paths(session_start_agg.watch_paths);
    }

    // CRIT-06: Fire InstructionsLoaded hook after session starts and instructions are loaded
    {
        let guard = agent.lock().await;
        guard.fire_hook_detached(
            archon_core::hooks::HookType::InstructionsLoaded,
            serde_json::json!({
                "hook_event": "InstructionsLoaded",
            }),
        )
    }
    .await;
}
