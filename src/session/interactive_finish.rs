use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::cli_args::Cli;
use archon_core::agent::{Agent, SessionStats, TimestampedEvent};
use archon_memory::MemoryTrait;
use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;

pub(super) struct FinishState {
    pub perm_prompt_tx: tokio::sync::mpsc::Sender<bool>,
    pub ask_user_tx: tokio::sync::mpsc::Sender<String>,
    pub show_thinking: Arc<AtomicBool>,
    pub session_stats_shared: Arc<tokio::sync::Mutex<SessionStats>>,
    pub last_assistant_response_shared: Arc<tokio::sync::Mutex<String>>,
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
    agent_event_rx: tokio::sync::mpsc::UnboundedReceiver<TimestampedEvent>,
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

    if config.memory.enabled && config.memory.garden.auto_consolidate {
        match archon_memory::garden::should_auto_consolidate(
            memory.as_ref(),
            config.memory.garden.min_hours_between_runs,
        ) {
            Ok(true) => {
                tracing::info!("garden: starting auto-consolidation");
                match archon_memory::garden::consolidate(memory.as_ref(), &config.memory.garden) {
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
                            "garden: consolidation complete"
                        );
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
            project_dir: Some(working_dir.clone()),
            ..Default::default()
        },
    );
    agent.set_auto_evaluator(auto_eval);
    agent.install_subagent_executor();

    let (perm_prompt_tx, perm_prompt_rx) = tokio::sync::mpsc::channel::<bool>(1);
    agent.permission_response_rx = Some(Arc::new(tokio::sync::Mutex::new(perm_prompt_rx)));
    let (ask_user_tx, ask_user_rx) = tokio::sync::mpsc::channel::<String>(1);
    agent.ask_user_response_rx = Some(Arc::new(tokio::sync::Mutex::new(ask_user_rx)));

    if let Some(messages) = resume_messages {
        let count = messages.len();
        agent.restore_conversation(messages);
        tracing::info!("restored {count} messages from previous session");
        if let Some(Some(ref resume_id)) = cli.resume
            && let Ok(meta) = session_store.get_session(resume_id)
            && let Some(name) = meta.name
        {
            let _ = tui_event_tx.send(TuiEvent::SessionRenamed(name));
        }
        if archon_consciousness::inner_voice::InnerVoice::is_enabled(
            config.consciousness.inner_voice,
        ) && let Ok(memories) = memory.recall_memories("inner_voice_snapshot", 1)
            && let Some(m) = memories.first()
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
            session_id: session_id.to_string(),
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
    }
}
