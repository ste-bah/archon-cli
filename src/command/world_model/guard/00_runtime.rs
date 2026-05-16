use std::collections::{BTreeMap, HashMap, HashSet};
use std::str::FromStr;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use anyhow::{Result, bail};

#[derive(Debug, Clone)]
pub(crate) struct RuntimeGuardrailRecord {
    pub action: archon_world_model::WorldGuardedAction,
    pub advisory: archon_world_model::integration::WorldAdvisorSurfaceRecord,
    pub decision: archon_world_model::WorldGuardrailDecision,
    pub task_class: archon_world_model::RuntimeTaskClass,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PipelineStepGuardrailReport {
    pub steps_recorded: usize,
    pub parent_verifications_recorded: usize,
    pub failed_step_verifications: usize,
}

static ACTIVE_GUARDRAILS: OnceLock<Mutex<HashMap<String, RuntimeGuardrailRecord>>> =
    OnceLock::new();
static ACTIVE_OBSERVATIONS: OnceLock<Mutex<HashMap<String, GuardrailRuntimeObservations>>> =
    OnceLock::new();
const HIGH_SURPRISE_STATUS_THRESHOLD: f32 = 0.30;

#[derive(Debug, Clone, Default)]
struct GuardrailRuntimeObservations {
    provider_incident_observed: bool,
    user_correction_observed: bool,
    plan_drift_observed: bool,
    reasoning_failure_observed: bool,
    retry_count: u32,
    evidence_refs: Vec<String>,
}

fn active_guardrails() -> &'static Mutex<HashMap<String, RuntimeGuardrailRecord>> {
    ACTIVE_GUARDRAILS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn active_observations() -> &'static Mutex<HashMap<String, GuardrailRuntimeObservations>> {
    ACTIVE_OBSERVATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn active_guardrail_for_session(session_id: &str) -> Option<RuntimeGuardrailRecord> {
    active_guardrails()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(session_id)
        .cloned()
}

fn remember_active_guardrail(record: &RuntimeGuardrailRecord) {
    active_guardrails()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(record.action.session_id.clone(), record.clone());
    active_observations()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(record.action.action_id.clone())
        .or_default();
}

fn clear_active_guardrail(session_id: &str, action_id: &str) {
    let mut guard = active_guardrails()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard
        .get(session_id)
        .is_some_and(|record| record.action.action_id == action_id)
    {
        guard.remove(session_id);
        active_observations()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(action_id);
    }
}

fn observations_for(action_id: &str) -> GuardrailRuntimeObservations {
    active_observations()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(action_id)
        .cloned()
        .unwrap_or_default()
}

pub(crate) fn policy_from_config(
    config: &archon_core::config::ArchonConfig,
) -> archon_world_model::WorldGuardrailPolicyConfig {
    let cfg = &config.learning.world_model.guardrails;
    archon_world_model::WorldGuardrailPolicyConfig {
        enabled: cfg.enabled,
        interactive_mode: parse_mode(&cfg.interactive_mode),
        pipeline_mode: parse_mode(&cfg.pipeline_mode),
        tool_run_mode: parse_mode(&cfg.tool_run_mode),
        verification_run_mode: parse_mode(&cfg.verification_run_mode),
        high_risk_threshold: cfg.high_risk_threshold,
        medium_risk_threshold: cfg.medium_risk_threshold,
        critical_risk_threshold: cfg.critical_risk_threshold,
        require_tests_for_coding_high_risk: cfg.require_tests_for_coding_high_risk,
        require_build_for_coding_high_risk: cfg.require_build_for_coding_high_risk,
        require_lint_for_coding_high_risk: cfg.require_lint_for_coding_high_risk,
        require_typecheck_for_coding_high_risk: cfg.require_typecheck_for_coding_high_risk,
        require_plan_review_for_plan_drift: cfg.require_plan_review_for_plan_drift,
        require_source_check_for_research_high_risk: cfg
            .require_source_check_for_research_high_risk,
        require_manual_approval_for_critical: cfg.require_manual_approval_for_critical,
        max_guardrail_overhead_ms: cfg.max_guardrail_overhead_ms,
        record_outcomes_without_prediction: cfg.record_outcomes_without_prediction,
        max_guardrail_events_per_session: cfg.max_guardrail_events_per_session,
    }
}

pub(crate) fn begin_guarded_action(
    config: &archon_core::config::ArchonConfig,
    surface: archon_world_model::integration::WorldAdvisorSurface,
    session_id: &str,
    action_ref: &str,
    summary: &str,
) -> Option<RuntimeGuardrailRecord> {
    let policy = policy_from_config(config);
    if !policy.enabled {
        return None;
    }
    let mode = archon_world_model::guardrail::mode_for_surface(&policy, surface);
    if matches!(mode, archon_world_model::WorldGuardrailMode::Off) {
        return None;
    }
    let root = super::world_model_root().ok()?;
    let task_class = archon_world_model::guardrail::classify_task(summary, surface);
    let action_kind = match surface {
        archon_world_model::integration::WorldAdvisorSurface::PipelineStep => {
            archon_world_model::GuardedActionKind::PipelineStep
        }
        archon_world_model::integration::WorldAdvisorSurface::ToolRun => {
            archon_world_model::GuardedActionKind::ShellCommand
        }
        archon_world_model::integration::WorldAdvisorSurface::VerificationRun => {
            archon_world_model::GuardedActionKind::TestCommand
        }
        _ => archon_world_model::GuardedActionKind::UserRequest,
    };
    let mut action = archon_world_model::WorldGuardedAction::new(
        session_id,
        surface,
        action_kind,
        summary,
        summary,
    );
    action.action_id = action_ref.to_string();
    action.idempotency_key = format!("world_guardrail:action:{session_id}:{action_ref}");

    let advisory =
        super::runtime::record_runtime_advisory(config, surface, session_id, action_ref, summary);
    let guardrail_started_at = Instant::now();
    let decision = if advisory.prediction.is_some() {
        let scores = guardrail_scores_for_prediction(task_class, advisory.prediction.as_ref());
        let context = archon_world_model::WorldGuardrailPredictionContext::from_scores(
            task_class, mode, scores, &policy,
        );
        archon_world_model::guardrail::decide_guardrail(
            &action,
            advisory.prediction.as_ref(),
            context,
            &policy,
        )
    } else {
        archon_world_model::WorldGuardrailDecision::unavailable(&action)
    };
    let guardrail_overhead_ms = elapsed_ms_u64(guardrail_started_at);
    let decision = archon_world_model::guardrail::enforce_guardrail_overhead_budget(
        decision,
        guardrail_overhead_ms,
        policy.max_guardrail_overhead_ms,
    );
    action.verification_plan = verification_plan_for_decision(&action.action_id, &decision);
    let _ = archon_world_model::guardrail::append_guarded_action(&root, &action);
    let _ = archon_world_model::guardrail::append_guardrail_decision(&root, &decision);
    let record = RuntimeGuardrailRecord {
        action,
        advisory,
        decision,
        task_class,
    };
    remember_active_guardrail(&record);
    Some(record)
}

fn elapsed_ms_u64(started_at: Instant) -> u64 {
    let elapsed = started_at.elapsed().as_millis();
    elapsed.min(u128::from(u64::MAX)) as u64
}

fn guardrail_scores_for_prediction(
    task_class: archon_world_model::RuntimeTaskClass,
    prediction: Option<&archon_world_model::WorldPrediction>,
) -> archon_world_model::GuardrailRiskScores {
    prediction
        .and_then(|prediction| prediction.guardrail_scores)
        .unwrap_or_else(|| archon_world_model::guardrail::default_scores_for_task(task_class))
}

fn verification_plan_for_decision(
    action_id: &str,
    decision: &archon_world_model::WorldGuardrailDecision,
) -> Vec<archon_world_model::VerificationRequirement> {
    decision
        .required_actions
        .iter()
        .map(|required| verification_requirement_for(action_id, *required))
        .collect()
}

fn verification_requirement_for(
    action_id: &str,
    required: archon_world_model::GuardrailRequiredAction,
) -> archon_world_model::VerificationRequirement {
    let (suffix, kind, command_hint) = match required {
        archon_world_model::GuardrailRequiredAction::RunTests => (
            "run-tests",
            archon_world_model::VerificationKind::UnitTests,
            Some("cargo test / pytest / npm test".to_string()),
        ),
        archon_world_model::GuardrailRequiredAction::RunBuild => (
            "run-build",
            archon_world_model::VerificationKind::Build,
            Some("cargo check / cargo build / npm run build".to_string()),
        ),
        archon_world_model::GuardrailRequiredAction::RunLint => (
            "run-lint",
            archon_world_model::VerificationKind::Lint,
            Some("cargo clippy / ruff check / npm run lint".to_string()),
        ),
        archon_world_model::GuardrailRequiredAction::RunTypecheck => (
            "run-typecheck",
            archon_world_model::VerificationKind::Typecheck,
            Some("mypy / tsc / npm run typecheck".to_string()),
        ),
        archon_world_model::GuardrailRequiredAction::RunVerifier => (
            "run-verifier",
            archon_world_model::VerificationKind::Custom("verifier".into()),
            Some("run the configured verifier for this task".to_string()),
        ),
        archon_world_model::GuardrailRequiredAction::ReviewPlanAgainstUserGoal => (
            "review-plan",
            archon_world_model::VerificationKind::Custom("plan_review".into()),
            Some("compare the final plan/work against the original user request".to_string()),
        ),
        archon_world_model::GuardrailRequiredAction::CheckSourceEvidence => (
            "check-source-evidence",
            archon_world_model::VerificationKind::SourceEvidenceCheck,
            Some("verify cited source evidence before finalizing".to_string()),
        ),
        archon_world_model::GuardrailRequiredAction::RecordManualOutcome => (
            "record-manual-outcome",
            archon_world_model::VerificationKind::Custom("manual_outcome".into()),
            Some("record the operator-observed outcome".to_string()),
        ),
        archon_world_model::GuardrailRequiredAction::RequireUserApproval => (
            "require-user-approval",
            archon_world_model::VerificationKind::HumanApproval,
            Some("obtain explicit user/operator approval".to_string()),
        ),
    };
    archon_world_model::VerificationRequirement {
        requirement_id: format!("world-guard-req-{action_id}-{suffix}"),
        kind,
        command_hint,
        applies_to: vec![action_id.to_string()],
        required_for_final: true,
    }
}

pub(crate) fn record_guardrail_turn_outcome(
    config: &archon_core::config::ArchonConfig,
    record: &RuntimeGuardrailRecord,
    turn_completed: bool,
) -> Option<archon_world_model::WorldGuardrailOutcome> {
    record_guardrail_completion_outcome(config, record, turn_completed, "", None)
}

pub(crate) fn record_guardrail_completion_outcome(
    config: &archon_core::config::ArchonConfig,
    record: &RuntimeGuardrailRecord,
    completed: bool,
    actual_summary: &str,
    bundle_id: Option<&str>,
) -> Option<archon_world_model::WorldGuardrailOutcome> {
    let root = super::world_model_root().ok()?;
    let verification_outcomes = archon_world_model::guardrail::load_verification_outcomes(&root)
        .unwrap_or_default()
        .into_iter()
        .filter(|outcome| outcome.action_id == record.action.action_id)
        .collect::<Vec<_>>();
    let has_manual_override = verification_outcomes.iter().any(|outcome| {
        outcome.status == archon_world_model::VerificationStatus::Skipped
            || outcome
                .evidence_refs
                .iter()
                .any(|evidence| evidence.starts_with("manual_override:"))
    });
    let observations = observations_for(&record.action.action_id);
    let final_status = if !completed {
        archon_world_model::GuardrailFinalStatus::Failed
    } else if archon_world_model::guardrail::finalization_allowed(
        &record.decision,
        &verification_outcomes,
    ) {
        if has_manual_override {
            archon_world_model::GuardrailFinalStatus::UserApprovedDespiteRisk
        } else if record.decision.required_actions.is_empty() {
            archon_world_model::GuardrailFinalStatus::CompletedWithCaveat
        } else {
            archon_world_model::GuardrailFinalStatus::CompletedVerified
        }
    } else if verification_outcomes
        .iter()
        .any(|outcome| outcome.status == archon_world_model::VerificationStatus::Failed)
        || (observations.reasoning_failure_observed && record.decision.mode.can_block())
    {
        archon_world_model::GuardrailFinalStatus::BlockedFailedVerification
    } else if !record.decision.allowed_to_finalize
        && record.decision.mode.can_block()
        && !record.decision.required_actions.is_empty()
    {
        archon_world_model::GuardrailFinalStatus::BlockedMissingVerification
    } else {
        archon_world_model::GuardrailFinalStatus::CompletedWithCaveat
    };
    let default_summary = match final_status {
        archon_world_model::GuardrailFinalStatus::BlockedMissingVerification => {
            "interactive turn completed but required verification was missing"
        }
        archon_world_model::GuardrailFinalStatus::Failed => "interactive turn failed",
        archon_world_model::GuardrailFinalStatus::BlockedFailedVerification => {
            "interactive turn completed but required verification failed"
        }
        archon_world_model::GuardrailFinalStatus::UserApprovedDespiteRisk => {
            "interactive turn completed after manual guardrail approval"
        }
        _ => "interactive turn completed",
    };
    let actual_summary = if actual_summary.trim().is_empty() {
        default_summary.to_string()
    } else if matches!(
        final_status,
        archon_world_model::GuardrailFinalStatus::BlockedMissingVerification
    ) {
        format!("{default_summary}; output: {}", actual_summary.trim())
    } else {
        actual_summary.trim().to_string()
    };
    let mut outcome = archon_world_model::WorldGuardrailOutcome::from_decision(
        &record.decision,
        record.task_class,
        final_status,
        actual_summary,
    );
    outcome.verification_outcomes = verification_outcomes;
    outcome.provider_incident_observed = observations.provider_incident_observed;
    outcome.user_correction_observed = observations.user_correction_observed;
    outcome.plan_drift_observed = observations.plan_drift_observed;
    outcome.retry_count = observations.retry_count;
    outcome.evidence_refs.extend(observations.evidence_refs);
    if matches!(
        final_status,
        archon_world_model::GuardrailFinalStatus::BlockedMissingVerification
    ) {
        outcome
            .evidence_refs
            .push("guardrail:missing_verification".into());
    }
    super::runtime::record_runtime_guardrail_outcome(
        config,
        &record.advisory,
        &mut outcome,
        bundle_id,
    );
    let _ = archon_world_model::guardrail::append_guardrail_outcome(&root, &outcome);
    if !matches!(
        final_status,
        archon_world_model::GuardrailFinalStatus::BlockedMissingVerification
            | archon_world_model::GuardrailFinalStatus::BlockedFailedVerification
    ) {
        clear_active_guardrail(&record.action.session_id, &record.action.action_id);
    }
    Some(outcome)
}

pub(crate) fn record_guardrail_pipeline_steps(
    config: &archon_core::config::ArchonConfig,
    parent: &RuntimeGuardrailRecord,
    result: &archon_pipeline::runner::PipelineResult,
) -> PipelineStepGuardrailReport {
    if !config.learning.world_model.guardrails.enabled {
        return PipelineStepGuardrailReport::default();
    }
    let Ok(root) = super::world_model_root() else {
        return PipelineStepGuardrailReport::default();
    };
    let mut report = PipelineStepGuardrailReport::default();

    for (ordinal, (agent, agent_result)) in result.agent_results.iter().enumerate() {
        let step_action_id = pipeline_step_action_id(&parent.action.action_id, ordinal, agent);
        let mut action = archon_world_model::WorldGuardedAction::new(
            &result.session_id,
            archon_world_model::integration::WorldAdvisorSurface::PipelineStep,
            archon_world_model::GuardedActionKind::PipelineStep,
            &parent.action.user_goal,
            pipeline_step_summary(agent, agent_result, ordinal),
        );
        action.action_id = step_action_id.clone();
        action.parent_action_id = Some(parent.action.action_id.clone());
        action.idempotency_key = format!(
            "world_guardrail:pipeline_step:{}:{ordinal}:{}",
            result.session_id, agent.key
        );
        let _ = archon_world_model::guardrail::append_guarded_action(&root, &action);

        let mut decision = parent.decision.clone();
        decision.decision_id = format!("world-guard-decision-{}", uuid::Uuid::new_v4());
        decision.action_id = action.action_id.clone();
        decision.surface = archon_world_model::integration::WorldAdvisorSurface::PipelineStep;
        decision.required_actions.clear();
        decision.allowed_to_continue = true;
        decision.allowed_to_finalize = true;
        decision.idempotency_key = format!("world_guardrail:decision:{}", action.action_id);
        decision.created_at = chrono::Utc::now();
        let _ = archon_world_model::guardrail::append_guardrail_decision(&root, &decision);

        let final_status = if pipeline_agent_quality_failed(agent, agent_result) {
            archon_world_model::GuardrailFinalStatus::Failed
        } else {
            archon_world_model::GuardrailFinalStatus::CompletedWithCaveat
        };
        let mut outcome = archon_world_model::WorldGuardrailOutcome::from_decision(
            &decision,
            parent.task_class,
            final_status,
            pipeline_step_outcome_summary(agent, agent_result),
        );
        outcome
            .evidence_refs
            .push(format!("parent_guarded_action:{}", parent.action.action_id));
        outcome
            .evidence_refs
            .push(format!("pipeline_session:{}", result.session_id));
        let _ = archon_world_model::guardrail::append_guardrail_outcome(&root, &outcome);
        report.steps_recorded += 1;

        if let Some(verification) =
            pipeline_agent_verification(parent, &step_action_id, agent, agent_result)
        {
            if verification.status == archon_world_model::VerificationStatus::Failed {
                report.failed_step_verifications += 1;
            }
            let _ =
                archon_world_model::guardrail::append_verification_outcome(&root, &verification);
            report.parent_verifications_recorded += 1;
        }
    }

    report
}

