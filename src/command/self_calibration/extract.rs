use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_llm::provider::{LlmProvider, LlmRequest};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::runtime::llm::build_configured_llm_provider;

const MAX_LLM_EVENTS: usize = 120;
const MAX_EVENT_MESSAGE_CHARS: usize = 420;
const MAX_PROMPT_CHARS: usize = 32_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AnalyzerMode {
    Heuristic,
    Llm,
    Hybrid,
}

impl AnalyzerMode {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            AnalyzerMode::Heuristic => "heuristic",
            AnalyzerMode::Llm => "llm",
            AnalyzerMode::Hybrid => "hybrid",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct CandidateExtraction {
    pub(super) analyzer: &'static str,
    pub(super) candidates: Vec<RetrospectiveCandidate>,
    pub(super) notes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct RetrospectiveCandidate {
    pub(super) category: String,
    pub(super) domain: String,
    pub(super) content: String,
    pub(super) confidence: f32,
    pub(super) evidence_event_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
struct CompactActivityEvent {
    event_id: String,
    kind: String,
    status: String,
    message: String,
    provider: Option<String>,
    model: Option<String>,
    agent_key: Option<String>,
    subagent_type: Option<String>,
    artifact_id: Option<String>,
    created_at: String,
}

#[derive(Debug, Deserialize)]
struct LlmCandidateEnvelope {
    candidates: Vec<LlmCandidate>,
}

#[derive(Debug, Deserialize)]
struct LlmCandidate {
    category: String,
    domain: String,
    content: String,
    confidence: f32,
    evidence_event_ids: Vec<String>,
}

pub(super) async fn extract_candidates(
    events: &[archon_observability::AgentActivityEvent],
    mode: AnalyzerMode,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> CandidateExtraction {
    let mut notes = Vec::new();
    let mut candidates = Vec::new();
    let heuristic = heuristic_candidates(events);

    if matches!(mode, AnalyzerMode::Heuristic | AnalyzerMode::Hybrid) {
        notes.push(format!(
            "heuristic analyzer proposed {} candidate(s)",
            heuristic.len()
        ));
        candidates.extend(heuristic.clone());
    }

    if matches!(mode, AnalyzerMode::Llm | AnalyzerMode::Hybrid) {
        match llm_candidates(events, config, env_vars).await {
            Ok(llm) => {
                notes.push(format!("llm analyzer accepted {} candidate(s)", llm.len()));
                candidates.extend(llm);
            }
            Err(error) => {
                notes.push(format!(
                    "llm analyzer unavailable; using deterministic candidates: {error}"
                ));
                if matches!(mode, AnalyzerMode::Llm) {
                    candidates.extend(heuristic);
                }
            }
        }
    }

    CandidateExtraction {
        analyzer: mode.as_str(),
        candidates: dedupe_candidates(candidates),
        notes,
    }
}

fn heuristic_candidates(
    events: &[archon_observability::AgentActivityEvent],
) -> Vec<RetrospectiveCandidate> {
    let mut out = Vec::new();
    for event in events {
        let message = event.message.to_lowercase();
        let (category, domain, content, confidence) = if message.contains("there is no")
            || message.contains("no such file")
            || message.contains("not found")
            || message.contains("wrong source")
            || message.contains("wrong repo")
        {
            (
                "source_tree_mistake",
                "rust-codebase-analysis",
                "Verify the actual source tree before making architecture or path claims.",
                0.90,
            )
        } else if matches!(
            event.kind,
            archon_observability::AgentActivityKind::ToolFailed
                | archon_observability::AgentActivityKind::AgentFailed
        ) {
            (
                "bug_pattern",
                "provider-debugging",
                "Treat failed tools and failed agents as learning evidence, not disposable noise.",
                0.75,
            )
        } else if message.contains("test failed") || message.contains("verify failed") {
            (
                "verification_habit",
                "rust-codebase-analysis",
                "When verification fails, report the concrete failing command and evidence.",
                0.80,
            )
        } else {
            continue;
        };
        out.push(RetrospectiveCandidate {
            category: category.into(),
            domain: domain.into(),
            content: content.into(),
            confidence,
            evidence_event_ids: vec![event.event_id.clone()],
        });
    }
    out
}

async fn llm_candidates(
    events: &[archon_observability::AgentActivityEvent],
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> anyhow::Result<Vec<RetrospectiveCandidate>> {
    let provider = build_configured_llm_provider(config, env_vars, "self-retrospective").await?;
    let request = build_llm_request(events, provider.as_ref(), config)?;
    let response = provider.complete(request).await?;
    let text = response_text(&response.content);
    let envelope = parse_candidate_envelope(&text)?;
    Ok(validate_llm_candidates(envelope, events))
}

fn build_llm_request(
    events: &[archon_observability::AgentActivityEvent],
    provider: &dyn LlmProvider,
    config: &ArchonConfig,
) -> anyhow::Result<LlmRequest> {
    let compact = compact_events(events);
    let compact_json = serde_json::to_string_pretty(&compact)?;
    let prompt = truncate_chars(
        &format!(
            "Analyze this Archon session activity excerpt and extract reusable retrospective learnings.\n\
             Return strict JSON only with this shape:\n\
             {{\"candidates\":[{{\"category\":\"snake_case\",\"domain\":\"kebab-case\",\"content\":\"imperative reusable lesson\",\"confidence\":0.0,\"evidence_event_ids\":[\"event-id\"]}}]}}\n\n\
             Rules:\n\
             - Return at most 5 candidates.\n\
             - Each candidate must cite real event_id values from the excerpt.\n\
             - Prefer lessons that would improve future behavior across sessions.\n\
             - Do not include secrets, raw commands with credentials, or one-off trivia.\n\
             - Good categories include memory_miss, planning_drift, documentation_overclaim,\n\
               user_preference_violation, environment_assumption, provider_auth_failure,\n\
               tool_permission_block, ci_failure_pattern, stale_context_claim,\n\
               verification_gap, interruption_recovery, and handoff_or_resume_gap.\n\n\
             Activity excerpt:\n{compact_json}"
        ),
        MAX_PROMPT_CHARS,
    );
    let model = retrospective_model(config, provider);
    Ok(LlmRequest {
        model,
        max_tokens: 1600,
        system: vec![serde_json::json!({
            "type": "text",
            "text": "You are Archon's retrospective analyzer. You only produce validated JSON candidates for durable self-calibration. Do not invent evidence ids."
        })],
        messages: vec![serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": prompt}]
        })],
        effort: Some("low".into()),
        request_origin: Some("self_retrospective".into()),
        ..LlmRequest::default()
    })
}

fn retrospective_model(config: &ArchonConfig, provider: &dyn LlmProvider) -> String {
    if provider.name() == "openai-codex"
        && let Some(model) = crate::runtime::codex_model::codex_model_for_anthropic_default(config)
    {
        return model;
    }
    config.api.default_model.clone()
}

fn compact_events(
    events: &[archon_observability::AgentActivityEvent],
) -> Vec<CompactActivityEvent> {
    let mut selected = Vec::new();
    for event in events.iter().rev() {
        if selected.len() >= MAX_LLM_EVENTS {
            break;
        }
        if selected.len() < 40 || interesting_event(event) {
            selected.push(CompactActivityEvent {
                event_id: event.event_id.clone(),
                kind: format!("{:?}", event.kind),
                status: format!("{:?}", event.status),
                message: truncate_chars(
                    &redact_inline_secrets(&event.message),
                    MAX_EVENT_MESSAGE_CHARS,
                ),
                provider: event.provider.clone(),
                model: event.model.clone(),
                agent_key: event.agent_key.clone(),
                subagent_type: event.subagent_type.clone(),
                artifact_id: event.artifact_id.clone(),
                created_at: event.created_at.to_rfc3339(),
            });
        }
    }
    selected.reverse();
    selected
}

fn interesting_event(event: &archon_observability::AgentActivityEvent) -> bool {
    matches!(
        event.kind,
        archon_observability::AgentActivityKind::ToolFailed
            | archon_observability::AgentActivityKind::AgentFailed
            | archon_observability::AgentActivityKind::AgentWaitingPermission
            | archon_observability::AgentActivityKind::AgentWaitingProvider
            | archon_observability::AgentActivityKind::MemorySurfaced
            | archon_observability::AgentActivityKind::ArtifactCreated
            | archon_observability::AgentActivityKind::Cancelled
    ) || matches!(
        event.status,
        archon_observability::AgentActivityStatus::Failed
            | archon_observability::AgentActivityStatus::Waiting
            | archon_observability::AgentActivityStatus::Cancelled
    ) || event.message.to_lowercase().contains("correct")
        || event.message.to_lowercase().contains("failed")
        || event.message.to_lowercase().contains("permission")
        || event.message.to_lowercase().contains("not found")
}

fn parse_candidate_envelope(text: &str) -> anyhow::Result<LlmCandidateEnvelope> {
    let trimmed = strip_json_fence(text.trim());
    serde_json::from_str(trimmed)
        .or_else(|_| extract_json_object(trimmed).and_then(|body| serde_json::from_str(body)))
        .map_err(|error| anyhow::anyhow!("parse llm retrospective JSON: {error}"))
}

fn strip_json_fence(text: &str) -> &str {
    let Some(stripped) = text.strip_prefix("```") else {
        return text;
    };
    let after_language = stripped
        .strip_prefix("json")
        .or_else(|| stripped.strip_prefix("JSON"))
        .unwrap_or(stripped)
        .trim_start_matches('\n')
        .trim();
    after_language
        .strip_suffix("```")
        .unwrap_or(after_language)
        .trim()
}

fn extract_json_object(text: &str) -> Result<&str, serde_json::Error> {
    let Some(start) = text.find('{') else {
        return serde_json::from_str::<Value>(text).map(|_| text);
    };
    let Some(end) = text.rfind('}') else {
        return serde_json::from_str::<Value>(text).map(|_| text);
    };
    Ok(&text[start..=end])
}

fn validate_llm_candidates(
    envelope: LlmCandidateEnvelope,
    events: &[archon_observability::AgentActivityEvent],
) -> Vec<RetrospectiveCandidate> {
    let event_ids: BTreeSet<&str> = events.iter().map(|event| event.event_id.as_str()).collect();
    envelope
        .candidates
        .into_iter()
        .take(8)
        .filter_map(|candidate| validate_llm_candidate(candidate, &event_ids))
        .collect()
}

fn validate_llm_candidate(
    candidate: LlmCandidate,
    event_ids: &BTreeSet<&str>,
) -> Option<RetrospectiveCandidate> {
    if !(0.55..=1.0).contains(&candidate.confidence) {
        return None;
    }
    let category = normalize_category(&candidate.category, 48)?;
    let domain = normalize_domain(&candidate.domain, 64)?;
    let content = normalize_content(&candidate.content)?;
    if contains_secret_shape(&content) {
        return None;
    }
    let evidence_event_ids: Vec<String> = candidate
        .evidence_event_ids
        .into_iter()
        .filter(|id| event_ids.contains(id.as_str()))
        .take(6)
        .collect();
    if evidence_event_ids.is_empty() {
        return None;
    }
    Some(RetrospectiveCandidate {
        category,
        domain,
        content,
        confidence: candidate.confidence.min(0.95),
        evidence_event_ids,
    })
}

fn normalize_category(input: &str, max_len: usize) -> Option<String> {
    let normalized = input.trim().to_ascii_lowercase().replace('-', "_");
    if normalized.is_empty()
        || normalized.len() > max_len
        || !normalized
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
    {
        return None;
    }
    Some(normalized)
}

fn normalize_domain(input: &str, max_len: usize) -> Option<String> {
    let normalized = input.trim().to_ascii_lowercase().replace('_', "-");
    if normalized.is_empty()
        || normalized.len() > max_len
        || !normalized
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        return None;
    }
    Some(normalized)
}

fn normalize_content(input: &str) -> Option<String> {
    let content = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if content.len() < 24 || content.len() > 280 {
        return None;
    }
    Some(content)
}

fn dedupe_candidates(candidates: Vec<RetrospectiveCandidate>) -> Vec<RetrospectiveCandidate> {
    let mut by_content: BTreeMap<String, RetrospectiveCandidate> = BTreeMap::new();
    for candidate in candidates {
        let key = candidate.content.to_ascii_lowercase();
        match by_content.get(&key) {
            Some(existing) if existing.confidence >= candidate.confidence => {}
            _ => {
                by_content.insert(key, candidate);
            }
        }
    }
    let mut out: Vec<_> = by_content.into_values().collect();
    out.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.content.cmp(&b.content))
    });
    out
}

