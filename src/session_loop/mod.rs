//! Session input-loop extracted from `session.rs`.
//!
//! This module hosts `run_session_loop` — the 900-line body that used
//! to live inside a single `tokio::spawn(async move { ... })` block at
//! `src/session.rs:1959`. Extraction into a named `async fn` with
//! explicit owned parameters was required to unblock the
//! `archon-cli-workspace` bin build: three cascading
//! "Send is not general enough" HRTB errors surfaced when rustc tried
//! to infer Send bounds for the anonymous `async move` future. A
//! named function's signature gives each parameter a concrete type
//! for Send analysis, eliminating the HRTB inference failure.
//!
//! ZERO SEMANTIC CHANGE: the body is a verbatim move of the original
//! spawn block. All captured bindings are now owned parameters (or
//! `Arc<T>` — still owned, just shared). Follow-up
//! `TASK-SESSION-LOOP-SPLIT` will break this file into per-event
//! helper modules (hooks, tui_events, slash_commands). See the
//! commit body for the full rationale.

use std::path::PathBuf;
use std::sync::Arc;

use archon_core::agent::Agent;
use archon_core::skills::{SkillContext, SkillOutput};
use archon_llm::effort::EffortState;
use archon_llm::fast_mode::FastModeState;
use archon_tui::app::TuiEvent;

use crate::command::slash::handle_slash_command;
use crate::slash_context::SlashCommandContext;

mod mcp_task;

pub(crate) use mcp_task::{McpLifecycleTx, spawn_mcp_lifecycle_task};

