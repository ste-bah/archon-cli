//! Runtime world-model guardrail contracts.
//!
//! These types are intentionally independent from the CLI/session loop. They
//! model the durable "predict -> guard -> verify -> learn" records used by
//! interactive sessions, tool runs, and pipeline steps.

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct GuardrailRiskScores {
    pub predicted_failure: Option<f32>,
    pub predicted_retry: Option<f32>,
    pub predicted_provider_incident: Option<f32>,
    pub predicted_verification_needed: Option<f32>,
    pub predicted_user_correction: Option<f32>,
    pub predicted_plan_drift: Option<f32>,
    pub predicted_high_cost: Option<f32>,
    pub predicted_slow_run: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldGuardrailDecision {
    pub schema_version: u32,
    pub decision_id: String,
    pub prediction_id: Option<String>,
    pub action_id: String,
    pub surface: WorldAdvisorSurface,
    pub mode: WorldGuardrailMode,
    pub risk_tier: WorldRiskTier,
    pub required_actions: Vec<GuardrailRequiredAction>,
    pub allowed_to_continue: bool,
    pub allowed_to_finalize: bool,
    pub reason_codes: Vec<GuardrailReasonCode>,
    pub prediction_context: Option<WorldGuardrailPredictionContext>,
    pub idempotency_key: String,
    pub created_at: DateTime<Utc>,
}

impl WorldGuardrailDecision {
    pub fn unavailable(action: &WorldGuardedAction) -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            decision_id: format!("world-guard-decision-{}", uuid::Uuid::new_v4()),
            prediction_id: None,
            action_id: action.action_id.clone(),
            surface: action.surface,
            mode: WorldGuardrailMode::Advisory,
            risk_tier: WorldRiskTier::Low,
            required_actions: Vec::new(),
            allowed_to_continue: true,
            allowed_to_finalize: true,
            reason_codes: vec![GuardrailReasonCode::AdvisorUnavailable],
            prediction_context: None,
            idempotency_key: format!("world_guardrail:decision:{}", action.action_id),
            created_at: Utc::now(),
        }
    }
}

impl Default for WorldGuardrailDecision {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            decision_id: String::new(),
            prediction_id: None,
            action_id: String::new(),
            surface: WorldAdvisorSurface::InteractiveSession,
            mode: WorldGuardrailMode::Advisory,
            risk_tier: WorldRiskTier::Low,
            required_actions: Vec::new(),
            allowed_to_continue: true,
            allowed_to_finalize: true,
            reason_codes: Vec::new(),
            prediction_context: None,
            idempotency_key: String::new(),
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VerificationOutcome {
    pub schema_version: u32,
    pub requirement_id: String,
    pub action_id: String,
    pub kind: VerificationKind,
    pub status: VerificationStatus,
    pub command: Option<String>,
    pub exit_code: Option<i32>,
    pub summary: String,
    pub evidence_refs: Vec<String>,
    pub idempotency_key: String,
    pub created_at: DateTime<Utc>,
}

impl VerificationOutcome {
    pub fn passed(
        action_id: impl Into<String>,
        kind: VerificationKind,
        summary: impl Into<String>,
    ) -> Self {
        let action_id = action_id.into();
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            requirement_id: String::new(),
            idempotency_key: format!("world_guardrail:verification:{action_id}:passed"),
            action_id,
            kind,
            status: VerificationStatus::Passed,
            command: None,
            exit_code: Some(0),
            summary: summary.into(),
            evidence_refs: Vec::new(),
            created_at: Utc::now(),
        }
    }
}

impl Default for VerificationOutcome {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            requirement_id: String::new(),
            action_id: String::new(),
            kind: VerificationKind::Custom(String::new()),
            status: VerificationStatus::NotRun,
            command: None,
            exit_code: None,
            summary: String::new(),
            evidence_refs: Vec::new(),
            idempotency_key: String::new(),
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldGuardrailOutcome {
    pub schema_version: u32,
    pub outcome_id: String,
    pub action_id: String,
    pub prediction_id: Option<String>,
    pub decision_id: Option<String>,
    pub surface: WorldAdvisorSurface,
    pub task_class: RuntimeTaskClass,
    pub final_status: GuardrailFinalStatus,
    pub verification_outcomes: Vec<VerificationOutcome>,
    pub user_correction_observed: bool,
    pub plan_drift_observed: bool,
    pub provider_incident_observed: bool,
    pub high_cost_observed: bool,
    pub slow_run_observed: bool,
    pub retry_count: u32,
    pub files_changed: Vec<String>,
    pub evidence_refs: Vec<String>,
    pub actual_summary: String,
    pub latent_surprise: Option<f32>,
    pub idempotency_key: String,
    pub created_at: DateTime<Utc>,
}

impl WorldGuardrailOutcome {
    pub fn from_decision(
        decision: &WorldGuardrailDecision,
        task_class: RuntimeTaskClass,
        final_status: GuardrailFinalStatus,
        actual_summary: impl Into<String>,
    ) -> Self {
        let outcome_id = format!("world-guard-outcome-{}", uuid::Uuid::new_v4());
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            outcome_id: outcome_id.clone(),
            action_id: decision.action_id.clone(),
            prediction_id: decision.prediction_id.clone(),
            decision_id: Some(decision.decision_id.clone()),
            surface: decision.surface,
            task_class,
            final_status,
            verification_outcomes: Vec::new(),
            user_correction_observed: false,
            plan_drift_observed: false,
            provider_incident_observed: false,
            high_cost_observed: false,
            slow_run_observed: false,
            retry_count: 0,
            files_changed: Vec::new(),
            evidence_refs: Vec::new(),
            actual_summary: actual_summary.into(),
            latent_surprise: None,
            idempotency_key: format!("world_guardrail:outcome:{outcome_id}"),
            created_at: Utc::now(),
        }
    }
}

