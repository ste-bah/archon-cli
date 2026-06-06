use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const POSTMORTEM_SLA_MS: u128 = 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionMode {
    Paper,
    LivePilot,
    Live,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TradeSummary {
    pub trade_id: String,
    pub instrument: String,
    pub quantity: f64,
    pub realized_pnl: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskEventSummary {
    pub event_id: String,
    pub control_id: String,
    pub decision: String,
    pub strategy_attributable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecDeviation {
    pub spec_f13_rule: String,
    pub observed: String,
    pub severity: DeviationSeverity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviationSeverity {
    Info,
    Warning,
    Breach,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionPostmortem {
    pub session_id: String,
    pub mode: SessionMode,
    pub strategy_ids: Vec<String>,
    pub trades: Vec<TradeSummary>,
    pub realized_pnl: f64,
    pub risk_events: Vec<RiskEventSummary>,
    pub spec_f13_deviations: Vec<SpecDeviation>,
    pub lessons: Vec<String>,
    pub session_closed_unix_ms: u128,
    pub completed_unix_ms: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailurePattern {
    pub pattern_id: String,
    pub session_id: String,
    pub strategy_ids: Vec<String>,
    pub source: FailurePatternSource,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailurePatternSource {
    RiskEvent,
    SpecDeviation,
    Lesson,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FailurePatternRegistry {
    patterns: Vec<FailurePattern>,
    blocked_live_limit_change_attempts: Vec<BlockedLiveLimitChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockedLiveLimitChange {
    pub session_id: String,
    pub requested_change: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostmortemError {
    MissingField(&'static str),
    SlaMissed,
    LiveLimitChangeBlocked,
    PromotionBlocked,
}

impl SessionPostmortem {
    pub fn validate(&self) -> Result<(), PostmortemError> {
        if self.session_id.trim().is_empty() {
            return Err(PostmortemError::MissingField("session_id"));
        }
        if self.strategy_ids.is_empty() {
            return Err(PostmortemError::MissingField("strategy_ids"));
        }
        if self.lessons.is_empty() {
            return Err(PostmortemError::MissingField("lessons"));
        }
        if self
            .completed_unix_ms
            .saturating_sub(self.session_closed_unix_ms)
            > POSTMORTEM_SLA_MS
        {
            return Err(PostmortemError::SlaMissed);
        }
        Ok(())
    }

    pub fn ready_for_promotion(&self) -> bool {
        self.validate().is_ok()
    }

    pub fn derived_patterns(&self) -> Vec<FailurePattern> {
        let mut patterns = Vec::new();
        for event in &self.risk_events {
            patterns.push(self.pattern(
                FailurePatternSource::RiskEvent,
                &event.event_id,
                format!("risk {} -> {}", event.control_id, event.decision),
            ));
        }
        for deviation in &self.spec_f13_deviations {
            patterns.push(self.pattern(
                FailurePatternSource::SpecDeviation,
                &deviation.spec_f13_rule,
                format!(
                    "SPEC-F13 {} observed {}",
                    deviation.spec_f13_rule, deviation.observed
                ),
            ));
        }
        for (index, lesson) in self.lessons.iter().enumerate() {
            patterns.push(self.pattern(
                FailurePatternSource::Lesson,
                &index.to_string(),
                lesson.clone(),
            ));
        }
        patterns
    }

    fn pattern(
        &self,
        source: FailurePatternSource,
        key: &str,
        description: String,
    ) -> FailurePattern {
        FailurePattern {
            pattern_id: stable_pattern_id(&self.session_id, source, key),
            session_id: self.session_id.clone(),
            strategy_ids: self.strategy_ids.clone(),
            source,
            description,
        }
    }
}

impl FailurePatternRegistry {
    pub fn patterns(&self) -> &[FailurePattern] {
        &self.patterns
    }

    pub fn blocked_live_limit_change_attempts(&self) -> &[BlockedLiveLimitChange] {
        &self.blocked_live_limit_change_attempts
    }

    pub fn ingest_postmortem(
        &mut self,
        postmortem: &SessionPostmortem,
    ) -> Result<(), PostmortemError> {
        postmortem.validate()?;
        let mut known: BTreeSet<String> = self
            .patterns
            .iter()
            .map(|item| item.pattern_id.clone())
            .collect();
        for pattern in postmortem.derived_patterns() {
            if known.insert(pattern.pattern_id.clone()) {
                self.patterns.push(pattern);
            }
        }
        Ok(())
    }

    pub fn request_live_limit_change(
        &mut self,
        session_id: &str,
        requested_change: &str,
    ) -> Result<(), PostmortemError> {
        self.blocked_live_limit_change_attempts
            .push(BlockedLiveLimitChange {
                session_id: session_id.to_string(),
                requested_change: requested_change.to_string(),
                reason: "postmortem_registry_is_advisory_only".to_string(),
            });
        Err(PostmortemError::LiveLimitChangeBlocked)
    }
}

pub fn require_postmortem_for_promotion(
    postmortem: Option<&SessionPostmortem>,
) -> Result<(), PostmortemError> {
    match postmortem {
        Some(report) => report.validate(),
        None => Err(PostmortemError::PromotionBlocked),
    }
}

impl std::fmt::Display for PostmortemError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingField(field) => write!(formatter, "missing postmortem field: {field}"),
            Self::SlaMissed => formatter.write_str("postmortem SLA missed"),
            Self::LiveLimitChangeBlocked => {
                formatter.write_str("postmortem cannot change live limits")
            }
            Self::PromotionBlocked => formatter.write_str("promotion requires a postmortem"),
        }
    }
}

impl std::error::Error for PostmortemError {}

fn stable_pattern_id(session_id: &str, source: FailurePatternSource, key: &str) -> String {
    let source = match source {
        FailurePatternSource::RiskEvent => "risk",
        FailurePatternSource::SpecDeviation => "spec-f13",
        FailurePatternSource::Lesson => "lesson",
    };
    blake3::hash(format!("{session_id}:{source}:{key}").as_bytes())
        .to_hex()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> SessionPostmortem {
        SessionPostmortem {
            session_id: "paper-session-1".to_string(),
            mode: SessionMode::Paper,
            strategy_ids: vec!["strategy-a".to_string()],
            trades: vec![TradeSummary {
                trade_id: "trade-1".to_string(),
                instrument: "SPY".to_string(),
                quantity: 1.0,
                realized_pnl: 12.5,
            }],
            realized_pnl: 12.5,
            risk_events: vec![RiskEventSummary {
                event_id: "risk-1".to_string(),
                control_id: "REQ-RISK-004".to_string(),
                decision: "blocked".to_string(),
                strategy_attributable: true,
            }],
            spec_f13_deviations: vec![SpecDeviation {
                spec_f13_rule: "exit-discipline".to_string(),
                observed: "late manual exit".to_string(),
                severity: DeviationSeverity::Warning,
            }],
            lessons: vec!["tighten paper runbook".to_string()],
            session_closed_unix_ms: 1_000,
            completed_unix_ms: 1_000 + POSTMORTEM_SLA_MS,
        }
    }

    #[test]
    fn t_paper_04_promotion_requires_valid_postmortem() {
        assert_eq!(
            require_postmortem_for_promotion(None),
            Err(PostmortemError::PromotionBlocked)
        );
        assert!(require_postmortem_for_promotion(Some(&report())).is_ok());
    }

    #[test]
    fn t_post_01_structured_report_updates_failure_patterns() {
        let mut registry = FailurePatternRegistry::default();
        registry
            .ingest_postmortem(&report())
            .expect("valid postmortem ingests");
        registry
            .ingest_postmortem(&report())
            .expect("duplicate ingest is idempotent");

        assert_eq!(registry.patterns().len(), 3);
        assert!(
            registry
                .patterns()
                .iter()
                .any(|item| item.source == FailurePatternSource::RiskEvent)
        );
        assert!(
            registry
                .patterns()
                .iter()
                .any(|item| item.source == FailurePatternSource::SpecDeviation)
        );
        assert!(
            registry
                .patterns()
                .iter()
                .any(|item| item.source == FailurePatternSource::Lesson)
        );
    }

    #[test]
    fn a_post_01_registry_never_changes_live_limits() {
        let mut registry = FailurePatternRegistry::default();
        let error = registry
            .request_live_limit_change("paper-session-1", "daily_loss_limit=10%")
            .expect_err("postmortem registry is advisory only");

        assert_eq!(error, PostmortemError::LiveLimitChangeBlocked);
        assert_eq!(registry.blocked_live_limit_change_attempts().len(), 1);
    }
}
