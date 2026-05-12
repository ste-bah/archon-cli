use std::path::{Path, PathBuf};
use std::sync::Arc;

use archon_core::agent::{
    Agent, ReasoningEvidenceEventPayload, ReasoningTurnEventPayload, UserCorrectionEventPayload,
};
use archon_llm::provider::LlmProvider;
use archon_pipeline::learning::integration::LearningIntegration;

pub(super) fn quality_root() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".archon").join("reasoning-quality"))
}

fn world_model_root() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".archon").join("world-model"))
}

fn session_briefing_context(working_dir: &Path) -> String {
    let mut parts = vec![format!("cwd={}", working_dir.display())];
    let head_path = working_dir.join(".git").join("HEAD");
    if let Ok(head) = std::fs::read_to_string(&head_path) {
        let head = head.trim();
        if let Some(branch) = head.strip_prefix("ref: refs/heads/") {
            parts.push(format!("branch={branch}"));
            if let Ok(sha) = std::fs::read_to_string(working_dir.join(".git").join(head)) {
                parts.push(format!(
                    "git_head={}",
                    sha.trim().chars().take(12).collect::<String>()
                ));
            }
        } else if !head.is_empty() {
            parts.push(format!(
                "git_head={}",
                head.chars().take(12).collect::<String>()
            ));
        }
    }
    if dirs::home_dir()
        .map(|home| home.join(".archon").join("sessions").exists())
        .unwrap_or(false)
    {
        parts.push("recent_activity=available".to_string());
    }
    parts.join(" ")
}

fn shadow_active(root: &Path, shadow_mode_days: u32) -> bool {
    if shadow_mode_days == 0 {
        return false;
    }
    let path = root.join("shadow").join("started_at");
    let now = chrono::Utc::now();
    if let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(started) = chrono::DateTime::parse_from_rfc3339(text.trim())
    {
        return (now - started.with_timezone(&chrono::Utc)).num_days() < shadow_mode_days as i64;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, now.to_rfc3339());
    true
}

fn map_evidence(
    payload: ReasoningEvidenceEventPayload,
    redaction: &archon_reasoning_quality::RedactionConfig,
) -> archon_reasoning_quality::EvidenceRef {
    archon_reasoning_quality::EvidenceRef {
        evidence_id: payload.evidence_id,
        kind: match payload.kind.as_str() {
            "file_read" => archon_reasoning_quality::EvidenceKind::FileRead,
            "search" => archon_reasoning_quality::EvidenceKind::Search,
            "git" => archon_reasoning_quality::EvidenceKind::Git,
            "test_output" => archon_reasoning_quality::EvidenceKind::TestOutput,
            "memory" => archon_reasoning_quality::EvidenceKind::Memory,
            "mcp_result" => archon_reasoning_quality::EvidenceKind::McpResult,
            "plugin_result" => archon_reasoning_quality::EvidenceKind::PluginResult,
            "pipeline_artifact" => archon_reasoning_quality::EvidenceKind::PipelineArtifact,
            _ => archon_reasoning_quality::EvidenceKind::ChatHistory,
        },
        entity_key: payload
            .entity_key
            .map(|key| archon_reasoning_quality::redact_entity_key(&key, redaction)),
        output_hash: payload.output_hash,
        redacted_excerpt: payload
            .redacted_excerpt
            .map(|text| archon_reasoning_quality::redact_text(&text, redaction)),
        created_at: chrono::DateTime::parse_from_rfc3339(&payload.created_at)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now()),
    }
}

