use archon_policy::CognitivePolicy;
use chrono::{DateTime, Utc};
use cozo::DbInstance;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::governed_apply_store::{
    query_reflection_evidence, rollback_ref, store_apply_result, store_canary, store_proposal,
};
use crate::schema::ensure_cognitive_schema;
use crate::{
    CognitiveError, PolicyGate, ProposalCheck, ReflectionRecord, RiskLevel, SituationKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BehaviourManifestKind {
    ToolPreference,
    MemoryRecallStrategy,
    AnswerFormatting,
    ActionRanking,
    VerificationThreshold,
    PipelineOverride,
    ScoringWeight,
    PolicyMutation,
    PromptMutation,
    NetworkConfig,
    BlockingGate,
    Unknown,
}

impl BehaviourManifestKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ToolPreference => "tool_preference",
            Self::MemoryRecallStrategy => "memory_recall_strategy",
            Self::AnswerFormatting => "answer_formatting",
            Self::ActionRanking => "action_ranking",
            Self::VerificationThreshold => "verification_threshold",
            Self::PipelineOverride => "pipeline_override",
            Self::ScoringWeight => "scoring_weight",
            Self::PolicyMutation => "policy_mutation",
            Self::PromptMutation => "prompt_mutation",
            Self::NetworkConfig => "network_config",
            Self::BlockingGate => "blocking_gate",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    pub proposal_id: String,
    pub reflection_ids: Vec<String>,
    pub manifest_kind: BehaviourManifestKind,
    pub risk_level: RiskLevel,
    pub evidence_count: u64,
    pub lesson_tag: String,
    pub domain: String,
    pub diff_summary: String,
    pub rollback_plan: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyResult {
    AutoApplied {
        proposal_id: String,
        canary_outcome_ref: String,
        rollback_ref: String,
        applied_at: DateTime<Utc>,
    },
    PendingReview {
        proposal_id: String,
        reason: String,
    },
    Denied {
        proposal_id: String,
        reason: String,
    },
    RolledBack {
        proposal_id: String,
        reason: String,
        rolled_back_at: DateTime<Utc>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanaryOutcome {
    pub canary_id: String,
    pub proposal_id: String,
    pub passed: bool,
    pub details: String,
    pub snapshot_ref: String,
    pub created_at: DateTime<Utc>,
}

pub struct GovernedAutonomousApply<'a> {
    db: &'a DbInstance,
    gate: PolicyGate,
}

impl<'a> GovernedAutonomousApply<'a> {
    pub fn new(
        db: &'a DbInstance,
        policy: Option<CognitivePolicy>,
    ) -> Result<Self, CognitiveError> {
        ensure_cognitive_schema(db)?;
        Ok(Self {
            db,
            gate: PolicyGate::new(policy),
        })
    }

    pub fn propose(&self, reflection: &ReflectionRecord) -> Result<Proposal, CognitiveError> {
        let domain = domain_for(reflection.situation_kind).to_string();
        let evidence = query_reflection_evidence(self.db, &reflection.lesson, &domain)?;
        let manifest_kind = classify_manifest_kind(&reflection.lesson);
        let proposal = Proposal {
            proposal_id: Uuid::new_v4().to_string(),
            reflection_ids: evidence.reflection_ids,
            manifest_kind,
            risk_level: classify_proposal_risk(manifest_kind),
            evidence_count: evidence.count,
            lesson_tag: reflection.lesson.clone(),
            domain,
            diff_summary: diff_summary(&reflection.lesson, manifest_kind),
            rollback_plan: Some("remove behaviour manifest entry".into()),
            created_at: Utc::now(),
        };
        store_proposal(self.db, &proposal)?;
        Ok(proposal)
    }

    pub fn apply(&self, proposal: &Proposal) -> Result<ApplyResult, CognitiveError> {
        store_proposal(self.db, proposal)?;
        if let Some(reason) = must_deny_reason(proposal) {
            return self.store_result(ApplyResult::Denied {
                proposal_id: proposal.proposal_id.clone(),
                reason,
            });
        }
        if let Some(denial) = self.gate.deny_proposal(&proposal_check(proposal)) {
            return self.store_result(result_for_denial(
                proposal,
                denial.policy_rule,
                denial.reason,
            ));
        }
        let canary = self.canary_apply(proposal)?;
        store_canary(self.db, &canary)?;
        if !canary.passed {
            return self.store_result(ApplyResult::Denied {
                proposal_id: proposal.proposal_id.clone(),
                reason: canary.details,
            });
        }
        self.store_result(ApplyResult::AutoApplied {
            proposal_id: proposal.proposal_id.clone(),
            canary_outcome_ref: canary.canary_id,
            rollback_ref: rollback_ref(proposal),
            applied_at: Utc::now(),
        })
    }

    pub fn rollback(&self, proposal_id: &str, reason: &str) -> Result<ApplyResult, CognitiveError> {
        self.store_result(ApplyResult::RolledBack {
            proposal_id: proposal_id.to_string(),
            reason: reason.to_string(),
            rolled_back_at: Utc::now(),
        })
    }

    pub fn canary_apply(&self, proposal: &Proposal) -> Result<CanaryOutcome, CognitiveError> {
        let passed = proposal.rollback_plan.is_some() && proposal.risk_level <= RiskLevel::Low;
        Ok(CanaryOutcome {
            canary_id: Uuid::new_v4().to_string(),
            proposal_id: proposal.proposal_id.clone(),
            passed,
            details: if passed {
                "shadow manifest validation passed".into()
            } else {
                "shadow manifest validation failed".into()
            },
            snapshot_ref: format!("shadow-{}", proposal.proposal_id),
            created_at: Utc::now(),
        })
    }

    fn store_result(&self, result: ApplyResult) -> Result<ApplyResult, CognitiveError> {
        store_apply_result(self.db, &result)?;
        Ok(result)
    }
}

fn classify_manifest_kind(lesson: &str) -> BehaviourManifestKind {
    let text = lesson.to_ascii_lowercase();
    if text.contains("prompt") {
        BehaviourManifestKind::PromptMutation
    } else if text.contains("policy") {
        BehaviourManifestKind::PolicyMutation
    } else if text.contains("network") {
        BehaviourManifestKind::NetworkConfig
    } else if text.contains("gate") {
        BehaviourManifestKind::BlockingGate
    } else if text.contains("memory") {
        BehaviourManifestKind::MemoryRecallStrategy
    } else if text.contains("tool") {
        BehaviourManifestKind::ToolPreference
    } else if text.contains("format") {
        BehaviourManifestKind::AnswerFormatting
    } else if text.contains("verification") {
        BehaviourManifestKind::VerificationThreshold
    } else {
        BehaviourManifestKind::Unknown
    }
}

fn classify_proposal_risk(kind: BehaviourManifestKind) -> RiskLevel {
    match kind {
        BehaviourManifestKind::ToolPreference
        | BehaviourManifestKind::MemoryRecallStrategy
        | BehaviourManifestKind::AnswerFormatting => RiskLevel::Low,
        BehaviourManifestKind::ActionRanking
        | BehaviourManifestKind::VerificationThreshold
        | BehaviourManifestKind::Unknown => RiskLevel::Medium,
        BehaviourManifestKind::PipelineOverride | BehaviourManifestKind::ScoringWeight => {
            RiskLevel::High
        }
        BehaviourManifestKind::PolicyMutation
        | BehaviourManifestKind::PromptMutation
        | BehaviourManifestKind::NetworkConfig
        | BehaviourManifestKind::BlockingGate => RiskLevel::Critical,
    }
}

fn proposal_check(proposal: &Proposal) -> ProposalCheck {
    ProposalCheck {
        proposal_id: proposal.proposal_id.clone(),
        touched_paths: vec![proposal.diff_summary.clone()],
        risk_level: proposal.risk_level,
        evidence_count: proposal.evidence_count as usize,
        recent_incidents: 0,
        rollback_available: proposal.rollback_plan.is_some(),
    }
}

fn result_for_denial(proposal: &Proposal, rule: String, reason: String) -> ApplyResult {
    if rule == "rollback_unavailable" || rule.contains("forbidden") {
        ApplyResult::Denied {
            proposal_id: proposal.proposal_id.clone(),
            reason,
        }
    } else {
        ApplyResult::PendingReview {
            proposal_id: proposal.proposal_id.clone(),
            reason,
        }
    }
}

fn must_deny_reason(proposal: &Proposal) -> Option<String> {
    match proposal.manifest_kind {
        BehaviourManifestKind::PromptMutation => {
            Some("prompt mutation requires human review".into())
        }
        BehaviourManifestKind::PolicyMutation => {
            Some("policy mutation requires human review".into())
        }
        BehaviourManifestKind::NetworkConfig => {
            Some("network configuration requires human review".into())
        }
        BehaviourManifestKind::BlockingGate => {
            Some("blocking gate changes require human review".into())
        }
        _ => None,
    }
}

fn diff_summary(lesson: &str, kind: BehaviourManifestKind) -> String {
    format!("manifest={}; lesson={}", kind.as_str(), truncate(lesson))
}

fn domain_for(kind: SituationKind) -> &'static str {
    match kind {
        SituationKind::CiDebug => "ci",
        SituationKind::CodeChange => "coding",
        SituationKind::GitMutation => "git",
        SituationKind::PipelineControl => "pipeline",
        SituationKind::Research => "research",
        SituationKind::WorldModelTask => "world_model",
        SituationKind::HighRisk => "safety",
        SituationKind::Greeting | SituationKind::SimpleQuestion | SituationKind::Ambiguous => {
            "general"
        }
    }
}

fn truncate(value: &str) -> String {
    value.chars().take(160).collect()
}
