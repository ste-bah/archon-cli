//! Proactive first-turn briefing assembled from existing learning systems.

use std::path::Path;

use anyhow::Result;

pub(crate) fn build_session_briefing(
    config: &archon_core::config::ArchonConfig,
    policy: &archon_policy::models::EffectivePolicy,
    reasoning_root: Option<&Path>,
    learning_db: Option<&cozo::DbInstance>,
    world_root: Option<&Path>,
    session_id: &str,
    task_hint: Option<&str>,
) -> Option<String> {
    if !config.learning.session_briefing.enabled
        || !policy.reasoning_quality.allow_session_start_injection
    {
        return None;
    }

    let mut sections = Vec::new();
    if config.learning.session_briefing.include_reasoning_quality
        && let Some(root) = reasoning_root
        && let Ok(section) =
            reasoning_section(root, task_hint, config.learning.session_briefing.max_items)
        && !section.is_empty()
    {
        sections.push(section);
    }
    if config
        .learning
        .session_briefing
        .include_pending_behaviour_proposals
        && policy.reasoning_quality.allow_behavior_proposal_generation
        && let Some(db) = learning_db
        && let Ok(section) = proposal_section(db, config.learning.session_briefing.max_items)
        && !section.is_empty()
    {
        sections.push(section);
    }
    if config.learning.session_briefing.include_world_model
        && let Some(root) = world_root
        && let Ok(section) = world_model_section(config, root, session_id, task_hint)
        && !section.is_empty()
    {
        sections.push(section);
    }
    if let Some(section) = shadow_hold_section(config) {
        sections.push(section);
    }

    if sections.is_empty() {
        return None;
    }
    if let Some(root) = reasoning_root {
        let _ = append_briefing_summary(root, session_id, sections.len(), task_hint);
    }
    let mut briefing = format!(
        "<proactive_session_briefing applies_to=\"first_turn\" session_id=\"{session_id}\">\n{}\n</proactive_session_briefing>",
        sections.join("\n\n")
    );
    truncate_chars(&mut briefing, config.learning.session_briefing.max_chars);
    Some(briefing)
}

fn append_briefing_summary(
    root: &Path,
    session_id: &str,
    section_count: usize,
    task_hint: Option<&str>,
) -> Result<()> {
    let path = root.join("briefings").join("summaries.jsonl");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    serde_json::to_writer(
        &mut file,
        &serde_json::json!({
            "session_id": session_id,
            "section_count": section_count,
            "task_hint_hash": task_hint.map(archon_reasoning_quality::hash_hex),
            "created_at": chrono::Utc::now().to_rfc3339(),
        }),
    )?;
    use std::io::Write;
    file.write_all(b"\n")?;
    Ok(())
}

fn shadow_hold_section(config: &archon_core::config::ArchonConfig) -> Option<String> {
    if !config.learning.reasoning_quality.enabled
        || config.learning.reasoning_quality.shadow_mode_days == 0
    {
        return None;
    }
    Some(format!(
        "Reasoning-quality shadow mode: trust updates are held for {} days. Run `archon reasoning shadow-report`; if labels are missing, run `archon reasoning sample-label <session-id>`.",
        config.learning.reasoning_quality.shadow_mode_days
    ))
}

fn reasoning_section(root: &Path, task_hint: Option<&str>, max_items: usize) -> Result<String> {
    let store = archon_reasoning_quality::store::ReasoningQualityStore::open(root)?;
    let mut scored: Vec<_> = store
        .recent_events(100)?
        .into_iter()
        .filter(|event| event.severity_effective >= 0.4)
        .filter(|event| {
            !matches!(
                event.event_kind,
                archon_reasoning_quality::ReasoningEventKind::SourceVerifiedClaim
                    | archon_reasoning_quality::ReasoningEventKind::UncertaintyDisclosed
                    | archon_reasoning_quality::ReasoningEventKind::ClaimEmitted
            )
        })
        .map(|event| (briefing_score(&event, task_hint), event))
        .collect();
    scored.sort_by(|left, right| right.0.total_cmp(&left.0));
    scored.truncate(max_items.min(3));
    if scored.is_empty() {
        return Ok(String::new());
    }

    let mut out = String::from("Reasoning-quality warnings:\n");
    for (_score, event) in scored {
        out.push_str(&format!(
            "- {:?} on {:?}: {}\n",
            event.event_kind,
            event.subject,
            event
                .redacted_excerpt
                .as_deref()
                .unwrap_or(event.canonical_text.as_str())
        ));
    }
    Ok(out.trim_end().to_string())
}