fn record_turn(
    payload: ReasoningTurnEventPayload,
    root: &Path,
    cfg: &archon_core::config::ReasoningQualityConfig,
    store_raw_text: bool,
    learning_db: Option<&cozo::DbInstance>,
    world_root: Option<&Path>,
    shadow: bool,
    critic_runtime: Option<crate::runtime::reasoning_critic::CriticRuntime>,
) {
    let redaction = archon_reasoning_quality::RedactionConfig {
        allow_raw_text: store_raw_text,
        workspace_root: payload.workspace_root.clone(),
        max_excerpt_chars: cfg.max_excerpt_chars,
        ..archon_reasoning_quality::RedactionConfig::default()
    };
    let mut input = archon_reasoning_quality::ReasoningTurnInput {
        session_id: payload.session_id,
        turn_number: payload.turn_number,
        assistant_text: payload.assistant_text,
        evidence_refs: payload
            .evidence_refs
            .into_iter()
            .map(|evidence| map_evidence(evidence, &redaction))
            .collect(),
        cwd: payload.cwd,
        workspace_root: payload.workspace_root,
        store_raw_text,
    };
    if let Ok(store) = archon_reasoning_quality::store::ReasoningQualityStore::open(root)
        && let Ok(prior_events) = store.events_for_session(&input.session_id)
    {
        input.evidence_refs.extend(
            prior_events
                .into_iter()
                .filter(|event| {
                    event.event_kind
                        == archon_reasoning_quality::ReasoningEventKind::SourceVerifiedClaim
                })
                .map(|event| archon_reasoning_quality::EvidenceRef {
                    evidence_id: event.event_id,
                    kind: archon_reasoning_quality::EvidenceKind::PriorVerifiedClaim,
                    entity_key: Some(event.entity_key),
                    output_hash: event.raw_text_hash,
                    redacted_excerpt: event.redacted_excerpt,
                    created_at: event.created_at,
                }),
        );
    }
    let extractor = archon_reasoning_quality::DeterministicExtractor::new(
        archon_reasoning_quality::ExtractorConfig {
            max_claims_per_turn: cfg.max_claims_per_turn,
            max_excerpt_chars: cfg.max_excerpt_chars,
            shadow,
        },
    );
    let mut events = extractor.extract_turn(&input);
    if input.turn_number == 1 {
        events.push(first_turn_briefing_event(
            &input.session_id,
            input.turn_number,
            shadow,
        ));
    }
    if events.is_empty() && input.evidence_refs.is_empty() {
        return;
    }
    match archon_reasoning_quality::store::ReasoningQualityStore::open(root) {
        Ok(store) => {
            if let Ok(prior) = store.events_for_session(&input.session_id) {
                events.extend(archon_reasoning_quality::build_superseding_source_events(
                    &prior,
                    &input.evidence_refs,
                ));
            }
            if events.is_empty() {
                return;
            }
            if let Err(e) = store.append_events(&events) {
                tracing::warn!(error = %e, "reasoning-quality event write failed");
            }
            crate::runtime::reasoning_quality::bridge_reasoning_events(
                &events,
                learning_db,
                root,
                world_root,
                cfg.feed_world_model,
                cfg.update_self_trust,
            );
            crate::runtime::reasoning_critic::spawn_critic_if_enabled(critic_runtime, events);
        }
        Err(e) => tracing::warn!(error = %e, "reasoning-quality store unavailable"),
    }
}

fn first_turn_briefing_event(
    session_id: &str,
    turn_number: u64,
    shadow: bool,
) -> archon_reasoning_quality::ReasoningQualityEvent {
    let claim_id = archon_reasoning_quality::claim_id_for(
        session_id,
        turn_number,
        "briefing updated with task context",
        archon_reasoning_quality::ReasoningSubject::GeneralReasoning,
        "session_briefing",
    );
    archon_reasoning_quality::ReasoningQualityEvent {
        event_id: archon_reasoning_quality::event_id_for(
            session_id,
            turn_number,
            &claim_id,
            "briefing_updated_with_task_context",
        ),
        session_id: session_id.to_string(),
        turn_number,
        claim_id,
        event_kind: archon_reasoning_quality::ReasoningEventKind::BriefingUpdatedWithTaskContext,
        subject: archon_reasoning_quality::ReasoningSubject::GeneralReasoning,
        entity_key: "session_briefing".to_string(),
        canonicalizer_version: archon_reasoning_quality::CANONICALIZER_VERSION.to_string(),
        canonical_text: "briefing updated with task context".to_string(),
        verification_state: archon_reasoning_quality::VerificationState::NotRequired,
        source_system: "proactive_session_briefing".to_string(),
        shadow,
        created_at: chrono::Utc::now(),
        ..archon_reasoning_quality::ReasoningQualityEvent::default()
    }
}

fn record_user_correction(
    session_id: &str,
    payload: UserCorrectionEventPayload,
    root: &Path,
    cfg: &archon_core::config::ReasoningQualityConfig,
    store_raw_text: bool,
    learning_db: Option<&cozo::DbInstance>,
    world_root: Option<&Path>,
    shadow: bool,
) {
    let turn_number = payload
        .session_context
        .strip_prefix("turn:")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let redaction = archon_reasoning_quality::RedactionConfig {
        allow_raw_text: store_raw_text,
        max_excerpt_chars: cfg.max_excerpt_chars,
        ..archon_reasoning_quality::RedactionConfig::default()
    };
    match archon_reasoning_quality::store::ReasoningQualityStore::open(root) {
        Ok(store) => {
            let prior_events = match store.events_for_session(session_id) {
                Ok(events) => events,
                Err(e) => {
                    tracing::warn!(error = %e, "reasoning-quality correction lookup failed");
                    return;
                }
            };
            let Some(event) = archon_reasoning_quality::build_user_correction_event(
                session_id,
                turn_number,
                &payload.user_input_excerpt,
                &prior_events,
                &redaction,
                shadow,
            ) else {
                return;
            };
            if let Err(e) = store.append_events(std::slice::from_ref(&event)) {
                tracing::warn!(error = %e, "reasoning-quality correction write failed");
                return;
            }
            crate::runtime::reasoning_quality::bridge_reasoning_events(
                &[event],
                learning_db,
                root,
                world_root,
                cfg.feed_world_model,
                cfg.update_self_trust,
            );
        }
        Err(e) => tracing::warn!(error = %e, "reasoning-quality store unavailable for correction"),
    }
}

