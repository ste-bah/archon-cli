
use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::advisor::WorldPrediction;
use crate::integration::WorldAdvisorSurface;
use crate::schema::WorldLabelSet;

pub const ACTIONS_LEDGER: &str = "world-guardrail-actions.jsonl";
pub const DECISIONS_LEDGER: &str = "world-guardrail-decisions.jsonl";
pub const VERIFICATIONS_LEDGER: &str = "world-guardrail-verifications.jsonl";
pub const OUTCOMES_LEDGER: &str = "world-guardrail-outcomes.jsonl";
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldGuardrailMode {
    Off,
    LearnOnly,
    Advisory,
    Guarded,
    Strict,
}

impl WorldGuardrailMode {
    pub fn parses(value: &str) -> bool {
        matches!(
            value,
            "off" | "learn_only" | "advisory" | "guarded" | "strict"
        )
    }

    pub fn requires_verification_for(self, tier: WorldRiskTier) -> bool {
        match self {
            Self::Off | Self::LearnOnly | Self::Advisory => false,
            Self::Guarded => matches!(tier, WorldRiskTier::High | WorldRiskTier::Critical),
            Self::Strict => {
                matches!(
                    tier,
                    WorldRiskTier::Medium | WorldRiskTier::High | WorldRiskTier::Critical
                )
            }
        }
    }

    pub fn can_block(self) -> bool {
        matches!(self, Self::Guarded | Self::Strict)
    }
}

