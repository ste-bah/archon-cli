//! Slash-command handlers extracted from `session_loop/mod.rs` body.
//!
//! TASK #219 SESSION-LOOP-SPLIT: pulls direct session-state handlers
//! (`/clear` and `/refresh-identity`) into free async functions.
//! `slash_dispatch.rs` owns the higher-level routing for exit, compact,
//! built-in commands, and skill fallback.
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
    input_tui_tx: &archon_tui::event_channel::TuiEventSender,
    session_store: &Arc<archon_session::storage::SessionStore>,
    session_id: &str,
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
    if let Err(e) = session_store.delete_all_messages(session_id) {
        tracing::warn!("delete_all_messages after /clear failed: {e}");
    }
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
    // #187: `/clear` re-fires SessionStart, so hook-contributed context is
    // re-established for the fresh conversation rather than lost with it.
    if !clear_start_agg.additional_contexts.is_empty() {
        agent
            .lock()
            .await
            .add_hook_session_context(clear_start_agg.additional_contexts);
    }
    if let Err(error) = input_tui_tx
        .send_async(TuiEvent::TextDelta(
            "\nConversation cleared. Session reset.\n".into(),
        ))
        .await
    {
        tracing::warn!(%error, "clear command status delivery failed");
        return;
    }
    if let Err(error) = input_tui_tx
        .send_async(TuiEvent::SlashCommandComplete)
        .await
    {
        tracing::warn!(%error, "clear command completion delivery failed");
    }
}

/// Handle `/refresh-identity` — clears beta caches and re-runs discovery
/// in a background task. Returns immediately after spawning.
pub(super) async fn handle_refresh_identity_command(
    agent: &Arc<tokio::sync::Mutex<Agent>>,
    api_url: &Option<String>,
    input_tui_tx: &archon_tui::event_channel::TuiEventSender,
) {
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
                if let Err(error) = input_tui_tx
                    .send_async(TuiEvent::TextDelta(
                        "\nIdentity refresh not supported for this provider.\n".into(),
                    ))
                    .await
                {
                    tracing::warn!(%error, "identity refresh status delivery failed");
                }
                return;
            }
        }
    };
    if let Err(error) = input_tui_tx
        .send_async(TuiEvent::TextDelta(
            "\nIdentity cache will be cleared. Re-discovering beta headers in background...\n"
                .into(),
        ))
        .await
    {
        tracing::warn!(%error, "identity refresh start delivery failed");
        return;
    }

    let config_dir = dirs::config_dir().unwrap_or_default().join("archon");
    let _ = std::fs::remove_file(config_dir.join("validated_betas.json"));
    let _ = std::fs::remove_file(config_dir.join("discovered_betas.json"));

    let refresh_api_url = api_url.clone();
    let refresh_tui_tx = input_tui_tx.clone();
    archon_observability::spawn_named("identity-refresh", async move {
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
        if let Err(error) = refresh_tui_tx
            .send_async(TuiEvent::TextDelta(format!(
                "\nIdentity refresh complete: {} betas validated and cached.\n\
                 Restart archon to apply the updated beta headers.\n",
                validated.len()
            )))
            .await
        {
            tracing::warn!(%error, "identity refresh result delivery failed");
        }
    });

    if let Err(error) = input_tui_tx
        .send_async(TuiEvent::SlashCommandComplete)
        .await
    {
        tracing::warn!(%error, "identity refresh command completion delivery failed");
    }
}