pub(super) fn wire_callbacks(
    agent: &mut Agent,
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    working_dir: &Path,
    governed_learning_db: Option<Arc<cozo::DbInstance>>,
    correction_learning: Option<Arc<LearningIntegration>>,
    provider: Arc<dyn LlmProvider>,
) {
    let correction_root = if config.learning.reasoning_quality.enabled
        && config.learning.reasoning_quality.emit_inline_events
    {
        quality_root()
    } else {
        None
    };
    if correction_learning.is_some() || correction_root.is_some() {
        let correction_learning_cb = correction_learning;
        let session_id_for_correction = session_id.to_string();
        let rq_cfg_for_correction = config.learning.reasoning_quality.clone();
        let reasoning_learning_db = governed_learning_db.clone();
        let correction_world_root = world_model_root();
        let allow_raw_text = archon_policy::load_effective_policy(working_dir)
            .map(|policy| policy.reasoning_quality.allow_raw_text_storage)
            .unwrap_or(false);
        let correction_cb: Arc<dyn Fn(UserCorrectionEventPayload) + Send + Sync> =
            Arc::new(move |payload| {
                if let Some(learning) = &correction_learning_cb {
                    learning.record_user_correction_event(payload.clone());
                }
                if let Some(root) = &correction_root {
                    record_user_correction(
                        &session_id_for_correction,
                        payload,
                        root,
                        &rq_cfg_for_correction,
                        rq_cfg_for_correction.store_raw_text && allow_raw_text,
                        reasoning_learning_db.as_deref(),
                        correction_world_root.as_deref(),
                        shadow_active(root, rq_cfg_for_correction.shadow_mode_days),
                    );
                }
            });
        agent.set_record_user_correction_event_callback(correction_cb);
    }

    if config.learning.reasoning_quality.enabled
        && config.learning.reasoning_quality.emit_inline_events
        && let Some(root) = quality_root()
    {
        let rq_cfg = config.learning.reasoning_quality.clone();
        let learning_db = governed_learning_db;
        let world_root = world_model_root();
        let policy_for_reasoning = archon_policy::load_effective_policy(working_dir).ok();
        let allow_raw_text = policy_for_reasoning
            .as_ref()
            .map(|policy| policy.reasoning_quality.allow_raw_text_storage)
            .unwrap_or(false);
        let critic_model_fallback = super::active_session_model(config);
        let reasoning_cb: Arc<dyn Fn(ReasoningTurnEventPayload) + Send + Sync> =
            Arc::new(move |payload| {
                let critic_runtime = policy_for_reasoning.clone().map(|policy| {
                    crate::runtime::reasoning_critic::CriticRuntime {
                        enabled: rq_cfg.post_turn_analysis,
                        config: rq_cfg.critic.clone(),
                        policy,
                        provider: Arc::clone(&provider),
                        model_fallback: critic_model_fallback.clone(),
                        root: root.clone(),
                        learning_db: learning_db.clone(),
                        world_root: world_root.clone(),
                        feed_world_model: rq_cfg.feed_world_model,
                        update_self_trust: rq_cfg.update_self_trust,
                    }
                });
                record_turn(
                    payload,
                    &root,
                    &rq_cfg,
                    rq_cfg.store_raw_text && allow_raw_text,
                    learning_db.as_deref(),
                    world_root.as_deref(),
                    shadow_active(&root, rq_cfg.shadow_mode_days),
                    critic_runtime,
                )
            });
        agent.set_record_reasoning_turn_callback(reasoning_cb);
        tracing::info!("reasoning-quality: visible turn event capture wired");
    }
}

pub(super) fn maybe_inject_proactive_briefing(
    agent: &mut Agent,
    config: &archon_core::config::ArchonConfig,
    working_dir: &Path,
    governed_learning_db: Option<&cozo::DbInstance>,
    session_id: &str,
) {
    match archon_policy::load_effective_policy(working_dir) {
        Ok(policy) => {
            let reasoning_root = quality_root();
            let world_root = world_model_root();
            let briefing_context = session_briefing_context(working_dir);
            if let Some(briefing) = crate::runtime::proactive_briefing::build_session_briefing(
                config,
                &policy,
                reasoning_root.as_deref(),
                governed_learning_db,
                world_root.as_deref(),
                session_id,
                Some(&briefing_context),
            ) {
                let combined = match agent.memory_briefing.take() {
                    Some(existing) if !existing.trim().is_empty() => {
                        format!("{existing}\n\n{briefing}")
                    }
                    _ => briefing,
                };
                agent.set_memory_briefing(combined);
                tracing::info!("reasoning-quality: proactive session briefing generated");
            }
        }
        Err(e) => tracing::debug!(error = %e, "proactive briefing policy load failed"),
    }
}
