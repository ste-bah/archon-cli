use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use archon_reasoning_quality::store::ReasoningQualityStore;
use archon_reasoning_quality::{ReasoningEventKind, ReasoningQualityEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightRecommendation {
    Clear,
    Caution,
    Block,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskFlag {
    pub kind: String,
    pub severity: f32,
    pub event_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskReport {
    pub store_available: bool,
    pub recommendation: PreflightRecommendation,
    pub flags: Vec<RiskFlag>,
    pub checked_at: DateTime<Utc>,
}

impl RiskReport {
    pub fn unavailable() -> Self {
        Self {
            store_available: false,
            recommendation: PreflightRecommendation::Caution,
            flags: Vec::new(),
            checked_at: Utc::now(),
        }
    }

    pub fn has_risks(&self) -> bool {
        !self.flags.is_empty()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ReasoningQualityPreflight {
    pub max_events: usize,
}

impl Default for ReasoningQualityPreflight {
    fn default() -> Self {
        Self { max_events: 100 }
    }
}

impl ReasoningQualityPreflight {
    pub fn run_for_store(
        &self,
        store: &ReasoningQualityStore,
        session_id: Option<&str>,
    ) -> RiskReport {
        let events = match session_id {
            Some(id) => store.events_for_session(id),
            None => store.recent_events(self.max_events),
        };
        match events {
            Ok(events) => self.run_for_events(&events),
            Err(_) => RiskReport::unavailable(),
        }
    }

    pub fn run_for_events(&self, events: &[ReasoningQualityEvent]) -> RiskReport {
        let mut flags: Vec<_> = events.iter().filter_map(flag_for_event).collect();
        flags.sort_by(|a, b| {
            b.severity
                .partial_cmp(&a.severity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        flags.truncate(self.max_events);
        let recommendation = recommendation_for(&flags);
        RiskReport {
            store_available: true,
            recommendation,
            flags,
            checked_at: Utc::now(),
        }
    }
}

fn flag_for_event(event: &ReasoningQualityEvent) -> Option<RiskFlag> {
    let (kind, reason) = match event.event_kind {
        ReasoningEventKind::UnsupportedClaim => (
            "unsupported_claim_tendency",
            "recent unsupported claim without adequate evidence",
        ),
        ReasoningEventKind::ClaimBeforeSourceRead => (
            "source_verification_gap",
            "recent claim was made before source verification",
        ),
        ReasoningEventKind::CompletionClaimWithoutEvidence => (
            "false_completion_history",
            "completion claim lacked command or source evidence",
        ),
        ReasoningEventKind::TestStatusClaimWithoutCommand => (
            "test_build_claim_risk",
            "test or build status claim lacked command evidence",
        ),
        ReasoningEventKind::ClaimCorrectedByUser => (
            "recent_user_correction",
            "recent user correction should lower confidence",
        ),
        ReasoningEventKind::ClaimContradictedBySource => (
            "source_contradiction",
            "recent claim was contradicted by source evidence",
        ),
        _ => return None,
    };
    Some(RiskFlag {
        kind: kind.into(),
        severity: event.severity_effective.max(event.severity_base),
        event_id: event.event_id.clone(),
        reason: reason.into(),
    })
}

fn recommendation_for(flags: &[RiskFlag]) -> PreflightRecommendation {
    let max = flags
        .iter()
        .map(|flag| flag.severity)
        .fold(0.0_f32, f32::max);
    if max >= 0.85 {
        PreflightRecommendation::Block
    } else if max > 0.0 {
        PreflightRecommendation::Caution
    } else {
        PreflightRecommendation::Clear
    }
}