impl Default for WorldGuardrailOutcome {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            outcome_id: String::new(),
            action_id: String::new(),
            prediction_id: None,
            decision_id: None,
            surface: WorldAdvisorSurface::InteractiveSession,
            task_class: RuntimeTaskClass::GeneralAnswer,
            final_status: GuardrailFinalStatus::CompletedWithCaveat,
            verification_outcomes: Vec::new(),
            user_correction_observed: false,
            plan_drift_observed: false,
            provider_incident_observed: false,
            high_cost_observed: false,
            slow_run_observed: false,
            retry_count: 0,
            files_changed: Vec::new(),
            evidence_refs: Vec::new(),
            actual_summary: String::new(),
            latent_surprise: None,
            idempotency_key: String::new(),
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuardrailStatusCounts {
    pub actions: usize,
    pub decisions: usize,
    pub verifications: usize,
    pub outcomes: usize,
    pub blocked_outcomes: usize,
    pub failed_verifications: usize,
    pub missing_verifications: usize,
    pub outcomes_without_prediction: usize,
    pub advisor_unavailable_decisions: usize,
}

pub fn risk_score(scores: GuardrailRiskScores) -> f32 {
    [
        scores.predicted_failure.unwrap_or(0.0),
        scores.predicted_verification_needed.unwrap_or(0.0),
        scores.predicted_user_correction.unwrap_or(0.0),
        scores.predicted_plan_drift.unwrap_or(0.0),
        scores.predicted_retry.unwrap_or(0.0) * 0.75,
        scores.predicted_provider_incident.unwrap_or(0.0) * 0.50,
        scores.predicted_high_cost.unwrap_or(0.0) * 0.50,
        scores.predicted_slow_run.unwrap_or(0.0) * 0.40,
    ]
    .into_iter()
    .fold(0.0_f32, f32::max)
}

pub fn risk_tier(score: f32, policy: &WorldGuardrailPolicyConfig) -> WorldRiskTier {
    if score >= policy.critical_risk_threshold {
        WorldRiskTier::Critical
    } else if score >= policy.high_risk_threshold {
        WorldRiskTier::High
    } else if score >= policy.medium_risk_threshold {
        WorldRiskTier::Medium
    } else {
        WorldRiskTier::Low
    }
}

pub fn classify_task(summary: &str, surface: WorldAdvisorSurface) -> RuntimeTaskClass {
    if matches!(surface, WorldAdvisorSurface::VerificationRun) {
        return RuntimeTaskClass::VerificationOnly;
    }
    let lower = summary.to_ascii_lowercase();
    let has_any = |terms: &[&str]| terms.iter().any(|term| lower.contains(term));
    if has_any(&["refactor", "rename module", "clean up"]) {
        RuntimeTaskClass::Refactor
    } else if has_any(&["debug", "bug", "fix", "failing", "failed test"]) {
        RuntimeTaskClass::Debugging
    } else if has_any(&[
        "code",
        "implement",
        "build",
        "python",
        "rust",
        "typescript",
        "app",
        "coding",
    ]) {
        RuntimeTaskClass::CodingChange
    } else if has_any(&["research", "source", "citation", "verify claim", "look up"]) {
        RuntimeTaskClass::ResearchAnswer
    } else if matches!(
        surface,
        WorldAdvisorSurface::Pipeline | WorldAdvisorSurface::PipelineStep
    ) {
        RuntimeTaskClass::PipelineExecution
    } else if has_any(&["delete", "deploy", "publish", "send", "external"]) {
        RuntimeTaskClass::ExternalSideEffect
    } else {
        RuntimeTaskClass::GeneralAnswer
    }
}

pub fn default_scores_for_task(task_class: RuntimeTaskClass) -> GuardrailRiskScores {
    match task_class {
        RuntimeTaskClass::CodingChange
        | RuntimeTaskClass::Debugging
        | RuntimeTaskClass::Refactor => GuardrailRiskScores {
            predicted_verification_needed: Some(0.72),
            predicted_failure: Some(0.50),
            predicted_plan_drift: Some(0.35),
            ..GuardrailRiskScores::default()
        },
        RuntimeTaskClass::ResearchAnswer => GuardrailRiskScores {
            predicted_verification_needed: Some(0.70),
            predicted_user_correction: Some(0.45),
            ..GuardrailRiskScores::default()
        },
        RuntimeTaskClass::PipelineExecution => GuardrailRiskScores {
            predicted_verification_needed: Some(0.55),
            predicted_failure: Some(0.45),
            ..GuardrailRiskScores::default()
        },
        _ => GuardrailRiskScores::default(),
    }
}

pub fn mode_for_surface(
    policy: &WorldGuardrailPolicyConfig,
    surface: WorldAdvisorSurface,
) -> WorldGuardrailMode {
    match surface {
        WorldAdvisorSurface::Pipeline | WorldAdvisorSurface::PipelineStep => policy.pipeline_mode,
        WorldAdvisorSurface::ToolRun => policy.tool_run_mode,
        WorldAdvisorSurface::VerificationRun => policy.verification_run_mode,
        _ => policy.interactive_mode,
    }
}

pub fn decide_guardrail(
    action: &WorldGuardedAction,
    prediction: Option<&WorldPrediction>,
    context: WorldGuardrailPredictionContext,
    policy: &WorldGuardrailPolicyConfig,
) -> WorldGuardrailDecision {
    if !policy.enabled || matches!(context.guardrail_mode, WorldGuardrailMode::Off) {
        return WorldGuardrailDecision {
            schema_version: CURRENT_SCHEMA_VERSION,
            decision_id: format!("world-guard-decision-{}", uuid::Uuid::new_v4()),
            prediction_id: prediction.map(|p| p.prediction_id.clone()),
            action_id: action.action_id.clone(),
            surface: action.surface,
            mode: WorldGuardrailMode::Off,
            risk_tier: context.risk_tier,
            required_actions: Vec::new(),
            allowed_to_continue: true,
            allowed_to_finalize: true,
            reason_codes: vec![GuardrailReasonCode::LowRiskAllowed],
            prediction_context: Some(context),
            idempotency_key: format!("world_guardrail:decision:{}", action.action_id),
            created_at: Utc::now(),
        };
    }

    let mut required_actions = Vec::new();
    let mut reason_codes = Vec::new();
    if context
        .guardrail_mode
        .requires_verification_for(context.risk_tier)
    {
        push_requirements(&mut required_actions, action, &context, policy);
        push_reason_codes(&mut reason_codes, &context, policy);
    }

    if required_actions.is_empty() {
        reason_codes.push(GuardrailReasonCode::LowRiskAllowed);
    }

    let allowed_to_finalize = required_actions.is_empty() || !context.guardrail_mode.can_block();

    WorldGuardrailDecision {
        schema_version: CURRENT_SCHEMA_VERSION,
        decision_id: format!("world-guard-decision-{}", uuid::Uuid::new_v4()),
        prediction_id: prediction.map(|p| p.prediction_id.clone()),
        action_id: action.action_id.clone(),
        surface: action.surface,
        mode: context.guardrail_mode,
        risk_tier: context.risk_tier,
        required_actions,
        allowed_to_continue: true,
        allowed_to_finalize,
        reason_codes,
        prediction_context: Some(context),
        idempotency_key: format!("world_guardrail:decision:{}", action.action_id),
        created_at: Utc::now(),
    }
}

pub fn finalization_allowed(
    decision: &WorldGuardrailDecision,
    verification_outcomes: &[VerificationOutcome],
) -> bool {
    if decision.allowed_to_finalize || !decision.mode.can_block() {
        return true;
    }
    if decision.required_actions.is_empty() {
        return true;
    }
    let mut latest_by_kind = BTreeMap::<String, &VerificationOutcome>::new();
    for outcome in verification_outcomes {
        let key = verification_kind_key(&outcome.kind);
        let replace = latest_by_kind
            .get(&key)
            .is_none_or(|existing| existing.created_at <= outcome.created_at);
        if replace {
            latest_by_kind.insert(key, outcome);
        }
    }
    decision
        .required_actions
        .iter()
        .all(|required| required_action_passed(*required, &latest_by_kind))
}

fn required_action_passed(
    required: GuardrailRequiredAction,
    latest_by_kind: &BTreeMap<String, &VerificationOutcome>,
) -> bool {
    required_action_kind_keys(required).into_iter().any(|key| {
        latest_by_kind.get(*key).is_some_and(|outcome| {
            matches!(
                outcome.status,
                VerificationStatus::Passed | VerificationStatus::Skipped
            )
        })
    })
}

fn required_action_kind_keys(required: GuardrailRequiredAction) -> &'static [&'static str] {
    match required {
        GuardrailRequiredAction::RunTests => &["unit_tests", "integration_tests"],
        GuardrailRequiredAction::RunBuild => &["build"],
        GuardrailRequiredAction::RunLint => &["lint"],
        GuardrailRequiredAction::RunTypecheck => &["typecheck"],
        GuardrailRequiredAction::RunVerifier => &[
            "custom:verifier",
            "unit_tests",
            "integration_tests",
            "build",
            "lint",
            "typecheck",
            "format_check",
            "static_analysis",
            "source_evidence_check",
            "human_approval",
        ],
        GuardrailRequiredAction::ReviewPlanAgainstUserGoal => &["custom:plan_review"],
        GuardrailRequiredAction::CheckSourceEvidence => &["source_evidence_check"],
        GuardrailRequiredAction::RecordManualOutcome => &["custom:manual_outcome"],
        GuardrailRequiredAction::RequireUserApproval => &["human_approval"],
    }
}