fn response_text(content: &[Value]) -> String {
    let mut out = String::new();
    for block in content {
        if let Some(text) = block.get("text").and_then(Value::as_str) {
            out.push_str(text);
        } else if let Some(text) = block.as_str() {
            out.push_str(text);
        }
    }
    out
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in input.chars().take(max_chars) {
        out.push(ch);
    }
    if input.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

fn redact_inline_secrets(input: &str) -> String {
    input
        .split_whitespace()
        .map(|part| {
            if looks_like_secret(part) {
                "***REDACTED***"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn contains_secret_shape(input: &str) -> bool {
    input.split_whitespace().any(looks_like_secret)
}

fn looks_like_secret(part: &str) -> bool {
    let trimmed = part.trim_matches(|ch: char| {
        ch == '"' || ch == '\'' || ch == ',' || ch == ';' || ch == ')' || ch == '('
    });
    let lower = trimmed.to_ascii_lowercase();
    trimmed.starts_with("sk-")
        || trimmed.starts_with("sk_ant_")
        || trimmed.starts_with("ghp_")
        || trimmed.starts_with("gho_")
        || trimmed.starts_with("ghu_")
        || trimmed.starts_with("ghs_")
        || trimmed.starts_with("ghr_")
        || lower.starts_with("bearer")
        || lower.contains("authorization:")
        || lower.contains("api_key=")
        || lower.contains("token=")
}

#[cfg(test)]
mod tests;