/// Run the interactive agent input loop to completion.
///
/// TASK-SESSION-LOOP-EXTRACT (A-2): returns an explicit
/// `Pin<Box<dyn Future + Send>>` (not `async fn` → `impl Future`).
/// The A-2 channel flip removed the `&Sender<TuiEvent>` HRTB error,
/// but the async body still holds `&mut SlashCommandContext` /
/// `&str` borrows across many `.await` sites, and rustc's
/// higher-ranked Send inference fails on those patterns
/// (rust-lang/rust#102211). The explicit trait-object return type
/// forces rustc to use the concrete boxed-future type for Send
/// analysis — `tokio::spawn(run_session_loop(..))` then type-checks
/// concretely. Zero semantic change.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_session_loop(
    agent: Agent,
    agent_def: Option<archon_core::agents::CustomAgentDefinition>,
    api_url: Option<String>,
    input_tui_tx: tokio::sync::mpsc::UnboundedSender<TuiEvent>,
    mut user_input_rx: tokio::sync::mpsc::Receiver<String>,
    agent_event_tx_for_dispatcher: tokio::sync::mpsc::UnboundedSender<
        archon_core::agent::TimestampedEvent,
    >,
    session_store_for_input: Arc<archon_session::storage::SessionStore>,
    session_id_for_input: String,
    persist_personality: bool,
    personality_history_limit: u32,
    session_start_instant: std::time::Instant,
    session_start_confidence: f32,
    slash_commands_disabled: bool,
    mut fast_mode: FastModeState,
    mut effort_state: EffortState,
    mut cmd_ctx: SlashCommandContext,
    mcp_lifecycle_tx: McpLifecycleTx,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
        let agent = Arc::new(tokio::sync::Mutex::new(agent));

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

        // AGT-015: Send agent name/color to TUI status bar when in --agent mode
        if let Some(ref def) = agent_def {
            let _ = input_tui_tx.send(TuiEvent::SetAgentInfo {
                name: def.agent_type.clone(),
                color: def.color.clone(),
            });
        }

        // AGT-011: Track whether the agent's initial_prompt has been prepended
        let mut initial_prompt_pending: Option<String> =
            agent_def.as_ref().and_then(|d| d.initial_prompt.clone());

        // TASK-TUI-107: AgentDispatcher owns the in-flight turn lifecycle,
        // queues overflow prompts FIFO, and polls completion without
        // blocking. AgentHandle bridges `Arc<Mutex<Agent>>` to TurnRunner.
        let adapter = Arc::new(crate::agent_handle::AgentHandle::new(Arc::clone(&agent)));
        let router: Arc<dyn archon_tui::AgentRouter> =
            Arc::new(crate::agent_handle::NoopAgentRouter);
        let mut dispatcher =
            archon_tui::AgentDispatcher::new(router, agent_event_tx_for_dispatcher);
        // Per-turn post-completion actions: pushed in dispatch order,
        // popped on each `poll_completion()` outcome. Replaces the
        // per-spawn tail logic that used to live inside the deleted
        // `tokio::spawn(async move { process_message })` wrapper.
        enum PostTurnAction {
            PersistSession,
            SkillComplete { reload_registry_for: Option<String> },
        }
        let mut post_turn_queue: std::collections::VecDeque<PostTurnAction> =
            std::collections::VecDeque::new();
        let mut poll_tick = tokio::time::interval(std::time::Duration::from_millis(16));
        poll_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // BEGIN INPUT_HANDLER — arch-lint.sh scopes D1 grep to this region
        loop {
            let input = tokio::select! {
                biased;
                _ = poll_tick.tick() => {
                    if let Some(outcome) = dispatcher.poll_completion() {
                        tracing::debug!("dispatcher turn outcome: {}",
                            match &outcome {
                                archon_tui::TurnOutcome::Completed => "completed",
                                archon_tui::TurnOutcome::Cancelled => "cancelled",
                                archon_tui::TurnOutcome::Failed(_) => "failed",
                            });
                        // FIFO: pop the post-turn action for the dispatch
                        // that just completed and run it.
                        match post_turn_queue.pop_front() {
                            Some(PostTurnAction::PersistSession) => {
                                let guard = agent.lock().await;
                                for (idx, msg) in
                                    guard.conversation_state().messages.iter().enumerate()
                                {
                                    if let Ok(json_str) = serde_json::to_string(msg) {
                                        if let Err(e) = session_store_for_input
                                            .save_message(
                                                &session_id_for_input,
                                                idx as u64,
                                                &json_str,
                                            )
                                        {
                                            tracing::warn!("save_message idx {idx}: {e}");
                                        }
                                    }
                                }
                            }
                            Some(PostTurnAction::SkillComplete { reload_registry_for }) => {
                                if reload_registry_for.as_deref() == Some("create-agent") {
                                    if let Ok(mut reg) = cmd_ctx.agent_registry.write() {
                                        reg.reload(&cmd_ctx.working_dir);
                                        tracing::info!("agent registry reloaded");
                                    }
                                }
                                let _ = input_tui_tx
                                    .send(TuiEvent::SlashCommandComplete);
                            }
                            None => {}
                        }
                    }
                    continue;
                }
                maybe_input = user_input_rx.recv() => {
                    match maybe_input {
                        Some(input) => input,
                        None => break,
                    }
                }
            };
            // Session picker selection — load messages and restore conversation
            if let Some(session_id) = input.strip_prefix("__resume_session__ ") {
                let session_id = session_id.trim();
                let db_path = archon_session::storage::default_db_path();
                match archon_session::storage::SessionStore::open(&db_path) {
                    Ok(store) => {
                        // Restore session name badge if present
                        if let Ok(meta) = store.get_session(session_id)
                            && let Some(name) = meta.name
                        {
                            let _ = input_tui_tx.send(TuiEvent::SessionRenamed(name));
                        }
                        match store.load_messages(session_id) {
                            Ok(raw_messages) => {
                                // Parse JSON strings back to Values
                                let messages: Vec<serde_json::Value> = raw_messages
                                    .iter()
                                    .filter_map(|s| serde_json::from_str(s).ok())
                                    .collect();
                                let count = messages.len();
                                {
                                    let mut guard = agent.lock().await;
                                    guard.clear_conversation_detached()
                                }
                                .await;

                                // Display the loaded conversation history in the output
                                let _ = input_tui_tx.send(TuiEvent::TextDelta(format!(
                                    "\n━━━ Resumed session {session_id} ({count} messages) ━━━\n\n"
                                )));
                                for msg in &messages {
                                    let role = msg["role"].as_str().unwrap_or("unknown");
                                    // Extract text content (handles both string and array formats)
                                    let content = match &msg["content"] {
                                        serde_json::Value::String(s) => s.clone(),
                                        serde_json::Value::Array(arr) => arr
                                            .iter()
                                            .filter_map(|item| {
                                                item["text"].as_str().map(|s| s.to_string())
                                            })
                                            .collect::<Vec<_>>()
                                            .join("\n"),
                                        _ => String::new(),
                                    };
                                    if content.is_empty() {
                                        continue;
                                    }
                                    let label = match role {
                                        "user" => "> ",
                                        "assistant" => "",
                                        _ => "",
                                    };
                                    let _ = input_tui_tx
                                        .send(TuiEvent::TextDelta(format!("{label}{content}\n\n")));
                                }
                                let _ = input_tui_tx.send(TuiEvent::TextDelta(
                                    "━━━ End of history — continue conversation ━━━\n\n"
                                        .to_string(),
                                ));

                                agent.lock().await.restore_conversation(messages);
                            }
                            Err(e) => {
                                let _ = input_tui_tx
                                    .send(TuiEvent::Error(format!("Failed to load session: {e}")));
                            }
                        }
                    }
                    Err(e) => {
                        let _ =
                            input_tui_tx.send(TuiEvent::Error(format!("Session store error: {e}")));
                    }
                }
                let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete);
                continue;
            }

            // ── TASK-TUI-620-followup: /rewind truncation ─────────
            // Emitted by the MessageSelector overlay when the user hits
            // Enter. `idx` is the message index the user rewound to —
            // messages [0..=idx] are kept, everything after is dropped
            // from the SessionStore AND the agent's in-memory
            // conversation is rebuilt to match.
            if let Some(idx_str) = input.strip_prefix("__truncate_session__ ") {
                let idx_str = idx_str.trim();
                let idx: u64 = match idx_str.parse() {
                    Ok(n) => n,
                    Err(_) => {
                        let _ = input_tui_tx.send(TuiEvent::TextDelta(format!(
                            "\n[rewind: invalid index '{idx_str}']\n"
                        )));
                        let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete);
                        continue;
                    }
                };

                let target_session_id = session_id_for_input.clone();
                let db_path = archon_session::storage::default_db_path();
                match archon_session::storage::SessionStore::open(&db_path) {
                    Ok(store) => {
                        if let Err(e) = store.truncate_messages_after(&target_session_id, idx) {
                            let _ = input_tui_tx
                                .send(TuiEvent::Error(format!("Failed to truncate session: {e}")));
                            let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete);
                            continue;
                        }

                        // Reload the retained messages and rebuild the
                        // in-memory conversation so the next turn sees
                        // only history up to `idx`.
                        match store.load_messages(&target_session_id) {
                            Ok(raw_messages) => {
                                let messages: Vec<serde_json::Value> = raw_messages
                                    .iter()
                                    .filter_map(|s| serde_json::from_str(s).ok())
                                    .collect();
                                let count = messages.len();
                                {
                                    let mut guard = agent.lock().await;
                                    guard.clear_conversation_detached()
                                }
                                .await;

                                let _ = input_tui_tx.send(TuiEvent::TextDelta(format!(
                                    "\n━━━ Rewound to message {idx} ({count} messages kept) ━━━\n\n"
                                )));
                                for msg in &messages {
                                    let role = msg["role"].as_str().unwrap_or("unknown");
                                    let content = match &msg["content"] {
                                        serde_json::Value::String(s) => s.clone(),
                                        serde_json::Value::Array(arr) => arr
                                            .iter()
                                            .filter_map(|item| {
                                                item["text"].as_str().map(|s| s.to_string())
                                            })
                                            .collect::<Vec<_>>()
                                            .join("\n"),
                                        _ => String::new(),
                                    };
                                    if content.is_empty() {
                                        continue;
                                    }
                                    let label = match role {
                                        "user" => "> ",
                                        "assistant" => "",
                                        _ => "",
                                    };
                                    let _ = input_tui_tx
                                        .send(TuiEvent::TextDelta(format!("{label}{content}\n\n")));
                                }
                                let _ = input_tui_tx.send(TuiEvent::TextDelta(
                                    "━━━ End of history — continue conversation ━━━\n\n"
                                        .to_string(),
                                ));

                                agent.lock().await.restore_conversation(messages);
                            }
                            Err(e) => {
                                let _ = input_tui_tx.send(TuiEvent::Error(format!(
                                    "Failed to reload session after truncate: {e}"
                                )));
                            }
                        }
                    }
                    Err(e) => {
                        let _ =
                            input_tui_tx.send(TuiEvent::Error(format!("Session store error: {e}")));
                    }
                }
                let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete);
                continue;
            }

            // ── TASK-AGS-107 / TASK-TUI-107: Ctrl+C cancel ───────
            // TUI sends "__cancel__" when user presses Ctrl+C during
            // generation. Fire the CancellationToken held by the adapter
            // (propagates into ToolContext.cancel_parent → subagent
            // child_token() chains) AND abort the tracked JoinHandle via
            // the dispatcher (reaches the turn future at its next `.await`).
            // Both operations are non-blocking.
            if input == "__cancel__" {
                adapter.fire_cancel();
                match dispatcher.cancel_current() {
                    archon_tui::CancelOutcome::NoInflight => {
                        tracing::debug!("Ctrl+C: no in-flight turn to cancel");
                    }
                    archon_tui::CancelOutcome::Aborted { elapsed_ms } => {
                        tracing::info!("Ctrl+C: aborted in-flight turn (elapsed_ms={elapsed_ms})");
                    }
                }
                continue;
            }

            // ── MCP manager actions from the overlay ─────────────
            if let Some(rest) = input.strip_prefix("__mcp_action__ ") {
                let parts: Vec<&str> = rest.trim().splitn(2, ' ').collect();
                if parts.len() == 2 {
                    let (server_name, action) = (parts[0], parts[1]);
                    match action {
                        "reconnect" => {
                            let _ = mcp_task::request_restart(&mcp_lifecycle_tx, server_name).await;
                        }
                        "disable" => {
                            let _ = mcp_task::request_disable(&mcp_lifecycle_tx, server_name).await;
                        }
                        "enable" => {
                            let _ = mcp_task::request_enable(&mcp_lifecycle_tx, server_name).await;
                        }
                        _ => {}
                    }
                    // Send updated state back to TUI overlay.
                    let info = cmd_ctx.mcp_manager.get_server_info().await;
                    let mut updated: Vec<archon_tui::app::McpServerEntry> = Vec::new();
                    for (name, state, disabled) in info {
                        let state_str = if disabled {
                            "disabled"
                        } else {
                            match state {
                                archon_mcp::types::ServerState::Ready => "ready",
                                archon_mcp::types::ServerState::Starting
                                | archon_mcp::types::ServerState::Restarting => "starting",
                                archon_mcp::types::ServerState::Crashed => "crashed",
                                archon_mcp::types::ServerState::Stopped => "stopped",
                            }
                        };
                        let tools = if state_str == "ready" {
                            cmd_ctx.mcp_manager.list_tools_for(&name).await
                        } else {
                            Vec::new()
                        };
                        updated.push(archon_tui::app::McpServerEntry {
                            name: name.clone(),
                            state: state_str.to_string(),
                            tool_count: tools.len(),
                            disabled,
                            tools,
                        });
                    }
                    let _ = input_tui_tx.send(TuiEvent::UpdateMcpManager(updated));
                }
                let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete);
                continue;
            }

            // ── Phase 2: Slash command dispatch (CLI-110) ────────
            if !slash_commands_disabled && input.starts_with('/') {
                // GAP 1: /compact needs direct access to agent.compact()
                if matches!(input.trim(), "/exit" | "/quit" | "/q") {
                    // CLI-416: Save personality snapshot before session ends.
                    if persist_personality {
                        let iv_arc = agent.lock().await.inner_voice().cloned();
                        if let Some(iv_arc) = iv_arc {
                            let iv = iv_arc.lock().await;
                            let stats = iv.to_session_stats(
                                session_start_confidence,
                                session_start_instant.elapsed().as_secs(),
                            );
                            let snapshot_iv = iv.on_compaction();
                            drop(iv);
                            let engine = archon_consciousness::rules::RulesEngine::new(
                                cmd_ctx.memory.as_ref(),
                            );
                            let rule_scores = engine.export_scores().unwrap_or_default();
                            let snap = archon_consciousness::persistence::PersonalitySnapshot {
                                session_id: cmd_ctx.session_id.clone(),
                                timestamp: chrono::Utc::now(),
                                inner_voice: snapshot_iv,
                                rule_scores,
                                stats,
                            };
                            if let Err(e) = archon_consciousness::persistence::save_snapshot(
                                cmd_ctx.memory.as_ref(),
                                &snap,
                            ) {
                                tracing::warn!("personality: failed to save snapshot: {e}");
                            }
                            if let Err(e) = archon_consciousness::persistence::prune_snapshots(
                                cmd_ctx.memory.as_ref(),
                                personality_history_limit,
                            ) {
                                tracing::warn!("personality: failed to prune snapshots: {e}");
                            }
                        }
                    }

                    // Fire SessionEnd hook and close the TUI
                    {
                        let guard = agent.lock().await;
                        guard.fire_hook_detached(
                            archon_core::hooks::HookType::SessionEnd,
                            serde_json::json!({"hook_type": "session_end", "reason": "exit"}),
                        )
                    }
                    .await;
                    agent.lock().await.clear_watch_paths();
                    let _ = input_tui_tx.send(TuiEvent::TextDelta("\nGoodbye.\n".into()));
                    let _ = input_tui_tx.send(TuiEvent::Done);
                    continue;
                }
                if input.trim() == "/compact" || input.trim().starts_with("/compact ") {
                    let subcommand = input.trim().strip_prefix("/compact").unwrap().trim();
                    let subcommand = if subcommand.is_empty() {
                        None
                    } else {
                        Some(subcommand)
                    };
                    let msg = {
                        let mut guard = agent.lock().await;
                        let fut: std::pin::Pin<
                            Box<dyn std::future::Future<Output = String> + Send + '_>,
                        > = Box::pin(guard.compact(subcommand));
                        fut.await
                    };
                    let _ = input_tui_tx.send(TuiEvent::TextDelta(format!("\n{msg}\n")));
                    let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete);
                    continue;
                }

                // /clear needs direct access to agent.clear_conversation()
                if input.trim() == "/clear" {
                    // CLI-416: Save personality snapshot before clearing session.
                    if persist_personality {
                        let iv_arc = agent.lock().await.inner_voice().cloned();
                        if let Some(iv_arc) = iv_arc {
                            let iv = iv_arc.lock().await;
                            let stats = iv.to_session_stats(
                                session_start_confidence,
                                session_start_instant.elapsed().as_secs(),
                            );
                            let snapshot_iv = iv.on_compaction();
                            drop(iv);
                            let engine = archon_consciousness::rules::RulesEngine::new(
                                cmd_ctx.memory.as_ref(),
                            );
                            let rule_scores = engine.export_scores().unwrap_or_default();
                            let snap = archon_consciousness::persistence::PersonalitySnapshot {
                                session_id: cmd_ctx.session_id.clone(),
                                timestamp: chrono::Utc::now(),
                                inner_voice: snapshot_iv,
                                rule_scores,
                                stats,
                            };
                            if let Err(e) = archon_consciousness::persistence::save_snapshot(
                                cmd_ctx.memory.as_ref(),
                                &snap,
                            ) {
                                tracing::warn!("personality: failed to save snapshot: {e}");
                            }
                            if let Err(e) = archon_consciousness::persistence::prune_snapshots(
                                cmd_ctx.memory.as_ref(),
                                personality_history_limit,
                            ) {
                                tracing::warn!("personality: failed to prune snapshots: {e}");
                            }
                        }
                    }

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
                    continue;
                }

                // /refresh-identity — clears beta caches and re-runs discovery in background
                if input.trim() == "/refresh-identity" {
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

                    // Spawn background re-discovery using a temporary client
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
                                continue;
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
                            archon_llm::identity::resolve_and_validate_betas(&refresh_client, None)
                                .await;
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
                        "\nIdentity cache cleared. Re-discovering beta headers in background...\n"
                            .into(),
                    ));
                    let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete);
                    continue;
                }

                // TASK-AGS-POST-6-EXPORT-MIGRATE: upstream /export
                // intercept DELETED. Parse + validate logic moved into
                // `ExportHandler::execute` (src/command/export.rs); the
                // mutex-requiring file-write I/O moved into the drain
                // block below inside the `if handled {` branch where
                // `agent` (Arc<tokio::sync::Mutex<Agent>>) and
                // `cmd_ctx.session_id` are in scope. See
                // src/command/export.rs module rustdoc for the full
                // SIDECAR-SLOT rationale.

                let handled = handle_slash_command(
                    input.trim(),
                    &mut fast_mode,
                    &mut effort_state,
                    &input_tui_tx,
                    &mut cmd_ctx,
                )
                .await;
                if handled {
                    // TASK-AGS-POST-6-EXPORT-MIGRATE drain. The sync
                    // `ExportHandler::execute` (src/command/export.rs)
                    // stashes an `ExportDescriptor` in the shared
                    // `cmd_ctx.pending_export_shared` slot when the
                    // format arg parses; we `.take()` it here — where
                    // the `agent` mutex and `cmd_ctx.session_id` are
                    // in scope — and perform the async I/O verbatim
                    // against the pre-migration intercept (former
                    // :2431-2486). The handler cannot do this itself
                    // because `CommandHandler::execute` is sync and
                    // `apply_effect` in slash.rs runs with only
                    // `SlashCommandContext` (no Agent mutex).
                    // Single-shot by `.take()`; a None here means
                    // either the command wasn't /export, or the
                    // handler hit the parse-error branch (which
                    // already emitted TuiEvent::Error directly).
                    let export_desc = cmd_ctx
                        .pending_export_shared
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .take();
                    if let Some(desc) = export_desc {
                        let format = desc.format;
                        let format_arg = desc.format_arg_display;
                        let export_result = {
                            let guard = agent.lock().await;
                            let messages = &guard.conversation_state().messages;
                            archon_session::export::export_session(
                                messages,
                                &cmd_ctx.session_id,
                                format,
                            )
                        };
                        match export_result {
                            Ok(content) => {
                                let export_dir = dirs::data_dir()
                                    .unwrap_or_else(|| PathBuf::from("."))
                                    .join("archon")
                                    .join("exports");
                                if let Err(e) = std::fs::create_dir_all(&export_dir) {
                                    let _ = input_tui_tx.send(TuiEvent::Error(format!(
                                        "Failed to create export dir: {e}"
                                    )));
                                    let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete);
                                    continue;
                                }
                                let filename = archon_session::export::default_export_filename(
                                    &cmd_ctx.session_id,
                                    format,
                                );
                                let path = export_dir.join(&filename);
                                match archon_session::export::write_export(&content, &path) {
                                    Ok(()) => {
                                        let _ = input_tui_tx.send(TuiEvent::TextDelta(format!(
                                            "\nExported ({format_arg_display}) to {}\n",
                                            path.display(),
                                            format_arg_display = if format_arg.is_empty() {
                                                "markdown"
                                            } else {
                                                format_arg.as_str()
                                            }
                                        )));
                                    }
                                    Err(e) => {
                                        let _ = input_tui_tx
                                            .send(TuiEvent::Error(format!("Export failed: {e}")));
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = input_tui_tx
                                    .send(TuiEvent::Error(format!("Export failed: {e}")));
                            }
                        }
                    }
                    let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete);
                    continue;
                }

                // Fallback: check the skill registry for expanded commands
                let (cmd_name, cmd_args) =
                    match archon_core::skills::parser::parse_slash_command(input.trim()) {
                        Some((name, args)) => (name, args),
                        None => (String::new(), Vec::new()),
                    };

                // TASK-SESSION-LOOP-EXTRACT: compute skill output eagerly and
                // drop the `&dyn Skill` borrow BEFORE the `.await`s below so
                // rustc doesn't have to prove `for<'a> &'a dyn Skill: Send`.
                // The skill registry lookup + `execute` are sync, so we can
                // take the output as an owned value and release the borrow.
                let skill_output: Option<SkillOutput> = {
                    let skill = cmd_ctx.skill_registry.resolve(&cmd_name);
                    skill.map(|s| {
                        let skill_ctx = SkillContext {
                            session_id: cmd_ctx.session_id.clone(),
                            working_dir: cmd_ctx.working_dir.clone(),
                            model: cmd_ctx.default_model.clone(),
                            agent_registry: Some(Arc::clone(&cmd_ctx.agent_registry)),
                        };
                        s.execute(&cmd_args, &skill_ctx)
                    })
                };
                if let Some(output) = skill_output {
                    match output {
                        SkillOutput::Prompt(prompt) => {
                            // Equivalent to Claude Code's PromptCommand — inject into
                            // the conversation as a user message and let the agent respond.
                            {
                                let mut resp = cmd_ctx.last_assistant_response.lock().await;
                                resp.clear();
                            }
                            let _ = input_tui_tx.send(TuiEvent::GenerationStarted);
                            // TASK-TUI-107: dispatch via AgentDispatcher. The
                            // old `handle.await`-prior serialization pattern
                            // is gone — queued prompts drain via poll_completion.
                            // Post-turn work (SlashCommandComplete event,
                            // optional registry reload) is pushed onto
                            // post_turn_queue and runs when poll_completion
                            // observes this turn's outcome.
                            match dispatcher.spawn_turn(
                                prompt.clone(),
                                adapter.clone() as std::sync::Arc<dyn archon_tui::TurnRunner>,
                            ) {
                                archon_tui::DispatchResult::Running { .. } => {
                                    tracing::debug!("spawned skill agent turn");
                                }
                                archon_tui::DispatchResult::Queued => {
                                    tracing::debug!("agent busy; queued skill prompt");
                                }
                                archon_tui::DispatchResult::Rejected(err) => {
                                    tracing::error!("skill dispatch rejected: {err}");
                                }
                            }
                            post_turn_queue.push_back(PostTurnAction::SkillComplete {
                                reload_registry_for: Some(cmd_name.clone()),
                            });
                        }
                        SkillOutput::Text(t) | SkillOutput::Markdown(t) => {
                            let _ = input_tui_tx.send(TuiEvent::TextDelta(format!("\n{t}\n")));
                            let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete);
                        }
                        SkillOutput::Error(e) => {
                            let _ =
                                input_tui_tx.send(TuiEvent::TextDelta(format!("\nError: {e}\n")));
                            let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete);
                        }
                    }
                    continue;
                }

                // Not a known slash command — falls through to agent as normal input
            }

            // Clear last response buffer for /copy
            {
                let mut resp = cmd_ctx.last_assistant_response.lock().await;
                resp.clear();
            }
            // CRIT-06: Fire UserPromptSubmit hook before processing
            {
                let guard = agent.lock().await;
                guard.fire_hook_detached(
                    archon_core::hooks::HookType::UserPromptSubmit,
                    serde_json::json!({
                        "hook_event": "UserPromptSubmit",
                        "prompt_length": input.len(),
                    }),
                )
            }
            .await;
            // Signal the TUI that generation is starting BEFORE the agent runs.
            // This is the canonical place is_generating gets set to true.
            let _ = input_tui_tx.send(TuiEvent::GenerationStarted);
            // AGT-011: Prepend initial_prompt to first user message
            let effective_input = if let Some(prefix) = initial_prompt_pending.take() {
                format!("{prefix}\n\n{input}")
            } else {
                input.clone()
            };
            // TASK-TUI-107: dispatch via AgentDispatcher. The old
            // `handle.await`-prior serialization pattern is gone — queued
            // prompts drain via poll_completion. Session persistence is
            // pushed onto post_turn_queue and runs when poll_completion
            // observes this turn's outcome.
            match dispatcher.spawn_turn(
                effective_input,
                adapter.clone() as std::sync::Arc<dyn archon_tui::TurnRunner>,
            ) {
                archon_tui::DispatchResult::Running { .. } => {
                    tracing::debug!("spawned agent turn");
                }
                archon_tui::DispatchResult::Queued => {
                    tracing::debug!("agent busy; queued prompt");
                }
                archon_tui::DispatchResult::Rejected(err) => {
                    tracing::error!("dispatch rejected: {err}");
                }
            }
            post_turn_queue.push_back(PostTurnAction::PersistSession);
        }
        // END INPUT_HANDLER — arch-lint.sh scopes D1 grep to this region

        // AGT-015: Increment agent invocation count on session end.
        // Wired ONLY here (not at /exit) to avoid double-counting — the Stop
        // hook fires on ALL exit paths (/exit, /quit, Ctrl-C, channel close).
        if let Some(ref def) = agent_def {
            if let Some(ref base_dir) = def.base_dir {
                let agent_dir = std::path::Path::new(base_dir);
                if let Err(e) = archon_core::agents::memory::increment_invocation_count(agent_dir) {
                    tracing::warn!(
                        agent = def.agent_type.as_str(),
                        "failed to increment invocation count: {e}"
                    );
                }
            }
        }

        // TASK-TUI-107: drain in-flight turn + queue before the Stop hook.
        let drain_deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while (dispatcher.is_busy() || dispatcher.queue_len() > 0)
            && std::time::Instant::now() < drain_deadline
        {
            tokio::time::sleep(std::time::Duration::from_millis(16)).await;
            let _ = dispatcher.poll_completion();
        }

        // CRIT-06: Fire Stop hook when the input channel closes (session ending)
        let stop_fut = {
            let guard = agent.lock().await;
            guard.fire_hook_detached(
                archon_core::hooks::HookType::Stop,
                serde_json::json!({
                    "hook_event": "Stop",
                    "reason": "session_end",
                }),
            )
        };
        let stop_result = tokio::time::timeout(std::time::Duration::from_secs(10), stop_fut).await;

        // CRIT-06: Fire StopFailure if the graceful stop timed out
        if stop_result.is_err() {
            tracing::warn!("Stop hook timed out — firing StopFailure");
            {
                let guard = agent.lock().await;
                guard.fire_hook_detached(
                    archon_core::hooks::HookType::StopFailure,
                    serde_json::json!({
                        "hook_event": "StopFailure",
                        "reason": "stop_hook_timeout",
                    }),
                )
            }
            .await;
        }
    })
}