fn verification_kind_key(kind: &VerificationKind) -> String {
    match kind {
        VerificationKind::Build => "build".into(),
        VerificationKind::UnitTests => "unit_tests".into(),
        VerificationKind::IntegrationTests => "integration_tests".into(),
        VerificationKind::Lint => "lint".into(),
        VerificationKind::Typecheck => "typecheck".into(),
        VerificationKind::FormatCheck => "format_check".into(),
        VerificationKind::StaticAnalysis => "static_analysis".into(),
        VerificationKind::SourceEvidenceCheck => "source_evidence_check".into(),
        VerificationKind::HumanApproval => "human_approval".into(),
        VerificationKind::Custom(value) => format!("custom:{value}"),
    }
}

pub fn guardrail_budget_allows_prediction_latency(
    prediction_elapsed_ms: u64,
    prediction_budget_ms: u64,
    guardrail_overhead_ms: u64,
    max_guardrail_overhead_ms: u64,
) -> bool {
    prediction_elapsed_ms <= prediction_budget_ms
        && guardrail_overhead_ms <= max_guardrail_overhead_ms
}

pub fn enforce_guardrail_overhead_budget(
    mut decision: WorldGuardrailDecision,
    guardrail_overhead_ms: u64,
    max_guardrail_overhead_ms: u64,
) -> WorldGuardrailDecision {
    if guardrail_overhead_ms <= max_guardrail_overhead_ms {
        return decision;
    }

    decision.required_actions.clear();
    decision.allowed_to_continue = true;
    decision.allowed_to_finalize = true;
    decision
        .reason_codes
        .retain(|reason| *reason != GuardrailReasonCode::LowRiskAllowed);
    if !decision
        .reason_codes
        .contains(&GuardrailReasonCode::GuardrailOverheadExceeded)
    {
        decision
            .reason_codes
            .push(GuardrailReasonCode::GuardrailOverheadExceeded);
    }
    decision
        .reason_codes
        .sort_by_key(|reason| format!("{reason:?}"));
    decision.reason_codes.dedup();
    decision
}

