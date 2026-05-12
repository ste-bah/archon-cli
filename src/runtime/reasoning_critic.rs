//! Optional LLM critic for reasoning-quality events.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use archon_llm::provider::{DataFlowClassification, LlmProvider, LlmRequest};
use archon_reasoning_quality::{
    CriticBudgetDecision, CriticBudgetLimits, CriticBudgetUsage, ReasoningEventKind,
    ReasoningQualityEvent, VerificationState, base_severity, check_critic_budget, event_id_for,
    parse_critic_response,
};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub(crate) struct CriticRuntime {
    pub(crate) enabled: bool,
    pub(crate) config: archon_core::config::ReasoningQualityCriticConfig,
    pub(crate) policy: archon_policy::models::EffectivePolicy,
    pub(crate) provider: Arc<dyn LlmProvider>,
    pub(crate) model_fallback: String,
    pub(crate) root: PathBuf,
    pub(crate) learning_db: Option<Arc<cozo::DbInstance>>,
    pub(crate) world_root: Option<PathBuf>,
    pub(crate) feed_world_model: bool,
    pub(crate) update_self_trust: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub(crate) struct CriticUsageSummary {
    pub(crate) session_tokens: u64,
    pub(crate) daily_tokens: u64,
    pub(crate) weekly_tokens: u64,
    pub(crate) daily_usd: f64,
    pub(crate) weekly_usd: f64,
}

pub(crate) fn spawn_critic_if_enabled(
    runtime: Option<CriticRuntime>,
    events: Vec<ReasoningQualityEvent>,
) {
    let Some(runtime) = runtime else { return };
    if !runtime.enabled || events.is_empty() || !runtime.config.allow_llm {
        return;
    }
    archon_observability::spawn_named("reasoning-quality-critic", async move {
        if let Err(error) = run_critic(runtime, events).await {
            tracing::warn!(error = %error, "reasoning-quality critic failed");
        }
    });
}

pub(crate) fn read_usage_summary(root: &Path, session_id: Option<&str>) -> CriticUsageSummary {
    let mut summary = CriticUsageSummary::default();
    let now = chrono::Utc::now();
    let dir = root.join("critic-cost");
    let Ok(entries) = fs::read_dir(dir) else {
        return summary;
    };
    for entry in entries.flatten() {
        let Ok(text) = fs::read_to_string(entry.path()) else {
            continue;
        };
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let Ok(row) = serde_json::from_str::<CriticCostRow>(line) else {
                continue;
            };
            let age_days = (now - row.created_at).num_days();
            if session_id.is_some_and(|wanted| wanted == row.session_id) {
                summary.session_tokens = summary
                    .session_tokens
                    .saturating_add(row.input_tokens.saturating_add(row.output_tokens));
            }
            if age_days <= 0 {
                summary.daily_tokens = summary
                    .daily_tokens
                    .saturating_add(row.input_tokens.saturating_add(row.output_tokens));
                summary.daily_usd += row.estimated_usd;
            }
            if age_days <= 7 {
                summary.weekly_tokens = summary
                    .weekly_tokens
                    .saturating_add(row.input_tokens.saturating_add(row.output_tokens));
                summary.weekly_usd += row.estimated_usd;
            }
        }
    }
    summary
}

