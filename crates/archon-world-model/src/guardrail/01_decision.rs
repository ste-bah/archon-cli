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
        latest_by_kind
            .get(*key)
            .is_some_and(|outcome| verification_satisfies_required_action(outcome))
    })
}

fn verification_satisfies_required_action(outcome: &VerificationOutcome) -> bool {
    match outcome.status {
        VerificationStatus::Passed => true,
        VerificationStatus::Skipped => outcome
            .evidence_refs
            .iter()
            .any(|evidence| evidence.starts_with("manual_override:")),
        _ => false,
    }
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