pub fn labels_from_guardrail_outcome(outcome: &WorldGuardrailOutcome) -> WorldLabelSet {
    let mut labels = WorldLabelSet::default();
    let verification_failed = outcome
        .verification_outcomes
        .iter()
        .any(|verification| verification.status == VerificationStatus::Failed);
    let verification_missing = matches!(
        outcome.final_status,
        GuardrailFinalStatus::BlockedMissingVerification
    );

    if verification_failed
        || matches!(
            outcome.final_status,
            GuardrailFinalStatus::BlockedFailedVerification | GuardrailFinalStatus::Failed
        )
    {
        labels.failure = true;
        labels.success = Some(false);
    }
    if verification_failed || verification_missing {
        labels.verification_needed = true;
    }
    if matches!(
        outcome.final_status,
        GuardrailFinalStatus::CompletedVerified | GuardrailFinalStatus::CompletedWithCaveat
    ) {
        labels.success = Some(true);
    }
    if outcome.user_correction_observed {
        labels.user_correction = true;
    }
    if outcome.plan_drift_observed {
        labels.plan_drift = true;
    }
    if outcome.retry_count > 0 {
        labels.retry = true;
    }
    if outcome.provider_incident_observed {
        labels.provider_incident = true;
    }
    labels.high_cost = outcome.high_cost_observed;
    labels.slow_run = outcome.slow_run_observed;
    labels
}

pub fn append_guarded_action(root: &Path, record: &WorldGuardedAction) -> anyhow::Result<PathBuf> {
    append_jsonl_idempotent(root, ACTIONS_LEDGER, record)
}

pub fn append_guardrail_decision(
    root: &Path,
    record: &WorldGuardrailDecision,
) -> anyhow::Result<PathBuf> {
    append_jsonl_idempotent(root, DECISIONS_LEDGER, record)
}