async fn run_critic(runtime: CriticRuntime, events: Vec<ReasoningQualityEvent>) -> Result<()> {
    let provider_name = runtime.provider.name().to_string();
    let data_flow = runtime.provider.data_flow_classification();
    let data_flow_str = data_flow_class(data_flow);
    let requested_third_party = runtime.config.provider != "default"
        && !runtime.config.provider.is_empty()
        && runtime.config.provider != provider_name;
    let decision = runtime
        .policy
        .reasoning_quality_llm_critic_decision(data_flow_str, requested_third_party);
    if !decision.allowed {
        append_unavailable(&runtime, &events, "policy_denied")?;
        return Ok(());
    }
    if requested_third_party {
        append_unavailable(&runtime, &events, "separate_critic_provider_not_wired")?;
        return Ok(());
    }

    let selected = select_events(events);
    let prompt = critic_prompt(&selected);
    let estimated_tokens =
        estimate_tokens(&prompt).saturating_add(runtime.config.max_tokens as u64);
    let usage = read_usage_summary(&runtime.root, Some(&selected[0].session_id));
    let budget = check_critic_budget(
        CriticBudgetLimits {
            per_session_token_cap: runtime.config.budget.per_session_token_cap,
            daily_usd_cap: runtime.config.budget.daily_usd_cap,
            weekly_usd_cap: runtime.config.budget.weekly_usd_cap,
        },
        CriticBudgetUsage {
            session_tokens: usage.session_tokens,
            daily_usd: usage.daily_usd,
            weekly_usd: usage.weekly_usd,
        },
        estimated_tokens,
        estimate_cost_usd(estimated_tokens, 0),
    );
    if budget != CriticBudgetDecision::Allowed {
        append_unavailable(&runtime, &selected, budget_reason(budget))?;
        return Ok(());
    }

    let model = critic_model(&runtime);
    let response = runtime
        .provider
        .complete(LlmRequest {
            model: model.clone(),
            max_tokens: runtime.config.max_tokens,
            system: vec![serde_json::json!({
                "type": "text",
                "text": "You are Archon's reasoning-quality critic. Return strict JSON only. Do not include prose."
            })],
            messages: vec![serde_json::json!({
                "role": "user",
                "content": [{"type": "text", "text": prompt}]
            })],
            request_origin: Some("reasoning_quality_critic".into()),
            extra: serde_json::json!({ "temperature": runtime.config.temperature }),
            ..LlmRequest::default()
        })
        .await?;
    let text = response_text(&response.content);
    let findings = match parse_critic_response(&text) {
        Ok(findings) => findings,
        Err(error) => {
            append_unavailable(&runtime, &selected, &format!("parse_failed:{error}"))?;
            return Ok(());
        }
    };
    let derived = derive_events(&selected, findings);
    append_cost_row(
        &runtime.root,
        CriticCostRow {
            created_at: chrono::Utc::now(),
            session_id: selected[0].session_id.clone(),
            provider: provider_name,
            model,
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
            estimated_usd: estimate_cost_usd(
                response.usage.input_tokens,
                response.usage.output_tokens,
            ),
            coverage: if derived.is_empty() {
                "none"
            } else {
                "partial"
            }
            .into(),
        },
    )?;
    if derived.is_empty() {
        return Ok(());
    }
    let store = archon_reasoning_quality::store::ReasoningQualityStore::open(&runtime.root)?;
    store.append_events(&derived)?;
    crate::runtime::reasoning_quality::bridge_reasoning_events(
        &derived,
        runtime.learning_db.as_deref(),
        &runtime.root,
        runtime.world_root.as_deref(),
        runtime.feed_world_model,
        runtime.update_self_trust,
    );
    Ok(())
}

fn select_events(mut events: Vec<ReasoningQualityEvent>) -> Vec<ReasoningQualityEvent> {
    events.sort_by(|left, right| right.severity_effective.total_cmp(&left.severity_effective));
    events.truncate(12);
    events
}

fn derive_events(
    source: &[ReasoningQualityEvent],
    findings: Vec<archon_reasoning_quality::CriticFinding>,
) -> Vec<ReasoningQualityEvent> {
    findings
        .into_iter()
        .filter_map(|finding| {
            let base = source
                .iter()
                .find(|event| event.claim_id == finding.claim_id)?;
            let mut event = base.clone();
            event.event_kind = finding.event_kind;
            event.verification_state = finding.verification_state;
            event.severity_base = base_severity(finding.event_kind);
            event.severity_effective = (event.severity_base * finding.confidence).clamp(0.0, 1.0);
            event.source_system = "reasoning_quality_critic".into();
            event.event_id = event_id_for(
                &event.session_id,
                event.turn_number,
                &event.claim_id,
                &format!("critic:{:?}", event.event_kind),
            );
            Some(event)
        })
        .collect()
}

fn append_unavailable(
    runtime: &CriticRuntime,
    source: &[ReasoningQualityEvent],
    reason: &str,
) -> Result<()> {
    let Some(base) = source.first() else {
        return Ok(());
    };
    let mut event = base.clone();
    event.event_kind = ReasoningEventKind::CriticUnavailable;
    event.verification_state = VerificationState::NeedsHumanReview;
    event.severity_base = base_severity(ReasoningEventKind::CriticUnavailable);
    event.severity_effective = event.severity_base;
    event.source_system = "reasoning_quality_critic".into();
    event.canonical_text = format!("critic unavailable: {reason}");
    event.redacted_excerpt = Some(event.canonical_text.clone());
    event.event_id = event_id_for(
        &event.session_id,
        event.turn_number,
        &event.claim_id,
        &format!("critic_unavailable:{reason}"),
    );
    let store = archon_reasoning_quality::store::ReasoningQualityStore::open(&runtime.root)?;
    store.append_events(std::slice::from_ref(&event))?;
    crate::runtime::reasoning_quality::bridge_reasoning_events(
        &[event],
        runtime.learning_db.as_deref(),
        &runtime.root,
        runtime.world_root.as_deref(),
        runtime.feed_world_model,
        runtime.update_self_trust,
    );
    Ok(())
}

fn critic_prompt(events: &[ReasoningQualityEvent]) -> String {
    let claims = events
        .iter()
        .map(|event| {
            serde_json::json!({
                "claim_id": event.claim_id,
                "event_kind": event.event_kind,
                "subject": event.subject,
                "verification_state": event.verification_state,
                "confidence_signal": event.confidence_signal,
                "claim": event.canonical_text,
                "evidence_count": event.evidence_refs.len(),
            })
        })
        .collect::<Vec<_>>();
    format!(
        "Review these visible assistant claims for reasoning-quality issues. \
Return JSON: {{\"findings\":[{{\"claim_id\":\"...\",\"event_kind\":\"verification_needed|unsupported_claim|claim_contradicted_by_source|source_verified_claim\",\"verification_state\":\"needs_human_review|unverified|contradicted|verified_after_claim\",\"confidence\":0.0,\"rationale\":\"short\"}}]}}.\n\n{}",
        serde_json::to_string(&claims).unwrap_or_else(|_| "[]".into())
    )
}

