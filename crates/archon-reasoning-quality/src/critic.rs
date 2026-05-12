use serde::{Deserialize, Serialize};

use crate::types::{ReasoningEventKind, VerificationState};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CriticFinding {
    pub claim_id: String,
    pub event_kind: ReasoningEventKind,
    pub verification_state: VerificationState,
    pub confidence: f32,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CriticResponse {
    #[serde(default)]
    pub findings: Vec<CriticFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CriticCoverage {
    Full,
    Partial,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CriticBudgetDecision {
    Allowed,
    BudgetExhaustedSession,
    BudgetExhaustedDaily,
    BudgetExhaustedWeekly,
}

#[derive(Debug, Clone, Copy)]
pub struct CriticBudgetLimits {
    pub per_session_token_cap: u64,
    pub daily_usd_cap: f64,
    pub weekly_usd_cap: f64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CriticBudgetUsage {
    pub session_tokens: u64,
    pub daily_usd: f64,
    pub weekly_usd: f64,
}

pub fn parse_critic_response(text: &str) -> Result<Vec<CriticFinding>, String> {
    let json = strip_markdown_fence(text);
    let response: CriticResponse =
        serde_json::from_str(json).map_err(|e| format!("critic JSON parse failed: {e}"))?;
    for finding in &response.findings {
        if finding.claim_id.trim().is_empty() {
            return Err("critic finding missing claim_id".to_string());
        }
        if !(0.0..=1.0).contains(&finding.confidence) {
            return Err(format!(
                "critic finding confidence out of range for {}",
                finding.claim_id
            ));
        }
    }
    Ok(response.findings)
}

pub fn check_critic_budget(
    limits: CriticBudgetLimits,
    usage: CriticBudgetUsage,
    estimated_tokens: u64,
    estimated_usd: f64,
) -> CriticBudgetDecision {
    if usage.session_tokens.saturating_add(estimated_tokens) > limits.per_session_token_cap {
        return CriticBudgetDecision::BudgetExhaustedSession;
    }
    if usage.daily_usd + estimated_usd > limits.daily_usd_cap {
        return CriticBudgetDecision::BudgetExhaustedDaily;
    }
    if usage.weekly_usd + estimated_usd > limits.weekly_usd_cap {
        return CriticBudgetDecision::BudgetExhaustedWeekly;
    }
    CriticBudgetDecision::Allowed
}

pub fn coverage_for(processed: usize, total: usize) -> CriticCoverage {
    match (processed, total) {
        (_, 0) => CriticCoverage::None,
        (0, _) => CriticCoverage::None,
        (p, t) if p >= t => CriticCoverage::Full,
        _ => CriticCoverage::Partial,
    }
}

fn strip_markdown_fence(text: &str) -> &str {
    let trimmed = text.trim();
    if let Some(stripped) = trimmed.strip_prefix("```json") {
        return stripped.trim_end_matches("```").trim();
    }
    if let Some(stripped) = trimmed.strip_prefix("```") {
        return stripped.trim_end_matches("```").trim();
    }
    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_strict_critic_json() {
        let findings = parse_critic_response(
            r#"{"findings":[{"claim_id":"rqclm_1","event_kind":"verification_needed","verification_state":"needs_human_review","confidence":0.7,"rationale":"needs source"}]}"#,
        )
        .unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].event_kind,
            ReasoningEventKind::VerificationNeeded
        );
    }

    #[test]
    fn rejects_out_of_range_confidence() {
        let err = parse_critic_response(
            r#"{"findings":[{"claim_id":"rqclm_1","event_kind":"verification_needed","verification_state":"needs_human_review","confidence":1.7,"rationale":"bad"}]}"#,
        )
        .unwrap_err();
        assert!(err.contains("out of range"));
    }

    #[test]
    fn budget_exhaustion_is_ordered_by_scope() {
        let decision = check_critic_budget(
            CriticBudgetLimits {
                per_session_token_cap: 10,
                daily_usd_cap: 10.0,
                weekly_usd_cap: 10.0,
            },
            CriticBudgetUsage {
                session_tokens: 9,
                daily_usd: 0.0,
                weekly_usd: 0.0,
            },
            2,
            0.01,
        );
        assert_eq!(decision, CriticBudgetDecision::BudgetExhaustedSession);
    }
}