pub fn append_verification_outcome(
    root: &Path,
    record: &VerificationOutcome,
) -> anyhow::Result<PathBuf> {
    append_jsonl_idempotent(root, VERIFICATIONS_LEDGER, record)
}

pub fn append_guardrail_outcome(
    root: &Path,
    record: &WorldGuardrailOutcome,
) -> anyhow::Result<PathBuf> {
    append_jsonl_idempotent(root, OUTCOMES_LEDGER, record)
}

pub fn load_guarded_actions(root: &Path) -> anyhow::Result<Vec<WorldGuardedAction>> {
    load_jsonl(root, ACTIONS_LEDGER)
}

pub fn load_guardrail_decisions(root: &Path) -> anyhow::Result<Vec<WorldGuardrailDecision>> {
    load_jsonl(root, DECISIONS_LEDGER)
}

pub fn load_verification_outcomes(root: &Path) -> anyhow::Result<Vec<VerificationOutcome>> {
    load_jsonl(root, VERIFICATIONS_LEDGER)
}

pub fn load_guardrail_outcomes(root: &Path) -> anyhow::Result<Vec<WorldGuardrailOutcome>> {
    load_jsonl(root, OUTCOMES_LEDGER)
}

pub fn guardrail_status_counts(root: &Path) -> GuardrailStatusCounts {
    let actions = load_guarded_actions(root).unwrap_or_default();
    let decisions = load_guardrail_decisions(root).unwrap_or_default();
    let verifications = load_verification_outcomes(root).unwrap_or_default();
    let outcomes = load_guardrail_outcomes(root).unwrap_or_default();
    GuardrailStatusCounts {
        actions: actions.len(),
        decisions: decisions.len(),
        verifications: verifications.len(),
        outcomes: outcomes.len(),
        blocked_outcomes: outcomes
            .iter()
            .filter(|outcome| {
                matches!(
                    outcome.final_status,
                    GuardrailFinalStatus::BlockedMissingVerification
                        | GuardrailFinalStatus::BlockedFailedVerification
                )
            })
            .count(),
        failed_verifications: verifications
            .iter()
            .filter(|verification| verification.status == VerificationStatus::Failed)
            .count(),
        missing_verifications: outcomes
            .iter()
            .filter(|outcome| {
                matches!(
                    outcome.final_status,
                    GuardrailFinalStatus::BlockedMissingVerification
                )
            })
            .count(),
        outcomes_without_prediction: outcomes
            .iter()
            .filter(|outcome| outcome.prediction_id.is_none())
            .count(),
        advisor_unavailable_decisions: decisions
            .iter()
            .filter(|decision| {
                decision.reason_codes.iter().any(|reason| {
                    matches!(
                        *reason,
                        GuardrailReasonCode::AdvisorUnavailable
                            | GuardrailReasonCode::GuardrailOverheadExceeded
                    )
                })
            })
            .count(),
    }
}

fn append_jsonl<T: Serialize>(root: &Path, filename: &str, record: &T) -> anyhow::Result<PathBuf> {
    let path = root.join("ledgers").join(filename);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut line = serde_json::to_vec(record)?;
    line.push(b'\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?
        .write_all(&line)?;
    Ok(path)
}

fn append_jsonl_idempotent<T: Serialize>(
    root: &Path,
    filename: &str,
    record: &T,
) -> anyhow::Result<PathBuf> {
    let path = root.join("ledgers").join(filename);
    let value = serde_json::to_value(record)?;
    let idempotency_key = value
        .get("idempotency_key")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty());

    if let Some(idempotency_key) = idempotency_key
        && path.exists()
    {
        let file =
            std::fs::File::open(&path).with_context(|| format!("open {}", path.display()))?;
        for line in std::io::BufReader::new(file).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let Ok(existing) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            if existing
                .get("idempotency_key")
                .and_then(|value| value.as_str())
                == Some(idempotency_key)
            {
                return Ok(path);
            }
        }
    }

    append_jsonl(root, filename, record)
}

fn load_jsonl<T>(root: &Path, filename: &str) -> anyhow::Result<Vec<T>>
where
    for<'de> T: Deserialize<'de>,
{
    let path = root.join("ledgers").join(filename);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = std::fs::File::open(&path).with_context(|| format!("open {}", path.display()))?;
    let mut rows = Vec::new();
    for line in std::io::BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        rows.push(serde_json::from_str(&line)?);
    }
    Ok(rows)
}

