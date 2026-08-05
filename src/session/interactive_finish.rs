use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::cli_args::Cli;
use archon_core::agent::{Agent, SessionStats, TimestampedEvent};
use archon_memory::MemoryTrait;
use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;

async fn display_initial_resume_history(
    tui_event_tx: &TuiEventSender,
    messages: &[serde_json::Value],
) -> Result<(), crate::session_loop::session_history::HistorySendError> {
    let banner = format!(
        "\n━━━ Resumed session history ({} messages) ━━━\n\n",
        messages.len()
    );
    crate::session_loop::session_history::send_history(tui_event_tx, &banner, messages).await
}

/// What the automatic consolidation pass changed, or `None` when it changed
/// nothing.
///
/// Returned rather than printed. An emission from here lands in the output
/// buffer, which the splash screen is drawn INSTEAD of at startup -- so the
/// message is real, queued, and invisible until the splash clears. It goes into
/// the splash's own activity panel instead, which is on screen immediately.
///
/// Silent on a no-op run. The pass fires on every session start once the
/// throttle elapses, and a line on every launch saying "nothing happened" is
/// noise people learn to skip — which would defeat the point of showing it at
/// all.
fn auto_consolidation_summary(report: &archon_memory::garden::GardenReport) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if report.duplicates_merged > 0 {
        parts.push(format!("{} duplicate(s) merged", report.duplicates_merged));
    }
    if report.fragments_merged > 0 {
        parts.push(format!("{} fragment(s) merged", report.fragments_merged));
    }
    if report.stale_pruned > 0 {
        parts.push(format!("{} stale pruned", report.stale_pruned));
    }
    if report.overflow_pruned > 0 {
        parts.push(format!("{} pruned for overflow", report.overflow_pruned));
    }
    if !report.review_pairs.is_empty() {
        parts.push(format!(
            "{} pair(s) awaiting review",
            report.review_pairs.len()
        ));
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join(", "))
}

/// The most recent inner-voice snapshot, newest first.
///
/// Fetched by explicit type rather than `recall_memories("inner_voice_snapshot")`.
/// The text-search form only worked because the row was written as a `Fact` at
/// importance 90, which floated a JSON blob of runtime state above every real
/// memory in ordinary recall.
///
/// Falls back to the legacy `Fact` shape so stores written by earlier versions
/// still restore. Without that, upgrading would silently lose the inner-voice
/// state -- silently, because the caller has no else branch.
fn latest_inner_voice_snapshot(
    memory: &dyn archon_memory::MemoryTrait,
) -> Option<archon_memory::types::Memory> {
    let newest = |mut rows: Vec<archon_memory::types::Memory>| {
        rows.retain(|m| m.tags.iter().any(|t| t == "inner_voice_snapshot"));
        rows.sort_by_key(|m| std::cmp::Reverse(m.created_at));
        rows.into_iter().next()
    };

    let typed = memory
        .search_memories(&archon_memory::types::SearchFilter {
            memory_type: Some(archon_memory::types::MemoryType::PersonalitySnapshot),
            ..Default::default()
        })
        .ok()
        .and_then(newest);
    if typed.is_some() {
        return typed;
    }

    memory
        .search_memories(&archon_memory::types::SearchFilter {
            tags: vec!["inner_voice_snapshot".to_string()],
            ..Default::default()
        })
        .ok()
        .and_then(newest)
}

async fn replay_resumed_conversation(
    tui_event_tx: &TuiEventSender,
    messages: Vec<serde_json::Value>,
) -> Option<Vec<serde_json::Value>> {
    if let Err(error) = display_initial_resume_history(tui_event_tx, &messages).await {
        tracing::error!("failed to replay resumed session history: {error}");
        return None;
    }
    Some(messages)
}