fn response_text(content: &[serde_json::Value]) -> String {
    content
        .iter()
        .filter_map(|block| block.get("text").and_then(|value| value.as_str()))
        .collect::<Vec<_>>()
        .join("")
}

fn critic_model(runtime: &CriticRuntime) -> String {
    if runtime.config.model.trim().is_empty() {
        runtime.model_fallback.clone()
    } else {
        runtime.config.model.clone()
    }
}

fn estimate_tokens(text: &str) -> u64 {
    (text.len() as u64 / 4).max(1)
}

fn estimate_cost_usd(input_tokens: u64, output_tokens: u64) -> f64 {
    (input_tokens as f64 * 0.000003) + (output_tokens as f64 * 0.000015)
}

fn data_flow_class(class: DataFlowClassification) -> &'static str {
    match class {
        DataFlowClassification::Local => "local",
        DataFlowClassification::UserOperated => "user_operated",
        DataFlowClassification::Cloud => "cloud",
    }
}

fn budget_reason(decision: CriticBudgetDecision) -> &'static str {
    match decision {
        CriticBudgetDecision::Allowed => "allowed",
        CriticBudgetDecision::BudgetExhaustedSession => "budget_exhausted_session",
        CriticBudgetDecision::BudgetExhaustedDaily => "budget_exhausted_daily",
        CriticBudgetDecision::BudgetExhaustedWeekly => "budget_exhausted_weekly",
    }
}

fn append_cost_row(root: &Path, row: CriticCostRow) -> Result<()> {
    let path = root
        .join("critic-cost")
        .join(format!("{}.jsonl", row.created_at.format("%Y-%m-%d")));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, &row)?;
    file.write_all(b"\n")?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CriticCostRow {
    created_at: chrono::DateTime<chrono::Utc>,
    session_id: String,
    provider: String,
    model: String,
    input_tokens: u64,
    output_tokens: u64,
    estimated_usd: f64,
    coverage: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_llm::provider::{LlmError, LlmResponse, ModelInfo, ProviderFeature};

    struct FakeProvider;

    #[async_trait::async_trait]
    impl LlmProvider for FakeProvider {
        fn name(&self) -> &str {
            "fake-local"
        }

        fn models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }

        async fn stream(
            &self,
            _request: LlmRequest,
        ) -> Result<tokio::sync::mpsc::Receiver<archon_llm::streaming::StreamEvent>, LlmError>
        {
            Err(LlmError::Unsupported("stream".into()))
        }

        async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
            Ok(LlmResponse {
                content: vec![serde_json::json!({
                    "text": r#"{"findings":[{"claim_id":"rqclm_fake","event_kind":"verification_needed","verification_state":"needs_human_review","confidence":0.8,"rationale":"needs source"}]}"#
                })],
                usage: archon_llm::types::Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    ..Default::default()
                },
                stop_reason: "end_turn".into(),
            })
        }

        fn supports_feature(&self, _feature: ProviderFeature) -> bool {
            false
        }

        fn data_flow_classification(&self) -> DataFlowClassification {
            DataFlowClassification::Local
        }
    }

    #[tokio::test]
    async fn critic_writes_derived_event_and_cost_row() {
        let temp = tempfile::tempdir().unwrap();
        let mut policy = archon_policy::models::EffectivePolicy::default();
        policy.reasoning_quality.allow_llm_critic = true;
        let mut config = archon_core::config::ReasoningQualityCriticConfig::default();
        config.allow_llm = true;
        let event = ReasoningQualityEvent {
            event_id: "rqevt_fake".into(),
            claim_id: "rqclm_fake".into(),
            session_id: "s1".into(),
            canonical_text: "the code does the thing".into(),
            severity_effective: 0.7,
            ..ReasoningQualityEvent::default()
        };

        run_critic(
            CriticRuntime {
                enabled: true,
                config,
                policy,
                provider: Arc::new(FakeProvider),
                model_fallback: "fake-model".into(),
                root: temp.path().to_path_buf(),
                learning_db: None,
                world_root: None,
                feed_world_model: false,
                update_self_trust: false,
            },
            vec![event],
        )
        .await
        .unwrap();

        let store =
            archon_reasoning_quality::store::ReasoningQualityStore::open(temp.path()).unwrap();
        let events = store.events_for_session("s1").unwrap();
        assert!(
            events
                .iter()
                .any(|event| event.event_kind == ReasoningEventKind::VerificationNeeded)
        );
        assert!(read_usage_summary(temp.path(), Some("s1")).session_tokens > 0);
    }
}