fn push_requirements(
    required_actions: &mut Vec<GuardrailRequiredAction>,
    action: &WorldGuardedAction,
    context: &WorldGuardrailPredictionContext,
    policy: &WorldGuardrailPolicyConfig,
) {
    if matches!(
        context.task_class,
        RuntimeTaskClass::CodingChange | RuntimeTaskClass::Debugging | RuntimeTaskClass::Refactor
    ) {
        if policy.require_tests_for_coding_high_risk {
            required_actions.push(GuardrailRequiredAction::RunTests);
        }
        if policy.require_build_for_coding_high_risk {
            required_actions.push(GuardrailRequiredAction::RunBuild);
        }
        if policy.require_lint_for_coding_high_risk {
            required_actions.push(GuardrailRequiredAction::RunLint);
        }
        if policy.require_typecheck_for_coding_high_risk {
            required_actions.push(GuardrailRequiredAction::RunTypecheck);
        }
    }
    if matches!(context.task_class, RuntimeTaskClass::ResearchAnswer)
        && policy.require_source_check_for_research_high_risk
    {
        required_actions.push(GuardrailRequiredAction::CheckSourceEvidence);
    }
    if matches!(context.task_class, RuntimeTaskClass::PipelineExecution) {
        required_actions.push(GuardrailRequiredAction::RunVerifier);
    }
    let plan_drift = context.predicted_plan_drift.unwrap_or(0.0);
    if plan_drift >= policy.high_risk_threshold && policy.require_plan_review_for_plan_drift {
        required_actions.push(GuardrailRequiredAction::ReviewPlanAgainstUserGoal);
    }
    if matches!(context.risk_tier, WorldRiskTier::Critical)
        && policy.require_manual_approval_for_critical
    {
        required_actions.push(GuardrailRequiredAction::RequireUserApproval);
    }
    if required_actions.is_empty()
        && matches!(
            action.surface,
            WorldAdvisorSurface::Pipeline | WorldAdvisorSurface::PipelineStep
        )
    {
        required_actions.push(GuardrailRequiredAction::RunVerifier);
    }
    required_actions.sort_by_key(|action| format!("{action:?}"));
    required_actions.dedup();
}