impl std::str::FromStr for WorldGuardrailMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "off" => Ok(Self::Off),
            "learn_only" => Ok(Self::LearnOnly),
            "advisory" => Ok(Self::Advisory),
            "guarded" => Ok(Self::Guarded),
            "strict" => Ok(Self::Strict),
            other => Err(format!("unknown guardrail mode: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardedActionKind {
    UserRequest,
    PlanStep,
    FileEdit,
    ShellCommand,
    TestCommand,
    BuildCommand,
    LintCommand,
    TypecheckCommand,
    ResearchClaim,
    FinalResponse,
    PipelineStep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeTaskClass {
    GeneralAnswer,
    CodingChange,
    ResearchAnswer,
    Refactor,
    Debugging,
    DataMutation,
    ExternalSideEffect,
    PipelineExecution,
    VerificationOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldRiskTier {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailRequiredAction {
    RunTests,
    RunBuild,
    RunLint,
    RunTypecheck,
    RunVerifier,
    ReviewPlanAgainstUserGoal,
    CheckSourceEvidence,
    RecordManualOutcome,
    RequireUserApproval,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailReasonCode {
    PredictedFailureHigh,
    PredictedVerificationNeededHigh,
    PredictedPlanDriftHigh,
    PredictedUserCorrectionHigh,
    PredictedRetryHigh,
    MissingVerification,
    VerificationFailed,
    ManualApprovalRequired,
    AdvisorUnavailable,
    GuardrailOverheadExceeded,
    LowRiskAllowed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationKind {
    Build,
    UnitTests,
    IntegrationTests,
    Lint,
    Typecheck,
    FormatCheck,
    StaticAnalysis,
    SourceEvidenceCheck,
    HumanApproval,
    Custom(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    Passed,
    Failed,
    Skipped,
    NotRun,
    Inconclusive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailFinalStatus {
    CompletedVerified,
    CompletedWithCaveat,
    BlockedMissingVerification,
    BlockedFailedVerification,
    UserApprovedDespiteRisk,
    UserAborted,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldGuardrailPolicyConfig {
    pub enabled: bool,
    pub interactive_mode: WorldGuardrailMode,
    pub pipeline_mode: WorldGuardrailMode,
    pub tool_run_mode: WorldGuardrailMode,
    pub verification_run_mode: WorldGuardrailMode,
    pub high_risk_threshold: f32,
    pub medium_risk_threshold: f32,
    pub critical_risk_threshold: f32,
    pub require_tests_for_coding_high_risk: bool,
    pub require_build_for_coding_high_risk: bool,
    pub require_lint_for_coding_high_risk: bool,
    pub require_typecheck_for_coding_high_risk: bool,
    pub require_plan_review_for_plan_drift: bool,
    pub require_source_check_for_research_high_risk: bool,
    pub require_manual_approval_for_critical: bool,
    pub max_guardrail_overhead_ms: u64,
    pub record_outcomes_without_prediction: bool,
    pub max_guardrail_events_per_session: usize,
}

impl Default for WorldGuardrailPolicyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interactive_mode: WorldGuardrailMode::Advisory,
            pipeline_mode: WorldGuardrailMode::Guarded,
            tool_run_mode: WorldGuardrailMode::LearnOnly,
            verification_run_mode: WorldGuardrailMode::LearnOnly,
            high_risk_threshold: 0.70,
            medium_risk_threshold: 0.45,
            critical_risk_threshold: 0.85,
            require_tests_for_coding_high_risk: true,
            require_build_for_coding_high_risk: true,
            require_lint_for_coding_high_risk: false,
            require_typecheck_for_coding_high_risk: false,
            require_plan_review_for_plan_drift: true,
            require_source_check_for_research_high_risk: true,
            require_manual_approval_for_critical: false,
            max_guardrail_overhead_ms: 40,
            record_outcomes_without_prediction: true,
            max_guardrail_events_per_session: 500,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldGuardedAction {
    pub schema_version: u32,
    pub action_id: String,
    pub session_id: String,
    pub parent_action_id: Option<String>,
    pub surface: WorldAdvisorSurface,
    pub action_kind: GuardedActionKind,
    pub user_goal: String,
    pub action_summary: String,
    pub files_touched: Vec<String>,
    pub commands_planned: Vec<String>,
    pub verification_plan: Vec<VerificationRequirement>,
    pub idempotency_key: String,
    pub created_at: DateTime<Utc>,
}

impl WorldGuardedAction {
    pub fn new(
        session_id: impl Into<String>,
        surface: WorldAdvisorSurface,
        action_kind: GuardedActionKind,
        user_goal: impl Into<String>,
        action_summary: impl Into<String>,
    ) -> Self {
        let session_id = session_id.into();
        let action_id = format!("world-guard-action-{}", uuid::Uuid::new_v4());
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            idempotency_key: format!("world_guardrail:action:{session_id}:{action_id}"),
            action_id,
            session_id,
            parent_action_id: None,
            surface,
            action_kind,
            user_goal: user_goal.into(),
            action_summary: action_summary.into(),
            files_touched: Vec::new(),
            commands_planned: Vec::new(),
            verification_plan: Vec::new(),
            created_at: Utc::now(),
        }
    }
}

impl Default for WorldGuardedAction {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            action_id: String::new(),
            session_id: String::new(),
            parent_action_id: None,
            surface: WorldAdvisorSurface::InteractiveSession,
            action_kind: GuardedActionKind::UserRequest,
            user_goal: String::new(),
            action_summary: String::new(),
            files_touched: Vec::new(),
            commands_planned: Vec::new(),
            verification_plan: Vec::new(),
            idempotency_key: String::new(),
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VerificationRequirement {
    pub requirement_id: String,
    pub kind: VerificationKind,
    pub command_hint: Option<String>,
    pub applies_to: Vec<String>,
    pub required_for_final: bool,
}

impl VerificationRequirement {
    pub fn new(kind: VerificationKind) -> Self {
        Self {
            requirement_id: format!("world-guard-req-{}", uuid::Uuid::new_v4()),
            kind,
            command_hint: None,
            applies_to: Vec::new(),
            required_for_final: true,
        }
    }
}

impl Default for VerificationRequirement {
    fn default() -> Self {
        Self {
            requirement_id: String::new(),
            kind: VerificationKind::Custom(String::new()),
            command_hint: None,
            applies_to: Vec::new(),
            required_for_final: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldGuardrailPredictionContext {
    pub task_class: RuntimeTaskClass,
    pub guardrail_mode: WorldGuardrailMode,
    pub predicted_failure: Option<f32>,
    pub predicted_retry: Option<f32>,
    pub predicted_provider_incident: Option<f32>,
    pub predicted_verification_needed: Option<f32>,
    pub predicted_user_correction: Option<f32>,
    pub predicted_plan_drift: Option<f32>,
    pub predicted_high_cost: Option<f32>,
    pub predicted_slow_run: Option<f32>,
    pub risk_score: f32,
    pub risk_tier: WorldRiskTier,
}

impl WorldGuardrailPredictionContext {
    pub fn from_scores(
        task_class: RuntimeTaskClass,
        guardrail_mode: WorldGuardrailMode,
        scores: GuardrailRiskScores,
        policy: &WorldGuardrailPolicyConfig,
    ) -> Self {
        let risk_score = risk_score(scores);
        let risk_tier = risk_tier(risk_score, policy);
        Self {
            task_class,
            guardrail_mode,
            predicted_failure: scores.predicted_failure,
            predicted_retry: scores.predicted_retry,
            predicted_provider_incident: scores.predicted_provider_incident,
            predicted_verification_needed: scores.predicted_verification_needed,
            predicted_user_correction: scores.predicted_user_correction,
            predicted_plan_drift: scores.predicted_plan_drift,
            predicted_high_cost: scores.predicted_high_cost,
            predicted_slow_run: scores.predicted_slow_run,
            risk_score,
            risk_tier,
        }
    }
}

impl Default for WorldGuardrailPredictionContext {
    fn default() -> Self {
        Self {
            task_class: RuntimeTaskClass::GeneralAnswer,
            guardrail_mode: WorldGuardrailMode::Advisory,
            predicted_failure: None,
            predicted_retry: None,
            predicted_provider_incident: None,
            predicted_verification_needed: None,
            predicted_user_correction: None,
            predicted_plan_drift: None,
            predicted_high_cost: None,
            predicted_slow_run: None,
            risk_score: 0.0,
            risk_tier: WorldRiskTier::Low,
        }
    }
}

