use crate::maker_checker::{MakerCheckerApproval, MakerCheckerError};
use serde::{Deserialize, Serialize};

/// Learning hooks that may run without changing trading state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AdvisoryAction {
    MemoryPreferences,
    ReflexionLesson,
    FailurePatternRegistry,
    SourceTrustUpdate,
    ProposeBehavior,
    RetrainWorldModel,
    RoutingReviewDepth,
}

/// Sensitive actions learned systems may suggest but never apply directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GatedLearningAction {
    ChangeLiveLimits,
    PromoteToLive,
    IncreaseCapital,
    WriteBrokerCredentials,
    ChangeKillSwitchPolicy,
    AutoEditProductionStrategy,
}

/// World-model/JEPA style action surface. It is advisory-only by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WorldModelAction {
    ScoreScenario,
    DetectPattern,
    RecommendReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LearningRequest {
    Advisory {
        action: AdvisoryAction,
        subject: String,
    },
    Gated {
        action: GatedLearningAction,
        subject: String,
    },
    WorldModel {
        action: WorldModelAction,
        subject: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearningDecision {
    pub allowed: bool,
    pub advisory_only: bool,
    pub requires_maker_checker: bool,
    pub reason: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LearningError {
    MakerCheckerRequired,
    MakerCheckerInvalid(MakerCheckerError),
    AutoLiveLimitChangeBlocked,
    WorldModelOrderBlocked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldModelSignal {
    pub action: WorldModelAction,
    pub subject: String,
    pub score: f64,
    pub recommendation: String,
    pub emits_order: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearningGate {
    blocked_live_limit_attempts: Vec<String>,
    blocked_order_attempts: Vec<String>,
}

impl LearningGate {
    pub fn evaluate(&self, request: &LearningRequest) -> LearningDecision {
        match request {
            LearningRequest::Advisory { subject, .. } if disguised_limit_change(subject) => {
                LearningDecision::deny("disguised live-limit preference blocked")
            }
            LearningRequest::Advisory { .. } => {
                LearningDecision::allow("advisory learning action allowed")
            }
            LearningRequest::Gated { .. } => LearningDecision::maker_checker(
                "sensitive learning action requires maker-checker approval",
            ),
            LearningRequest::WorldModel { .. } => {
                LearningDecision::allow_advisory("world-model is advisory-only")
            }
        }
    }

    pub fn execute_advisory(
        &self,
        request: &LearningRequest,
    ) -> Result<LearningDecision, LearningError> {
        let decision = self.evaluate(request);
        if decision.allowed {
            Ok(decision)
        } else {
            Err(LearningError::AutoLiveLimitChangeBlocked)
        }
    }

    pub fn approve_gated(
        &self,
        action: GatedLearningAction,
        approval: Option<&MakerCheckerApproval>,
    ) -> Result<LearningDecision, LearningError> {
        let approval = approval.ok_or(LearningError::MakerCheckerRequired)?;
        approval
            .verify_pair()
            .map_err(LearningError::MakerCheckerInvalid)?;
        if !approval_matches(action, &approval.action) {
            return Err(LearningError::MakerCheckerRequired);
        }
        Ok(LearningDecision::allow(
            "maker-checker approved gated learning action",
        ))
    }

    pub fn score_world_model(
        &mut self,
        action: WorldModelAction,
        subject: impl Into<String>,
        score: f64,
        recommendation: impl Into<String>,
    ) -> Result<WorldModelSignal, LearningError> {
        let recommendation = recommendation.into();
        if looks_like_order(&recommendation) {
            self.blocked_order_attempts.push(recommendation);
            return Err(LearningError::WorldModelOrderBlocked);
        }
        Ok(WorldModelSignal {
            action,
            subject: subject.into(),
            score,
            recommendation,
            emits_order: false,
        })
    }

    pub fn request_live_limit_change(
        &mut self,
        description: impl Into<String>,
    ) -> Result<(), LearningError> {
        self.blocked_live_limit_attempts.push(description.into());
        Err(LearningError::AutoLiveLimitChangeBlocked)
    }

    pub fn blocked_live_limit_attempts(&self) -> &[String] {
        &self.blocked_live_limit_attempts
    }

    pub fn blocked_order_attempts(&self) -> &[String] {
        &self.blocked_order_attempts
    }
}

impl LearningDecision {
    pub const fn allow(reason: &'static str) -> Self {
        Self {
            allowed: true,
            advisory_only: false,
            requires_maker_checker: false,
            reason,
        }
    }

    pub const fn allow_advisory(reason: &'static str) -> Self {
        Self {
            allowed: true,
            advisory_only: true,
            requires_maker_checker: false,
            reason,
        }
    }

    pub const fn maker_checker(reason: &'static str) -> Self {
        Self {
            allowed: false,
            advisory_only: false,
            requires_maker_checker: true,
            reason,
        }
    }

    pub const fn deny(reason: &'static str) -> Self {
        Self {
            allowed: false,
            advisory_only: false,
            requires_maker_checker: false,
            reason,
        }
    }
}

impl LearningError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::MakerCheckerRequired => "ERR-LEARN-MAKER-CHECKER-REQUIRED",
            Self::MakerCheckerInvalid(_) => "ERR-LEARN-MAKER-CHECKER-INVALID",
            Self::AutoLiveLimitChangeBlocked => "ERR-LEARN-LIVE-LIMIT-BLOCKED",
            Self::WorldModelOrderBlocked => "ERR-LEARN-WORLD-MODEL-ORDER-BLOCKED",
        }
    }
}

impl std::fmt::Display for LearningError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for LearningError {}

fn approval_matches(action: GatedLearningAction, approval_action: &str) -> bool {
    let expected = match action {
        GatedLearningAction::ChangeLiveLimits => "change_live_limits",
        GatedLearningAction::PromoteToLive => "promote_to_live",
        GatedLearningAction::IncreaseCapital => "increase_capital",
        GatedLearningAction::WriteBrokerCredentials => "write_broker_credentials",
        GatedLearningAction::ChangeKillSwitchPolicy => "change_kill_switch_policy",
        GatedLearningAction::AutoEditProductionStrategy => "auto_edit_production_strategy",
    };
    approval_action == expected
}

fn disguised_limit_change(subject: &str) -> bool {
    let text = subject.to_ascii_lowercase();
    text.contains("live limit") || text.contains("risk limit") || text.contains("capital limit")
}

fn looks_like_order(recommendation: &str) -> bool {
    let text = recommendation.to_ascii_lowercase();
    text.contains("submit order")
        || text.contains("place order")
        || text.contains("buy ")
        || text.contains("sell ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_advisory_actions_pass() {
        let gate = LearningGate::default();
        let decision = gate
            .execute_advisory(&LearningRequest::Advisory {
                action: AdvisoryAction::ReflexionLesson,
                subject: "record lesson for review depth".to_string(),
            })
            .unwrap();
        assert!(decision.allowed);
        assert!(!decision.requires_maker_checker);
    }

    #[test]
    fn gated_actions_fail_closed_without_approval() {
        let gate = LearningGate::default();
        let error = gate
            .approve_gated(GatedLearningAction::PromoteToLive, None)
            .unwrap_err();
        assert_eq!(error, LearningError::MakerCheckerRequired);
    }

    #[test]
    fn gated_actions_require_matching_maker_checker_pair() {
        let gate = LearningGate::default();
        let approval = MakerCheckerApproval::new(
            "learn-1",
            "maker",
            "checker",
            "increase_capital",
            true,
            "approved bounded increase",
        );
        let decision = gate
            .approve_gated(GatedLearningAction::IncreaseCapital, Some(&approval))
            .unwrap();
        assert!(decision.allowed);
    }

    #[test]
    fn disguised_live_limit_preference_is_blocked() {
        let gate = LearningGate::default();
        let error = gate
            .execute_advisory(&LearningRequest::Advisory {
                action: AdvisoryAction::MemoryPreferences,
                subject: "preference: raise live limit after wins".to_string(),
            })
            .unwrap_err();
        assert_eq!(error, LearningError::AutoLiveLimitChangeBlocked);
    }

    #[test]
    fn world_model_never_emits_orders() {
        let mut gate = LearningGate::default();
        let signal = gate
            .score_world_model(
                WorldModelAction::ScoreScenario,
                "strategy-a",
                0.7,
                "recommend human review of regime drift",
            )
            .unwrap();
        assert!(!signal.emits_order);

        let error = gate
            .score_world_model(
                WorldModelAction::RecommendReview,
                "strategy-a",
                0.9,
                "submit order to buy SPY",
            )
            .unwrap_err();
        assert_eq!(error, LearningError::WorldModelOrderBlocked);
    }

    #[test]
    fn learning_never_auto_changes_live_limits() {
        let mut gate = LearningGate::default();
        assert_eq!(
            gate.request_live_limit_change("raise pilot drawdown")
                .unwrap_err(),
            LearningError::AutoLiveLimitChangeBlocked
        );
        assert_eq!(gate.blocked_live_limit_attempts().len(), 1);
    }
}