pub(super) struct FinishState {
    pub perm_prompt_tx: tokio::sync::mpsc::Sender<bool>,
    pub ask_user_tx: tokio::sync::mpsc::Sender<String>,
    pub show_thinking: Arc<AtomicBool>,
    pub session_stats_shared: Arc<tokio::sync::Mutex<SessionStats>>,
    pub last_assistant_response_shared: Arc<tokio::sync::Mutex<String>>,
    /// Which session row writes land in. Shared with the event forwarder here
    /// and with the session loop by the caller, so a resume can move both at
    /// once instead of leaving cost and history in different rows.
    pub active_session: super::active_session::ActiveSessionId,
    /// What automatic consolidation changed at startup, for the splash panel.
    pub garden_summary: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn finish(
    agent: &mut Agent,
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    cli: &Cli,
    config_path: PathBuf,
    working_dir: PathBuf,
    memory: Arc<dyn MemoryTrait>,
    hook_registry: Arc<archon_core::hooks::HookRegistry>,
    governed_learning_db: Option<Arc<cozo::DbInstance>>,
    session_store: Arc<archon_session::storage::SessionStore>,
    tui_event_tx: TuiEventSender,
    agent_event_rx: tokio::sync::mpsc::Receiver<TimestampedEvent>,
    metrics: Arc<archon_tui::observability::ChannelMetrics>,
    cost_alert_state: archon_core::cost_alerts::CostAlertState,
    permission_mode_shared: Arc<tokio::sync::Mutex<String>>,
    agent_def: Option<&archon_core::agents::definition::CustomAgentDefinition>,
    agent_model_for_ledger: String,
    provider_name: String,
    resume_messages: Option<Vec<serde_json::Value>>,
) -> FinishState {
    if archon_consciousness::inner_voice::InnerVoice::is_enabled(config.consciousness.inner_voice) {
        let iv = Arc::new(tokio::sync::Mutex::new(
            archon_consciousness::inner_voice::InnerVoice::with_energy_policy(
                config.consciousness.energy_decay_rate,
                config.consciousness.energy_regen_rate,
                config.consciousness.energy_floor,
            ),
        ));
        agent.set_inner_voice(iv);
    }

    if config.consciousness.persist_personality {
        match archon_consciousness::persistence::load_latest_snapshot(memory.as_ref()) {
            Ok(Some(snap)) => {
                if let Some(iv_arc) = agent.inner_voice() {
                    let mut restored = archon_consciousness::inner_voice::InnerVoice::from_snapshot(
                        snap.inner_voice.clone(),
                    );
                    restored.set_energy_policy(
                        config.consciousness.energy_decay_rate,
                        config.consciousness.energy_regen_rate,
                        config.consciousness.energy_floor,
                    );
                    let restored_confidence = restored.confidence;
                    let restored_energy = restored.energy;
                    *iv_arc.lock().await = restored;
                    tracing::info!(
                        confidence = restored_confidence,
                        energy = restored_energy,
                        snapshot_energy = snap.inner_voice.energy,
                        "personality: restored inner voice from previous session"
                    );
                }
                let engine = archon_consciousness::rules::RulesEngine::new(memory.as_ref());
                match engine.import_scores(&snap.rule_scores) {
                    Ok(n) => tracing::info!(imported = n, "personality: restored rule scores"),
                    Err(e) => tracing::warn!("personality: failed to restore rule scores: {e}"),
                }
            }
            Ok(None) => {
                tracing::debug!("personality: no previous snapshot found (first run)");
            }
            Err(e) => {
                tracing::warn!("personality: failed to load snapshot: {e}");
            }
        }

        if let Ok(trends) = archon_consciousness::persistence::compute_trends(memory.as_ref(), 10)
            && let Ok(Some(last)) =
                archon_consciousness::persistence::load_latest_snapshot(memory.as_ref())
            && trends.total_sessions > 0
        {
            let briefing = archon_consciousness::persistence::generate_briefing(&trends, &last);
            agent.set_personality_briefing(briefing);
            tracing::info!(
                sessions = trends.total_sessions,
                "personality: briefing generated for first turn"
            );
        }
    }

    let mut garden_summary: Option<String> = None;
    if config.memory.enabled && config.memory.garden.auto_consolidate {
        match archon_memory::garden::should_auto_consolidate(
            memory.as_ref(),
            config.memory.garden.min_hours_between_runs,
        ) {
            Ok(true) => {
                tracing::info!("garden: starting auto-consolidation");
                match archon_memory::garden::consolidate_with_run_id(
                    memory.as_ref(),
                    &config.memory.garden,
                    session_id,
                ) {
                    Ok(report) => {
                        tracing::info!(
                            decayed = report.importance_decayed,
                            pruned = report.stale_pruned,
                            deduped = report.duplicates_merged,
                            merged = report.fragments_merged,
                            overflow = report.overflow_pruned,
                            before = report.total_memories_before,
                            after = report.total_memories_after,
                            ms = report.duration_ms,
                            review_pairs = report.review_pairs.len(),
                            "garden: consolidation complete"
                        );
                        // Handed to the splash panel by the caller. This pass
                        // merges and prunes memories on every session start and
                        // its only record was a log line nobody reads -- a
                        // process that quietly reshapes your memory is one whose
                        // mistakes are indistinguishable from it working.
                        garden_summary = auto_consolidation_summary(&report);
                    }
                    Err(e) => tracing::warn!("garden: consolidation failed: {e}"),
                }
            }
            Ok(false) => tracing::debug!("garden: skipping — last run too recent"),
            Err(e) => tracing::warn!("garden: failed to check last run: {e}"),
        }
        match archon_memory::garden::generate_briefing(
            memory.as_ref(),
            config.memory.garden.briefing_limit,
        ) {
            Ok(briefing) if !briefing.is_empty() => {
                agent.set_memory_briefing(briefing);
                tracing::info!("garden: memory briefing generated for first turn");
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("garden: failed to generate briefing: {e}"),
        }
    }

    super::reasoning_quality::maybe_inject_proactive_briefing(
        agent,
        config,
        &working_dir,
        governed_learning_db.as_deref(),
        session_id,
    );

    agent.set_hook_registry(Arc::clone(&hook_registry));
    if let Some(def) = agent_def
        && let Some(ref reminder) = def.critical_system_reminder
    {
        agent.set_critical_system_reminder(reminder.clone());
    }
    let auto_eval = archon_permissions::auto::AutoModeEvaluator::new(
        archon_permissions::auto::AutoModeConfig {
            safe_commands: config.permissions.safe_commands.clone(),
            risky_commands: config.permissions.risky_commands.clone(),
            dangerous_commands: config.permissions.dangerous_commands.clone(),
            allow_paths: config.permissions.allow_paths.clone(),
            deny_paths: config.permissions.deny_paths.clone(),
            project_dir: Some(working_dir.clone()),
        },
    );
    agent.set_auto_evaluator(auto_eval);
    agent.install_subagent_executor();

    let (perm_prompt_tx, perm_prompt_rx) = tokio::sync::mpsc::channel::<bool>(1);
    agent.permission_response_rx = Some(Arc::new(tokio::sync::Mutex::new(perm_prompt_rx)));
    let (ask_user_tx, ask_user_rx) = tokio::sync::mpsc::channel::<String>(1);
    agent.ask_user_response_rx = Some(Arc::new(tokio::sync::Mutex::new(ask_user_rx)));

    let active_session = super::active_session::ActiveSessionId::new(session_id);

    if let Some(messages) = resume_messages {
        let count = messages.len();
        let replayed = replay_resumed_conversation(&tui_event_tx, messages).await;
        if let Some(messages) = replayed {
            agent.restore_conversation(messages);
            tracing::info!("restored {count} messages from previous session");
        }
        if let Some(Some(ref resume_id)) = cli.resume
            && let Ok(meta) = session_store.get_session(resume_id)
        {
            // Continue the session that was resumed rather than the row this
            // launch minted. Without this the new row silently takes over:
            // `post_turn` writes the whole restored conversation under the new
            // id and the resumed row stays frozen at whatever it last held.
            //
            // `meta.id`, not `resume_id` -- the flag accepts an id prefix, and
            // a prefix is not a key.
            active_session.set(&meta.id);
            if let Some(name) = meta.name
                && let Err(error) = tui_event_tx
                    .send_async(TuiEvent::SessionRenamed(name))
                    .await
            {
                tracing::warn!(%error, "resumed session name delivery failed");
            }
        }
        if archon_consciousness::inner_voice::InnerVoice::is_enabled(
            config.consciousness.inner_voice,
        ) && let Some(m) = latest_inner_voice_snapshot(memory.as_ref())
            && let Ok(snapshot) = serde_json::from_str::<
                archon_consciousness::inner_voice::InnerVoiceSnapshot,
            >(&m.content)
        {
            let mut restored =
                archon_consciousness::inner_voice::InnerVoice::from_snapshot(snapshot);
            restored.set_energy_policy(
                config.consciousness.energy_decay_rate,
                config.consciousness.energy_regen_rate,
                config.consciousness.energy_floor,
            );
            let iv = Arc::new(tokio::sync::Mutex::new(restored));
            agent.set_inner_voice(iv);
            tracing::info!("inner voice state restored from snapshot");
        }
    }

    if cli.fork_session && cli.resume.is_some() {
        let fork_name = cli.session_name.as_deref();
        match archon_session::fork::fork_session(&session_store, session_id, fork_name) {
            Ok(new_id) => {
                eprintln!("Forked session as: {}", &new_id[..8.min(new_id.len())]);
            }
            Err(e) => {
                tracing::warn!("fork-session failed: {e}");
            }
        }
    }

    let show_thinking = Arc::clone(&agent.show_thinking);
    let session_stats_shared = Arc::clone(&agent.session_stats);

    super::config_watcher::spawn_config_watcher(
        config_path,
        config.clone(),
        tui_event_tx.clone(),
        Arc::clone(&hook_registry),
        working_dir,
        session_id.to_string(),
    );

    let last_assistant_response_shared = super::event_forwarder::spawn_agent_event_forwarder(
        super::event_forwarder::AgentEventForwarderConfig {
            event_rx: agent_event_rx,
            metrics,
            tui_tx: tui_event_tx,
            session_stats: Arc::clone(&session_stats_shared),
            cost_alert_state,
            cost_config: config.cost.clone(),
            active_session: active_session.clone(),
            session_store: Arc::clone(&session_store),
            permission_mode: Arc::clone(&permission_mode_shared),
            permission_events_db: governed_learning_db.clone(),
            agent_ledger_db: governed_learning_db,
            ledger_context: super::agent_ledger::context(
                session_id,
                agent_def,
                agent_model_for_ledger.clone(),
                provider_name,
            ),
            selected_model: agent_model_for_ledger,
        },
    );

    FinishState {
        perm_prompt_tx,
        ask_user_tx,
        show_thinking,
        session_stats_shared,
        last_assistant_response_shared,
        active_session,
        garden_summary,
    }
}

#[cfg(test)]
#[path = "interactive_finish_tests.rs"]
mod tests;
