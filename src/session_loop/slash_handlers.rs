//! Slash-command handlers extracted from `session_loop/mod.rs` body.
//!
//! TASK #219 SESSION-LOOP-SPLIT: pulls the two largest in-loop
//! handlers (`/clear` and `/refresh-identity`) into free async functions
//! so `run_session_loop` shrinks toward the workspace 500-line ceiling.
//! The smaller `/exit | /quit | /q` and `/compact` handlers stay inline
//! because their bodies are already short (≤30 lines each) and their
//! captured-state spread is minimal.
//!
//! Behavior is unchanged — these helpers are byte-for-byte the same
//! sequence the inline blocks ran, just parameterized.

use std::sync::Arc;
use std::time::Instant;

use archon_core::agent::Agent;
use archon_tui::app::TuiEvent;

use crate::session_loop::personality_save::save_personality_snapshot_if_enabled;
use crate::slash_context::SlashCommandContext;

/// Handle `/clear` — saves personality snapshot, fires SessionEnd then
/// SessionStart hooks, clears conversation + session stats + assistant
/// response buffer.
#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_clear_command(
    agent: &Arc<tokio::sync::Mutex<Agent>>,
    cmd_ctx: &SlashCommandContext,
    input_tui_tx: &tokio::sync::mpsc::UnboundedSender<TuiEvent>,
    persist_personality: bool,
    personality_history_limit: u32,
    session_start_confidence: f32,
    session_start_instant: Instant,
) {
    // CLI-416 / TASK #242: save personality snapshot before clearing.
    let iv_arc = agent.lock().await.inner_voice().cloned();
    save_personality_snapshot_if_enabled(
        iv_arc,
        cmd_ctx.memory.as_ref(),
        &cmd_ctx.session_id,
        persist_personality,
        personality_history_limit,
        session_start_confidence,
        session_start_instant,
    )
    .await;

    // Fire SessionEnd hook before clearing
    {
        let guard = agent.lock().await;
        guard.fire_hook_detached(
            archon_core::hooks::HookType::SessionEnd,
            serde_json::json!({"hook_type": "session_end", "reason": "clear"}),
        )
    }
    .await;
    agent.lock().await.clear_watch_paths();
    // Clear conversation
    {
        let mut guard = agent.lock().await;
        guard.clear_conversation_detached()
    }
    .await;
    // Reset session stats
    {
        let mut stats = cmd_ctx.session_stats.lock().await;
        *stats = archon_core::agent::SessionStats::default();
    }
    // Clear last assistant response buffer
    {
        let mut resp = cmd_ctx.last_assistant_response.lock().await;
        resp.clear();
    }
    // Fire SessionStart hook after
    let clear_start_agg = {
        let guard = agent.lock().await;
        guard.fire_hook_detached(
            archon_core::hooks::HookType::SessionStart,
            serde_json::json!({"hook_type": "session_start", "reason": "clear"}),
        )
    }
    .await;
    if !clear_start_agg.watch_paths.is_empty() {
        tracing::info!(
            "SessionStart hook returned {} watch paths",
            clear_start_agg.watch_paths.len()
        );
        agent
            .lock()
            .await
            .add_watch_paths(clear_start_agg.watch_paths);
    }
    let _ = input_tui_tx.send(TuiEvent::TextDelta(
        "\nConversation cleared. Session reset.\n".into(),
    ));
    let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete);
}

/// Handle `/refresh-identity` — clears beta caches and re-runs discovery
/// in a background task. Returns immediately after spawning.
pub(super) async fn handle_refresh_identity_command(
    agent: &Arc<tokio::sync::Mutex<Agent>>,
    api_url: &Option<String>,
    input_tui_tx: &tokio::sync::mpsc::UnboundedSender<TuiEvent>,
) {
    // Clear the validated beta cache
    let validated_cache = dirs::config_dir()
        .unwrap_or_default()
        .join("archon")
        .join("validated_betas.json");
    let _ = std::fs::remove_file(&validated_cache);
    // Clear the raw discovered cache
    let raw_cache = dirs::config_dir()
        .unwrap_or_default()
        .join("archon")
        .join("discovered_betas.json");
    let _ = std::fs::remove_file(&raw_cache);

    // Fetch auth + identity providers under a single guard
    let (refresh_auth, refresh_identity) = {
        let guard = agent.lock().await;
        match (
            guard.auth_provider().cloned(),
            guard.identity_provider().cloned(),
        ) {
            (Some(a), Some(i)) => (a, i),
            _ => {
                drop(guard);
                let _ = input_tui_tx.send(TuiEvent::TextDelta(
                    "\nIdentity refresh not supported for this provider.\n".into(),
                ));
                return;
            }
        }
    };
    let refresh_api_url = api_url.clone();
    let refresh_tui_tx = input_tui_tx.clone();
    tokio::spawn(async move {
        let refresh_client = archon_llm::anthropic::AnthropicClient::new(
            refresh_auth,
            refresh_identity,
            refresh_api_url,
        );
        let validated =
            archon_llm::identity::resolve_and_validate_betas(&refresh_client, None).await;
        tracing::info!(
            "Identity refresh complete: {} betas validated",
            validated.len()
        );
        let _ = refresh_tui_tx.send(TuiEvent::TextDelta(format!(
            "\nIdentity refresh complete: {} betas validated and cached.\n\
             Restart archon to apply the updated beta headers.\n",
            validated.len()
        )));
    });

    let _ = input_tui_tx.send(TuiEvent::TextDelta(
        "\nIdentity cache cleared. Re-discovering beta headers in background...\n".into(),
    ));
    let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete);
}