fn proposal_section(db: &cozo::DbInstance, max_items: usize) -> Result<String> {
    let mut proposals = archon_learning::store::list_behaviour_proposals(db, Some("Pending"))?;
    proposals.truncate(max_items.min(3));
    if proposals.is_empty() {
        return Ok(String::new());
    }
    let mut out = String::from("Pending behaviour proposals:\n");
    for proposal in proposals {
        out.push_str(&format!(
            "- {} {} risk={} decision={}\n",
            proposal.proposal_id,
            proposal.manifest_kind.as_str(),
            proposal.risk_level.as_str(),
            proposal.policy_decision.as_str(),
        ));
    }
    Ok(out.trim_end().to_string())
}

fn world_model_section(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    session_id: &str,
    task_hint: Option<&str>,
) -> Result<String> {
    let store = archon_world_model::storage::WorldModelStore::open(root)?;
    let stats = store.cold_start_stats()?;
    let thresholds = archon_world_model::ColdStartThresholds {
        min_rows: config.learning.world_model.cold_start.min_rows,
        min_sessions: config.learning.world_model.cold_start.min_sessions,
        min_observed_days: config.learning.world_model.cold_start.min_observed_days,
    };
    let status = archon_world_model::trace::evaluate_cold_start(stats, thresholds);
    let summary = task_hint.unwrap_or("session start");
    let advisor = archon_world_model::WorldAdvisor::new(
        archon_world_model::WorldAdvisorConfig {
            thresholds,
            active_model_id: None,
            training_in_progress: false,
        },
        stats,
    );
    let decision = advisor.evaluate(&archon_world_model::WorldAdvisorContext {
        session_id: session_id.to_string(),
        action_ref: "session_start".to_string(),
        action_summary: summary.to_string(),
    });
    let advisor_status = decision
        .unavailable
        .map(|event| format!("{:?}", event.reason))
        .unwrap_or_else(|| "ready".to_string());
    Ok(format!(
        "World-model advisory: corpus rows={} sessions={} status={:?} advisor={advisor_status}",
        stats.rows, stats.sessions, status
    ))
}

fn briefing_score(
    event: &archon_reasoning_quality::ReasoningQualityEvent,
    task_hint: Option<&str>,
) -> f32 {
    let severity = event.severity_effective.clamp(0.0, 1.0);
    let age_days = (chrono::Utc::now() - event.created_at).num_hours().max(0) as f32 / 24.0;
    let recency = 1.0 / (1.0 + age_days / 7.0);
    let relevance = task_relevance(event, task_hint);
    0.40 * severity + 0.30 * recency + 0.30 * relevance
}

fn task_relevance(
    event: &archon_reasoning_quality::ReasoningQualityEvent,
    task_hint: Option<&str>,
) -> f32 {
    let Some(task) = task_hint else { return 0.2 };
    let task = task.to_lowercase();
    if !event.entity_key.is_empty() && task.contains(&event.entity_key.to_lowercase()) {
        1.0
    } else if task.contains(&event.subject_string().to_lowercase()) {
        0.7
    } else {
        0.2
    }
}

trait ReasoningSubjectString {
    fn subject_string(&self) -> String;
}

impl ReasoningSubjectString for archon_reasoning_quality::ReasoningQualityEvent {
    fn subject_string(&self) -> String {
        format!("{:?}", self.subject)
    }
}

fn truncate_chars(text: &mut String, max_chars: usize) {
    if text.chars().count() <= max_chars {
        return;
    }
    *text = text.chars().take(max_chars.saturating_sub(32)).collect();
    text.push_str("\n[briefing truncated]");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_prefers_relevant_severe_events() {
        let event = archon_reasoning_quality::ReasoningQualityEvent {
            event_kind: archon_reasoning_quality::ReasoningEventKind::ClaimBeforeSourceRead,
            entity_key: "src/lib.rs".into(),
            severity_effective: 0.7,
            created_at: chrono::Utc::now(),
            ..archon_reasoning_quality::ReasoningQualityEvent::default()
        };
        assert!(briefing_score(&event, Some("check src/lib.rs")) > 0.8);
    }
}