fn push_reason_codes(
    reason_codes: &mut Vec<GuardrailReasonCode>,
    context: &WorldGuardrailPredictionContext,
    policy: &WorldGuardrailPolicyConfig,
) {
    if context.predicted_failure.unwrap_or(0.0) >= policy.high_risk_threshold {
        reason_codes.push(GuardrailReasonCode::PredictedFailureHigh);
    }
    if context.predicted_verification_needed.unwrap_or(0.0) >= policy.high_risk_threshold {
        reason_codes.push(GuardrailReasonCode::PredictedVerificationNeededHigh);
    }
    if context.predicted_plan_drift.unwrap_or(0.0) >= policy.high_risk_threshold {
        reason_codes.push(GuardrailReasonCode::PredictedPlanDriftHigh);
    }
    if context.predicted_user_correction.unwrap_or(0.0) >= policy.high_risk_threshold {
        reason_codes.push(GuardrailReasonCode::PredictedUserCorrectionHigh);
    }
    if context.predicted_retry.unwrap_or(0.0) >= policy.high_risk_threshold {
        reason_codes.push(GuardrailReasonCode::PredictedRetryHigh);
    }
    if reason_codes.is_empty()
        && matches!(
            context.risk_tier,
            WorldRiskTier::High | WorldRiskTier::Critical
        )
    {
        reason_codes.push(GuardrailReasonCode::PredictedVerificationNeededHigh);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn risk_score_treats_none_as_zero_and_applies_weights() {
        let score = risk_score(GuardrailRiskScores {
            predicted_retry: Some(1.0),
            predicted_high_cost: Some(1.0),
            ..GuardrailRiskScores::default()
        });

        assert_eq!(score, 0.75);
    }

    #[test]
    fn risk_tier_uses_inclusive_thresholds() {
        let policy = WorldGuardrailPolicyConfig::default();

        assert_eq!(risk_tier(0.85, &policy), WorldRiskTier::Critical);
        assert_eq!(risk_tier(0.70, &policy), WorldRiskTier::High);
        assert_eq!(risk_tier(0.45, &policy), WorldRiskTier::Medium);
        assert_eq!(risk_tier(0.44, &policy), WorldRiskTier::Low);
    }

    #[test]
    fn pipeline_step_classification_preserves_coding_and_research_tasks() {
        assert_eq!(
            classify_task(
                "coding pipeline: implement authentication",
                WorldAdvisorSurface::PipelineStep,
            ),
            RuntimeTaskClass::CodingChange,
        );
        assert_eq!(
            classify_task(
                "research pipeline: verify citation sources",
                WorldAdvisorSurface::PipelineStep,
            ),
            RuntimeTaskClass::ResearchAnswer,
        );
        assert_eq!(
            classify_task("pipeline batch", WorldAdvisorSurface::PipelineStep),
            RuntimeTaskClass::PipelineExecution,
        );
    }

    #[test]
    fn guarded_high_risk_coding_requires_verification() {
        let policy = WorldGuardrailPolicyConfig::default();
        let action = WorldGuardedAction::new(
            "s1",
            WorldAdvisorSurface::CodingTask,
            GuardedActionKind::UserRequest,
            "build app",
            "build a Python app",
        );
        let context = WorldGuardrailPredictionContext::from_scores(
            RuntimeTaskClass::CodingChange,
            WorldGuardrailMode::Guarded,
            GuardrailRiskScores {
                predicted_verification_needed: Some(0.90),
                ..GuardrailRiskScores::default()
            },
            &policy,
        );

        let decision = decide_guardrail(&action, None, context, &policy);

        assert!(!decision.allowed_to_finalize);
        assert!(
            decision
                .required_actions
                .contains(&GuardrailRequiredAction::RunTests)
        );
        assert!(
            decision
                .required_actions
                .contains(&GuardrailRequiredAction::RunBuild)
        );
        assert!(
            decision
                .reason_codes
                .contains(&GuardrailReasonCode::PredictedVerificationNeededHigh)
        );
    }

    #[test]
    fn advisory_high_risk_warns_but_does_not_block() {
        let policy = WorldGuardrailPolicyConfig::default();
        let action = WorldGuardedAction::new(
            "s1",
            WorldAdvisorSurface::InteractiveSession,
            GuardedActionKind::UserRequest,
            "code",
            "implement feature",
        );
        let context = WorldGuardrailPredictionContext::from_scores(
            RuntimeTaskClass::CodingChange,
            WorldGuardrailMode::Advisory,
            GuardrailRiskScores {
                predicted_verification_needed: Some(0.90),
                ..GuardrailRiskScores::default()
            },
            &policy,
        );

        let decision = decide_guardrail(&action, None, context, &policy);

        assert!(decision.allowed_to_finalize);
        assert!(decision.required_actions.is_empty());
    }

    #[test]
    fn finalization_requires_passed_verification_when_blocking() {
        let mut decision = WorldGuardrailDecision::default();
        decision.mode = WorldGuardrailMode::Guarded;
        decision.allowed_to_finalize = false;
        decision.required_actions = vec![GuardrailRequiredAction::RunTests];

        assert!(!finalization_allowed(&decision, &[]));
        assert!(!finalization_allowed(
            &decision,
            &[VerificationOutcome {
                kind: VerificationKind::UnitTests,
                status: VerificationStatus::Failed,
                ..VerificationOutcome::default()
            }]
        ));
        assert!(finalization_allowed(
            &decision,
            &[VerificationOutcome {
                kind: VerificationKind::UnitTests,
                status: VerificationStatus::Passed,
                ..VerificationOutcome::default()
            }]
        ));
    }

    #[test]
    fn finalization_allows_explicitly_skipped_verification() {
        let mut decision = WorldGuardrailDecision::default();
        decision.mode = WorldGuardrailMode::Guarded;
        decision.allowed_to_finalize = false;
        decision.required_actions = vec![GuardrailRequiredAction::RunTests];

        assert!(finalization_allowed(
            &decision,
            &[VerificationOutcome {
                kind: VerificationKind::UnitTests,
                status: VerificationStatus::Skipped,
                summary: "manual override: operator accepted the risk".into(),
                ..VerificationOutcome::default()
            }]
        ));
    }

    #[test]
    fn finalization_accepts_explicit_verifier_record() {
        let mut decision = WorldGuardrailDecision::default();
        decision.mode = WorldGuardrailMode::Guarded;
        decision.allowed_to_finalize = false;
        decision.required_actions = vec![GuardrailRequiredAction::RunVerifier];

        assert!(finalization_allowed(
            &decision,
            &[VerificationOutcome {
                kind: VerificationKind::Custom("verifier".into()),
                status: VerificationStatus::Skipped,
                summary: "manual verifier override".into(),
                ..VerificationOutcome::default()
            }]
        ));
    }

    #[test]
    fn finalization_uses_latest_outcome_for_each_required_kind() {
        let mut decision = WorldGuardrailDecision::default();
        decision.mode = WorldGuardrailMode::Guarded;
        decision.allowed_to_finalize = false;
        decision.required_actions = vec![
            GuardrailRequiredAction::RunTests,
            GuardrailRequiredAction::RunBuild,
        ];
        let old_failure = VerificationOutcome {
            kind: VerificationKind::UnitTests,
            status: VerificationStatus::Failed,
            created_at: Utc::now() - chrono::Duration::seconds(10),
            ..VerificationOutcome::default()
        };
        let test_pass = VerificationOutcome {
            kind: VerificationKind::UnitTests,
            status: VerificationStatus::Passed,
            created_at: Utc::now(),
            ..VerificationOutcome::default()
        };

        assert!(!finalization_allowed(
            &decision,
            &[old_failure.clone(), test_pass.clone()]
        ));
        assert!(finalization_allowed(
            &decision,
            &[
                old_failure,
                test_pass,
                VerificationOutcome {
                    kind: VerificationKind::Build,
                    status: VerificationStatus::Passed,
                    created_at: Utc::now(),
                    ..VerificationOutcome::default()
                }
            ]
        ));
    }

    #[test]
    fn guardrail_budget_helper_keeps_prediction_and_overhead_budgets_separate() {
        assert!(guardrail_budget_allows_prediction_latency(100, 100, 40, 40));
        assert!(!guardrail_budget_allows_prediction_latency(101, 100, 1, 40));
        assert!(!guardrail_budget_allows_prediction_latency(
            100, 100, 41, 40
        ));
    }

    #[test]
    fn guardrail_overhead_budget_fails_open_without_double_decision() {
        let policy = WorldGuardrailPolicyConfig::default();
        let action = WorldGuardedAction::new(
            "s1",
            WorldAdvisorSurface::CodingTask,
            GuardedActionKind::UserRequest,
            "build app",
            "build a Python app",
        );
        let context = WorldGuardrailPredictionContext::from_scores(
            RuntimeTaskClass::CodingChange,
            WorldGuardrailMode::Guarded,
            GuardrailRiskScores {
                predicted_verification_needed: Some(0.90),
                ..GuardrailRiskScores::default()
            },
            &policy,
        );
        let decision = decide_guardrail(&action, None, context, &policy);
        let decision_id = decision.decision_id.clone();
        let idempotency_key = decision.idempotency_key.clone();

        let fail_open = enforce_guardrail_overhead_budget(decision, 41, 40);

        assert_eq!(fail_open.decision_id, decision_id);
        assert_eq!(fail_open.idempotency_key, idempotency_key);
        assert!(fail_open.allowed_to_continue);
        assert!(fail_open.allowed_to_finalize);
        assert!(fail_open.required_actions.is_empty());
        assert!(
            fail_open
                .reason_codes
                .contains(&GuardrailReasonCode::GuardrailOverheadExceeded)
        );
        assert!(
            fail_open
                .reason_codes
                .contains(&GuardrailReasonCode::PredictedVerificationNeededHigh)
        );
    }

    #[test]
    fn structured_outcome_maps_to_labels_without_prose() {
        let outcome = WorldGuardrailOutcome {
            final_status: GuardrailFinalStatus::BlockedFailedVerification,
            verification_outcomes: vec![VerificationOutcome {
                status: VerificationStatus::Failed,
                ..VerificationOutcome::default()
            }],
            user_correction_observed: true,
            plan_drift_observed: true,
            retry_count: 1,
            ..WorldGuardrailOutcome::default()
        };

        let labels = labels_from_guardrail_outcome(&outcome);

        assert_eq!(labels.success, Some(false));
        assert!(labels.failure);
        assert!(labels.verification_needed);
        assert!(labels.user_correction);
        assert!(labels.plan_drift);
        assert!(labels.retry);
    }

    #[test]
    fn ledgers_append_and_load_guardrail_rows() {
        let temp = tempfile::tempdir().unwrap();
        let action = WorldGuardedAction::new(
            "s1",
            WorldAdvisorSurface::InteractiveSession,
            GuardedActionKind::UserRequest,
            "build",
            "build",
        );
        append_guarded_action(temp.path(), &action).unwrap();
        let decision = WorldGuardrailDecision::unavailable(&action);
        append_guardrail_decision(temp.path(), &decision).unwrap();
        let verification = VerificationOutcome::passed(
            action.action_id.clone(),
            VerificationKind::UnitTests,
            "tests passed",
        );
        append_verification_outcome(temp.path(), &verification).unwrap();
        let outcome = WorldGuardrailOutcome::from_decision(
            &decision,
            RuntimeTaskClass::CodingChange,
            GuardrailFinalStatus::CompletedVerified,
            "done",
        );
        append_guardrail_outcome(temp.path(), &outcome).unwrap();

        let counts = guardrail_status_counts(temp.path());

        assert_eq!(counts.actions, 1);
        assert_eq!(counts.decisions, 1);
        assert_eq!(counts.verifications, 1);
        assert_eq!(counts.outcomes, 1);
        assert_eq!(counts.advisor_unavailable_decisions, 1);
    }

    #[test]
    fn guardrail_ledgers_skip_duplicate_idempotency_keys() {
        let temp = tempfile::tempdir().unwrap();
        let action = WorldGuardedAction::new(
            "s1",
            WorldAdvisorSurface::CodingTask,
            GuardedActionKind::UserRequest,
            "build",
            "build",
        );
        let decision = WorldGuardrailDecision::unavailable(&action);

        append_guarded_action(temp.path(), &action).unwrap();
        append_guarded_action(temp.path(), &action).unwrap();
        append_guardrail_decision(temp.path(), &decision).unwrap();
        append_guardrail_decision(temp.path(), &decision).unwrap();

        let counts = guardrail_status_counts(temp.path());

        assert_eq!(counts.actions, 1);
        assert_eq!(counts.decisions, 1);
    }
}
