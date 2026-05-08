use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryPromotionSignalSource {
    UserCorrection,
    RepeatedRetrieval,
    UserConfirmation,
    PipelineEvidence,
    GateOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryPromotionTarget {
    MemoryGraphFact,
    RetrievalIndexEntry,
    GovernedLearningEvent,
    AgentRecallQuery,
    BehaviourRule,
    SourceQualityProfile,
    PromptProfileProposal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryPromotionCandidate {
    pub candidate_id: String,
    pub agent_type: String,
    pub signal_source: MemoryPromotionSignalSource,
    pub target: MemoryPromotionTarget,
    pub claim: String,
    pub confidence: f64,
    pub frequency_score: f64,
    pub recency_score: f64,
    pub diversity_score: f64,
    pub evidence_quality: f64,
    pub evidence_ids: Vec<String>,
    pub proposal_required: bool,
    pub created_at: DateTime<Utc>,
}

impl MemoryPromotionCandidate {
    pub fn new(
        agent_type: impl Into<String>,
        signal_source: MemoryPromotionSignalSource,
        target: MemoryPromotionTarget,
        claim: impl Into<String>,
    ) -> Self {
        Self {
            candidate_id: memory_promotion_candidate_id(),
            agent_type: agent_type.into(),
            signal_source,
            target,
            claim: claim.into(),
            confidence: 0.0,
            frequency_score: 0.0,
            recency_score: 0.0,
            diversity_score: 0.0,
            evidence_quality: 0.0,
            evidence_ids: Vec::new(),
            proposal_required: true,
            created_at: Utc::now(),
        }
    }

    pub fn with_scores(
        mut self,
        confidence: f64,
        frequency_score: f64,
        recency_score: f64,
        diversity_score: f64,
        evidence_quality: f64,
    ) -> Self {
        self.confidence = clamp_unit(confidence);
        self.frequency_score = clamp_unit(frequency_score);
        self.recency_score = clamp_unit(recency_score);
        self.diversity_score = clamp_unit(diversity_score);
        self.evidence_quality = clamp_unit(evidence_quality);
        self
    }

    pub fn add_evidence(mut self, evidence_id: impl Into<String>) -> Self {
        let evidence_id = evidence_id.into();
        if !self
            .evidence_ids
            .iter()
            .any(|existing| existing == &evidence_id)
        {
            self.evidence_ids.push(evidence_id);
        }
        self
    }

    pub fn promotion_score(&self) -> f64 {
        (self.confidence * 0.35)
            + (self.frequency_score * 0.20)
            + (self.recency_score * 0.15)
            + (self.diversity_score * 0.10)
            + (self.evidence_quality * 0.20)
    }

    pub fn has_sufficient_evidence(&self) -> bool {
        self.evidence_ids.len() >= 2 && self.evidence_quality >= 0.7 && self.confidence >= 0.7
    }

    pub fn can_be_low_risk_auto_candidate(&self) -> bool {
        self.has_sufficient_evidence()
            && self.promotion_score() >= 0.85
            && matches!(
                self.target,
                MemoryPromotionTarget::RetrievalIndexEntry
                    | MemoryPromotionTarget::AgentRecallQuery
            )
    }
}

pub fn memory_promotion_candidate_id() -> String {
    format!("memory-promotion-{}", uuid::Uuid::new_v4())
}

fn clamp_unit(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promotion_targets_exclude_markdown_memory_files() {
        let targets = serde_json::to_string(&[
            MemoryPromotionTarget::MemoryGraphFact,
            MemoryPromotionTarget::RetrievalIndexEntry,
            MemoryPromotionTarget::GovernedLearningEvent,
            MemoryPromotionTarget::AgentRecallQuery,
            MemoryPromotionTarget::BehaviourRule,
            MemoryPromotionTarget::SourceQualityProfile,
            MemoryPromotionTarget::PromptProfileProposal,
        ])
        .unwrap();

        assert!(!targets.contains("memory_file"));
        assert!(!targets.contains("markdown"));
        assert!(!targets.contains("dreams"));
    }

    #[test]
    fn promotion_candidate_scores_are_clamped_and_weighted() {
        let candidate = MemoryPromotionCandidate::new(
            "researcher",
            MemoryPromotionSignalSource::UserCorrection,
            MemoryPromotionTarget::MemoryGraphFact,
            "Prefer primary-source citations for regulatory claims.",
        )
        .with_scores(2.0, 0.5, 0.5, -1.0, 1.0);

        assert_eq!(candidate.confidence, 1.0);
        assert_eq!(candidate.diversity_score, 0.0);
        assert!((candidate.promotion_score() - 0.725).abs() < f64::EPSILON);
        assert!(candidate.candidate_id.starts_with("memory-promotion-"));
    }

    #[test]
    fn weak_evidence_cannot_auto_promote() {
        let candidate = MemoryPromotionCandidate::new(
            "planner",
            MemoryPromotionSignalSource::RepeatedRetrieval,
            MemoryPromotionTarget::RetrievalIndexEntry,
            "Use the release checklist before patch releases.",
        )
        .with_scores(0.9, 0.9, 0.9, 0.9, 0.4)
        .add_evidence("ev-1");

        assert!(!candidate.has_sufficient_evidence());
        assert!(!candidate.can_be_low_risk_auto_candidate());
        assert!(candidate.proposal_required);
    }

    #[test]
    fn strong_low_risk_recall_candidates_can_be_policy_candidates() {
        let candidate = MemoryPromotionCandidate::new(
            "planner",
            MemoryPromotionSignalSource::UserConfirmation,
            MemoryPromotionTarget::AgentRecallQuery,
            "Recall patch-release checklist before version bumps.",
        )
        .with_scores(0.95, 0.95, 0.95, 0.95, 0.95)
        .add_evidence("ev-1")
        .add_evidence("ev-2");

        assert!(candidate.has_sufficient_evidence());
        assert!(candidate.can_be_low_risk_auto_candidate());
        assert!(candidate.proposal_required);
    }
}
