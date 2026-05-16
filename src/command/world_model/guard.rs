//! `archon world guard` command rendering and policy helpers.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::str::FromStr;
use std::sync::{Mutex, OnceLock};

use anyhow::{Result, bail};

#[derive(Debug, Clone)]
pub(crate) struct RuntimeGuardrailRecord {
    pub action: archon_world_model::WorldGuardedAction,
    pub advisory: archon_world_model::integration::WorldAdvisorSurfaceRecord,
    pub decision: archon_world_model::WorldGuardrailDecision,
    pub task_class: archon_world_model::RuntimeTaskClass,
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
    let decision = if advisory.prediction.is_some() {
        let scores = archon_world_model::guardrail::default_scores_for_task(task_class);
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

pub(crate) fn record_guardrail_provider_incident_for_session(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    provider_event_id: &str,
    reason_code: &str,
) -> bool {
    if !config.learning.world_model.guardrails.enabled {
        return false;
    }
    let Some(parent) = active_guardrail_for_session(session_id) else {
        return false;
    };
    let mut observations = active_observations()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = observations
        .entry(parent.action.action_id.clone())
        .or_default();
    entry.provider_incident_observed = true;
    if provider_reason_implies_retry(reason_code) {
        entry.retry_count = entry.retry_count.saturating_add(1);
    }
    entry
        .evidence_refs
        .push(format!("provider_event:{provider_event_id}"));
    entry
        .evidence_refs
        .push(format!("provider_reason:{reason_code}"));
    entry.evidence_refs.sort();
    entry.evidence_refs.dedup();
    true
}

fn provider_reason_implies_retry(reason_code: &str) -> bool {
    let reason = reason_code.to_ascii_lowercase();
    reason.contains("rate")
        || reason.contains("timeout")
        || reason.contains("transient")
        || reason.contains("overloaded")
        || reason.contains("quota")
}

pub(crate) fn record_guardrail_reasoning_quality_event(
    event: &archon_reasoning_quality::ReasoningQualityEvent,
) -> bool {
    let Some(parent) = active_guardrail_for_session(&event.session_id) else {
        return false;
    };
    let mut observations = active_observations()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = observations
        .entry(parent.action.action_id.clone())
        .or_default();
    match event.event_kind {
        archon_reasoning_quality::ReasoningEventKind::ClaimCorrectedByUser => {
            entry.user_correction_observed = true;
            entry.reasoning_failure_observed = true;
        }
        archon_reasoning_quality::ReasoningEventKind::ClaimContradictedBySource => {
            entry.plan_drift_observed = true;
            entry.reasoning_failure_observed = true;
        }
        archon_reasoning_quality::ReasoningEventKind::CompletionClaimWithoutEvidence
        | archon_reasoning_quality::ReasoningEventKind::TestStatusClaimWithoutCommand
        | archon_reasoning_quality::ReasoningEventKind::UnsupportedClaim => {
            entry.reasoning_failure_observed = true;
        }
        archon_reasoning_quality::ReasoningEventKind::VerificationNeeded => {
            entry.plan_drift_observed = true;
        }
        _ => {}
    }
    entry
        .evidence_refs
        .push(format!("reasoning_quality:{}", event.event_id));
    entry
        .evidence_refs
        .push(format!("reasoning_kind:{:?}", event.event_kind));
    entry.evidence_refs.sort();
    entry.evidence_refs.dedup();
    true
}

pub(crate) fn record_guardrail_tool_result_for_session(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    command: &str,
    is_error: bool,
    output_summary: &str,
) -> Option<archon_world_model::VerificationOutcome> {
    if !config.learning.world_model.guardrails.enabled {
        return None;
    }
    let parent = active_guardrail_for_session(session_id)?;
    record_guardrail_tool_result(config, &parent, command, is_error, output_summary)
}

pub(crate) fn record_guardrail_tool_result(
    _config: &archon_core::config::ArchonConfig,
    parent: &RuntimeGuardrailRecord,
    command: &str,
    is_error: bool,
    output_summary: &str,
) -> Option<archon_world_model::VerificationOutcome> {
    let root = super::world_model_root().ok()?;
    let (action_kind, verification_kind) = classify_tool_command(command);
    let surface = if verification_kind.is_some() {
        archon_world_model::integration::WorldAdvisorSurface::VerificationRun
    } else {
        archon_world_model::integration::WorldAdvisorSurface::ToolRun
    };
    let mut action = archon_world_model::WorldGuardedAction::new(
        &parent.action.session_id,
        surface,
        action_kind,
        &parent.action.user_goal,
        command,
    );
    action.parent_action_id = Some(parent.action.action_id.clone());
    action.commands_planned.push(command.to_string());
    let _ = archon_world_model::guardrail::append_guarded_action(&root, &action);

    if let Some(kind) = verification_kind {
        let status = if is_error {
            archon_world_model::VerificationStatus::Failed
        } else {
            archon_world_model::VerificationStatus::Passed
        };
        let mut verification = archon_world_model::VerificationOutcome {
            schema_version: archon_world_model::guardrail::CURRENT_SCHEMA_VERSION,
            requirement_id: matching_requirement_id(&parent.action, &kind),
            action_id: parent.action.action_id.clone(),
            kind,
            status,
            command: Some(command.to_string()),
            exit_code: Some(if is_error { 1 } else { 0 }),
            summary: if output_summary.trim().is_empty() {
                format!("command completed; is_error={is_error}")
            } else {
                output_summary.trim().chars().take(500).collect()
            },
            evidence_refs: vec![format!("guardrail_tool_action:{}", action.action_id)],
            idempotency_key: format!(
                "world_guardrail:verification:{}:{}",
                parent.action.action_id, action.action_id
            ),
            created_at: chrono::Utc::now(),
        };
        if verification.status == archon_world_model::VerificationStatus::Failed {
            verification
                .evidence_refs
                .push("guardrail:verification_failed".into());
        }
        let _ = archon_world_model::guardrail::append_verification_outcome(&root, &verification);
        Some(verification)
    } else {
        let decision = archon_world_model::WorldGuardrailDecision::unavailable(&action);
        let final_status = if is_error {
            archon_world_model::GuardrailFinalStatus::Failed
        } else {
            archon_world_model::GuardrailFinalStatus::CompletedWithCaveat
        };
        let mut outcome = archon_world_model::WorldGuardrailOutcome::from_decision(
            &decision,
            parent.task_class,
            final_status,
            if output_summary.trim().is_empty() {
                format!("tool command completed; is_error={is_error}")
            } else {
                output_summary.trim().chars().take(500).collect::<String>()
            },
        );
        outcome
            .evidence_refs
            .push(format!("parent_guarded_action:{}", parent.action.action_id));
        let _ = archon_world_model::guardrail::append_guardrail_outcome(&root, &outcome);
        None
    }
}

fn matching_requirement_id(
    action: &archon_world_model::WorldGuardedAction,
    kind: &archon_world_model::VerificationKind,
) -> String {
    action
        .verification_plan
        .iter()
        .find(|requirement| requirement.kind == *kind)
        .map(|requirement| requirement.requirement_id.clone())
        .unwrap_or_default()
}

fn classify_tool_command(
    command: &str,
) -> (
    archon_world_model::GuardedActionKind,
    Option<archon_world_model::VerificationKind>,
) {
    let lower = command.to_ascii_lowercase();
    if lower.contains("cargo test")
        || lower.contains("pytest")
        || lower.contains("npm test")
        || lower.contains("pnpm test")
        || lower.contains("yarn test")
        || lower.contains("go test")
        || lower.contains(" test ")
    {
        return (
            archon_world_model::GuardedActionKind::TestCommand,
            Some(archon_world_model::VerificationKind::UnitTests),
        );
    }
    if lower.contains("cargo build")
        || lower.contains("cargo check")
        || lower.contains("npm run build")
        || lower.contains("pnpm build")
        || lower.contains("go build")
    {
        return (
            archon_world_model::GuardedActionKind::BuildCommand,
            Some(archon_world_model::VerificationKind::Build),
        );
    }
    if lower.contains("clippy")
        || lower.contains("eslint")
        || lower.contains("ruff check")
        || lower.contains(" lint")
    {
        return (
            archon_world_model::GuardedActionKind::LintCommand,
            Some(archon_world_model::VerificationKind::Lint),
        );
    }
    if lower.contains("tsc") || lower.contains("mypy") || lower.contains("typecheck") {
        return (
            archon_world_model::GuardedActionKind::TypecheckCommand,
            Some(archon_world_model::VerificationKind::Typecheck),
        );
    }
    if lower.contains("fmt --check")
        || lower.contains("format --check")
        || lower.contains("cargo fmt")
    {
        return (
            archon_world_model::GuardedActionKind::TestCommand,
            Some(archon_world_model::VerificationKind::FormatCheck),
        );
    }
    (archon_world_model::GuardedActionKind::ShellCommand, None)
}

pub(crate) fn forced_repair_prompt(record: &RuntimeGuardrailRecord) -> Option<String> {
    if record.decision.allowed_to_finalize || !record.decision.mode.can_block() {
        return None;
    }
    if record.decision.required_actions.is_empty() {
        return None;
    }
    Some(format!(
        "World-model guardrail forced repair for action {}.\n\n\
         The previous turn must not be marked complete yet. Required verification is missing: {:?}.\n\
         Run the missing verification or explain the blocker before making any completion claim.",
        record.action.action_id, record.decision.required_actions
    ))
}

pub(crate) fn render_guard_status(
    config: &archon_core::config::ArchonConfig,
    root: &std::path::Path,
) -> String {
    let policy = policy_from_config(config);
    let counts = archon_world_model::guardrail::guardrail_status_counts(root);
    let summary = guardrail_status_summary(root);
    let unavailable = if summary.unavailable_by_reason.is_empty() {
        "none".to_string()
    } else {
        summary
            .unavailable_by_reason
            .iter()
            .map(|(reason, count)| format!("{reason}={count}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let latest_actions = if summary.latest_actions.is_empty() {
        "none".to_string()
    } else {
        summary
            .latest_actions
            .iter()
            .map(|action| format!("{}:{:?}", action.action_id, action.surface))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let blocked_actions = if summary.blocked_actions.is_empty() {
        "none".to_string()
    } else {
        summary.blocked_actions.join(", ")
    };
    format!(
        "World Model Guardrails\n\
         ======================\n\
         Enabled:                  {enabled}\n\
         Interactive mode:         {interactive_mode:?}\n\
         Pipeline mode:            {pipeline_mode:?}\n\
         Tool-run mode:            {tool_run_mode:?}\n\
         Verification-run mode:    {verification_run_mode:?}\n\
         Medium risk threshold:    {medium:.2}\n\
         High risk threshold:      {high:.2}\n\
         Critical risk threshold:  {critical:.2}\n\
         Guardrail overhead cap:   {overhead_ms}ms\n\
         Actions:                  {actions}\n\
         Decisions:                {decisions}\n\
         Verifications:            {verifications}\n\
         Outcomes:                 {outcomes}\n\
         Blocked outcomes:         {blocked}\n\
         Failed verifications:     {failed}\n\
         Missing verifications:    {missing}\n\
         Outcomes without pred:    {without_prediction}\n\
         Advisor unavailable:      {unavailable_count}\n\
         Unavailable reasons:      {unavailable}\n\
         User overrides:           {overrides}\n\
         High-surprise outcomes:   {high_surprise}\n\
         Missing requirements:     {missing_requirements}\n\
         Latest actions:           {latest_actions}\n\
         Blocked actions:          {blocked_actions}",
        enabled = policy.enabled,
        interactive_mode = policy.interactive_mode,
        pipeline_mode = policy.pipeline_mode,
        tool_run_mode = policy.tool_run_mode,
        verification_run_mode = policy.verification_run_mode,
        medium = policy.medium_risk_threshold,
        high = policy.high_risk_threshold,
        critical = policy.critical_risk_threshold,
        overhead_ms = policy.max_guardrail_overhead_ms,
        actions = counts.actions,
        decisions = counts.decisions,
        verifications = counts.verifications,
        outcomes = counts.outcomes,
        blocked = counts.blocked_outcomes,
        failed = counts.failed_verifications,
        missing = counts.missing_verifications,
        without_prediction = counts.outcomes_without_prediction,
        unavailable_count = counts.advisor_unavailable_decisions,
        overrides = summary.user_overrides,
        high_surprise = summary.high_surprise_outcomes,
        missing_requirements = summary.missing_requirements,
    )
}

#[derive(Debug, Default)]
struct GuardrailStatusSummary {
    unavailable_by_reason: BTreeMap<String, usize>,
    user_overrides: usize,
    high_surprise_outcomes: usize,
    missing_requirements: usize,
    latest_actions: Vec<archon_world_model::WorldGuardedAction>,
    blocked_actions: Vec<String>,
}

fn guardrail_status_summary(root: &std::path::Path) -> GuardrailStatusSummary {
    let actions = archon_world_model::guardrail::load_guarded_actions(root).unwrap_or_default();
    let decisions =
        archon_world_model::guardrail::load_guardrail_decisions(root).unwrap_or_default();
    let verifications =
        archon_world_model::guardrail::load_verification_outcomes(root).unwrap_or_default();
    let outcomes = archon_world_model::guardrail::load_guardrail_outcomes(root).unwrap_or_default();
    let mut summary = GuardrailStatusSummary::default();

    for decision in &decisions {
        for reason in &decision.reason_codes {
            if *reason == archon_world_model::GuardrailReasonCode::AdvisorUnavailable {
                *summary
                    .unavailable_by_reason
                    .entry(format!("{reason:?}"))
                    .or_default() += 1;
            }
        }
        let action_verifications = verifications
            .iter()
            .filter(|verification| verification.action_id == decision.action_id)
            .cloned()
            .collect::<Vec<_>>();
        if !decision.allowed_to_finalize
            && !archon_world_model::guardrail::finalization_allowed(decision, &action_verifications)
        {
            summary.missing_requirements += decision.required_actions.len();
        }
    }

    let mut override_actions = HashSet::<String>::new();
    for verification in &verifications {
        if verification.status == archon_world_model::VerificationStatus::Skipped
            || verification
                .evidence_refs
                .iter()
                .any(|evidence| evidence.starts_with("manual_override:"))
        {
            override_actions.insert(verification.action_id.clone());
        }
    }
    for outcome in &outcomes {
        if outcome.final_status == archon_world_model::GuardrailFinalStatus::UserApprovedDespiteRisk
        {
            override_actions.insert(outcome.action_id.clone());
        }
        if outcome.latent_surprise.unwrap_or(0.0) >= HIGH_SURPRISE_STATUS_THRESHOLD {
            summary.high_surprise_outcomes += 1;
        }
    }
    summary.user_overrides = override_actions.len();

    let mut latest_actions = actions.clone();
    latest_actions.sort_by_key(|action| std::cmp::Reverse(action.created_at));
    summary.latest_actions = latest_actions.into_iter().take(5).collect();
    summary.blocked_actions = actions
        .iter()
        .filter(|action| {
            action_guard_status(action, &decisions, &verifications, &outcomes) == "blocked"
        })
        .map(|action| action.action_id.clone())
        .collect();
    summary
}

pub(crate) fn render_guard_policy(config: &archon_core::config::ArchonConfig) -> String {
    let policy = policy_from_config(config);
    format!(
        "World Model Guardrail Policy\n\
         ============================\n\
         Enabled: {enabled}\n\
         interactive_mode = {interactive_mode:?}\n\
         pipeline_mode = {pipeline_mode:?}\n\
         tool_run_mode = {tool_run_mode:?}\n\
         verification_run_mode = {verification_run_mode:?}\n\
         medium_risk_threshold = {medium:.2}\n\
         high_risk_threshold = {high:.2}\n\
         critical_risk_threshold = {critical:.2}\n\
         require_tests_for_coding_high_risk = {require_tests}\n\
         require_build_for_coding_high_risk = {require_build}\n\
         require_lint_for_coding_high_risk = {require_lint}\n\
         require_typecheck_for_coding_high_risk = {require_typecheck}\n\
         require_plan_review_for_plan_drift = {require_plan_review}\n\
         require_source_check_for_research_high_risk = {require_source_check}\n\
         require_manual_approval_for_critical = {require_approval}\n\
         max_guardrail_overhead_ms = {overhead_ms}\n\
         record_outcomes_without_prediction = {record_without_prediction}\n\
         max_guardrail_events_per_session = {max_events}",
        enabled = policy.enabled,
        interactive_mode = policy.interactive_mode,
        pipeline_mode = policy.pipeline_mode,
        tool_run_mode = policy.tool_run_mode,
        verification_run_mode = policy.verification_run_mode,
        medium = policy.medium_risk_threshold,
        high = policy.high_risk_threshold,
        critical = policy.critical_risk_threshold,
        require_tests = policy.require_tests_for_coding_high_risk,
        require_build = policy.require_build_for_coding_high_risk,
        require_lint = policy.require_lint_for_coding_high_risk,
        require_typecheck = policy.require_typecheck_for_coding_high_risk,
        require_plan_review = policy.require_plan_review_for_plan_drift,
        require_source_check = policy.require_source_check_for_research_high_risk,
        require_approval = policy.require_manual_approval_for_critical,
        overhead_ms = policy.max_guardrail_overhead_ms,
        record_without_prediction = policy.record_outcomes_without_prediction,
        max_events = policy.max_guardrail_events_per_session,
    )
}

pub(crate) fn render_guard_list(
    root: &std::path::Path,
    session: Option<&str>,
    surface: Option<&str>,
    status: Option<&str>,
) -> Result<String> {
    let mut actions = archon_world_model::guardrail::load_guarded_actions(root)?;
    let decisions = archon_world_model::guardrail::load_guardrail_decisions(root)?;
    let verifications = archon_world_model::guardrail::load_verification_outcomes(root)?;
    let outcomes = archon_world_model::guardrail::load_guardrail_outcomes(root)?;
    if let Some(session) = session {
        actions.retain(|action| action.session_id == session);
    }
    if let Some(surface) = surface {
        actions.retain(|action| format!("{:?}", action.surface).eq_ignore_ascii_case(surface));
    }
    if let Some(status) = status
        && !matches!(status, "all" | "blocked" | "open" | "complete")
    {
        bail!("guard list --status must be all, blocked, open, or complete");
    }
    if let Some(status) = status
        && status != "all"
    {
        actions.retain(|action| {
            action_guard_status(action, &decisions, &verifications, &outcomes) == status
        });
    }
    actions.sort_by_key(|action| std::cmp::Reverse(action.created_at));
    let mut lines = vec![
        "World Model Guardrail Actions".to_string(),
        "===============================".to_string(),
    ];
    if actions.is_empty() {
        lines.push("No guarded actions recorded.".into());
        return Ok(lines.join("\n"));
    }
    for action in actions.into_iter().take(50) {
        let status = action_guard_status(&action, &decisions, &verifications, &outcomes);
        lines.push(format!(
            "{} {} [{}] {:?} {:?} {}",
            action.created_at.to_rfc3339(),
            action.action_id,
            status,
            action.surface,
            action.action_kind,
            action.action_summary
        ));
    }
    Ok(lines.join("\n"))
}

fn action_guard_status(
    action: &archon_world_model::WorldGuardedAction,
    decisions: &[archon_world_model::WorldGuardrailDecision],
    verifications: &[archon_world_model::VerificationOutcome],
    outcomes: &[archon_world_model::WorldGuardrailOutcome],
) -> &'static str {
    if let Some(outcome) = outcomes
        .iter()
        .filter(|outcome| outcome.action_id == action.action_id)
        .max_by_key(|outcome| outcome.created_at)
    {
        return if matches!(
            outcome.final_status,
            archon_world_model::GuardrailFinalStatus::BlockedMissingVerification
                | archon_world_model::GuardrailFinalStatus::BlockedFailedVerification
        ) {
            "blocked"
        } else {
            "complete"
        };
    }
    if let Some(decision) = decisions
        .iter()
        .filter(|decision| decision.action_id == action.action_id)
        .max_by_key(|decision| decision.created_at)
    {
        let action_verifications = verifications
            .iter()
            .filter(|verification| verification.action_id == action.action_id)
            .cloned()
            .collect::<Vec<_>>();
        if decision.allowed_to_finalize
            || archon_world_model::guardrail::finalization_allowed(decision, &action_verifications)
        {
            "complete"
        } else {
            "open"
        }
    } else {
        "open"
    }
}

pub(crate) fn render_guard_inspect(root: &std::path::Path, action_id: &str) -> Result<String> {
    let action = archon_world_model::guardrail::load_guarded_actions(root)?
        .into_iter()
        .find(|action| action.action_id == action_id)
        .ok_or_else(|| anyhow::anyhow!("guarded action not found: {action_id}"))?;
    let decisions = archon_world_model::guardrail::load_guardrail_decisions(root)?
        .into_iter()
        .filter(|decision| decision.action_id == action_id)
        .collect::<Vec<_>>();
    let verifications = archon_world_model::guardrail::load_verification_outcomes(root)?
        .into_iter()
        .filter(|verification| verification.action_id == action_id)
        .collect::<Vec<_>>();
    let outcomes = archon_world_model::guardrail::load_guardrail_outcomes(root)?
        .into_iter()
        .filter(|outcome| outcome.action_id == action_id)
        .collect::<Vec<_>>();
    Ok(format!(
        "World Model Guardrail Inspect\n\
         =============================\n\
         Action:        {action_id}\n\
         Session:       {session}\n\
         Surface:       {surface:?}\n\
         Kind:          {kind:?}\n\
         Summary:       {summary}\n\
         Decisions:     {decisions}\n\
         Verifications: {verifications}\n\
         Outcomes:      {outcomes}",
        session = action.session_id,
        surface = action.surface,
        kind = action.action_kind,
        summary = action.action_summary,
        decisions = decisions.len(),
        verifications = verifications.len(),
        outcomes = outcomes.len(),
    ))
}

pub(crate) fn render_guard_replay_outcomes(
    root: &std::path::Path,
    session: Option<&str>,
) -> Result<String> {
    let outcomes = archon_world_model::guardrail::load_guardrail_outcomes(root)?;
    let count = outcomes
        .iter()
        .filter(|outcome| session.map_or(true, |session| outcome.action_id.contains(session)))
        .count();
    Ok(format!(
        "World Model Guardrail Replay\n\
         ============================\n\
         Outcomes scanned: {count}\n\
         Replay status:    no-op (structured outcomes already persisted)"
    ))
}

pub(crate) fn render_guard_approve(
    root: &std::path::Path,
    action_id: &str,
    reason: &str,
) -> Result<String> {
    let reason = required_reason(reason)?;
    let action = load_guarded_action(root, action_id)?;
    let decision = latest_decision_for_action(root, &action)?;
    let task_class = decision
        .prediction_context
        .as_ref()
        .map(|context| context.task_class)
        .unwrap_or_else(|| {
            archon_world_model::guardrail::classify_task(&action.action_summary, action.surface)
        });

    let required_actions = if decision.required_actions.is_empty() {
        vec![archon_world_model::GuardrailRequiredAction::RequireUserApproval]
    } else {
        decision.required_actions.clone()
    };
    let mut verification_count = 0usize;
    for required in required_actions {
        let requirement = requirement_for_required_action(&action, required);
        let status = if required == archon_world_model::GuardrailRequiredAction::RequireUserApproval
        {
            archon_world_model::VerificationStatus::Passed
        } else {
            archon_world_model::VerificationStatus::Skipped
        };
        append_manual_verification(root, action_id, &requirement, status, "approve", reason)?;
        verification_count += 1;
    }

    let mut outcome = archon_world_model::WorldGuardrailOutcome::from_decision(
        &decision,
        task_class,
        archon_world_model::GuardrailFinalStatus::UserApprovedDespiteRisk,
        format!("manual approval despite unresolved risk: {reason}"),
    );
    outcome.verification_outcomes =
        archon_world_model::guardrail::load_verification_outcomes(root)?
            .into_iter()
            .filter(|verification| verification.action_id == action_id)
            .collect();
    outcome.evidence_refs.push("manual_override:approve".into());
    archon_world_model::guardrail::append_guardrail_outcome(root, &outcome)?;
    clear_active_guardrail(&action.session_id, action_id);

    Ok(format!(
        "World Model Guardrail Approval\n\
         ==============================\n\
         Action: {action_id}\n\
         Outcome: user_approved_despite_risk\n\
         Manual verification records: {verification_count}\n\
         Reason: {reason}"
    ))
}

pub(crate) fn render_guard_skip_verification(
    root: &std::path::Path,
    requirement_id: &str,
    reason: &str,
) -> Result<String> {
    let reason = required_reason(reason)?;
    let (action, requirement) = load_requirement(root, requirement_id)?;
    let verification = append_manual_verification(
        root,
        &action.action_id,
        &requirement,
        archon_world_model::VerificationStatus::Skipped,
        "skip_verification",
        reason,
    )?;

    Ok(format!(
        "World Model Guardrail Verification Skip\n\
         =====================================\n\
         Requirement: {requirement_id}\n\
         Action:      {action_id}\n\
         Kind:        {kind:?}\n\
         Status:      skipped\n\
         Reason:      {reason}",
        action_id = action.action_id,
        kind = verification.kind,
    ))
}

fn required_reason(reason: &str) -> Result<&str> {
    let reason = reason.trim();
    if reason.is_empty() {
        bail!("manual guardrail override requires --reason");
    }
    Ok(reason)
}

fn load_guarded_action(
    root: &std::path::Path,
    action_id: &str,
) -> Result<archon_world_model::WorldGuardedAction> {
    archon_world_model::guardrail::load_guarded_actions(root)?
        .into_iter()
        .find(|action| action.action_id == action_id)
        .ok_or_else(|| anyhow::anyhow!("guarded action not found: {action_id}"))
}

fn latest_decision_for_action(
    root: &std::path::Path,
    action: &archon_world_model::WorldGuardedAction,
) -> Result<archon_world_model::WorldGuardrailDecision> {
    Ok(
        archon_world_model::guardrail::load_guardrail_decisions(root)?
            .into_iter()
            .filter(|decision| decision.action_id == action.action_id)
            .max_by_key(|decision| decision.created_at)
            .unwrap_or_else(|| archon_world_model::WorldGuardrailDecision::unavailable(action)),
    )
}

fn load_requirement(
    root: &std::path::Path,
    requirement_id: &str,
) -> Result<(
    archon_world_model::WorldGuardedAction,
    archon_world_model::VerificationRequirement,
)> {
    for action in archon_world_model::guardrail::load_guarded_actions(root)? {
        if let Some(requirement) = action
            .verification_plan
            .iter()
            .find(|requirement| requirement.requirement_id == requirement_id)
        {
            return Ok((action.clone(), requirement.clone()));
        }
    }
    bail!("verification requirement not found: {requirement_id}")
}

fn requirement_for_required_action(
    action: &archon_world_model::WorldGuardedAction,
    required: archon_world_model::GuardrailRequiredAction,
) -> archon_world_model::VerificationRequirement {
    let fallback = verification_requirement_for(&action.action_id, required);
    action
        .verification_plan
        .iter()
        .find(|requirement| requirement.kind == fallback.kind)
        .cloned()
        .unwrap_or(fallback)
}

fn append_manual_verification(
    root: &std::path::Path,
    action_id: &str,
    requirement: &archon_world_model::VerificationRequirement,
    status: archon_world_model::VerificationStatus,
    override_kind: &str,
    reason: &str,
) -> Result<archon_world_model::VerificationOutcome> {
    let verification = archon_world_model::VerificationOutcome {
        schema_version: archon_world_model::guardrail::CURRENT_SCHEMA_VERSION,
        requirement_id: requirement.requirement_id.clone(),
        action_id: action_id.to_string(),
        kind: requirement.kind.clone(),
        status,
        command: None,
        exit_code: None,
        summary: format!("manual {override_kind}: {reason}"),
        evidence_refs: vec![format!("manual_override:{override_kind}")],
        idempotency_key: format!(
            "world_guardrail:verification:{action_id}:manual:{override_kind}:{}",
            uuid::Uuid::new_v4()
        ),
        created_at: chrono::Utc::now(),
    };
    archon_world_model::guardrail::append_verification_outcome(root, &verification)?;
    Ok(verification)
}

fn parse_mode(value: &str) -> archon_world_model::WorldGuardrailMode {
    archon_world_model::WorldGuardrailMode::from_str(value)
        .unwrap_or(archon_world_model::WorldGuardrailMode::Advisory)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_from_config_maps_modes_and_overhead() {
        let mut config = archon_core::config::ArchonConfig::default();
        config.learning.world_model.guardrails.interactive_mode = "guarded".into();
        config
            .learning
            .world_model
            .guardrails
            .max_guardrail_overhead_ms = 41;

        let policy = policy_from_config(&config);

        assert_eq!(
            policy.interactive_mode,
            archon_world_model::WorldGuardrailMode::Guarded
        );
        assert_eq!(policy.max_guardrail_overhead_ms, 41);
    }

    #[test]
    fn status_reports_guardrail_counts() {
        let temp = tempfile::tempdir().unwrap();
        let action = archon_world_model::WorldGuardedAction::new(
            "s1",
            archon_world_model::integration::WorldAdvisorSurface::InteractiveSession,
            archon_world_model::GuardedActionKind::UserRequest,
            "goal",
            "summary",
        );
        archon_world_model::guardrail::append_guarded_action(temp.path(), &action).unwrap();

        let rendered =
            render_guard_status(&archon_core::config::ArchonConfig::default(), temp.path());

        assert!(rendered.contains("World Model Guardrails"));
        assert!(rendered.contains("Interactive mode:"));
        assert!(rendered.contains("Actions:                  1"));
    }

    #[test]
    fn status_reports_override_surprise_and_unavailable_breakdown() {
        let temp = tempfile::tempdir().unwrap();
        let mut action = archon_world_model::WorldGuardedAction::new(
            "s1",
            archon_world_model::integration::WorldAdvisorSurface::InteractiveSession,
            archon_world_model::GuardedActionKind::UserRequest,
            "goal",
            "summary",
        );
        action.action_id = "status-action".into();
        let decision = archon_world_model::WorldGuardrailDecision::unavailable(&action);
        let mut outcome = archon_world_model::WorldGuardrailOutcome::from_decision(
            &decision,
            archon_world_model::RuntimeTaskClass::CodingChange,
            archon_world_model::GuardrailFinalStatus::UserApprovedDespiteRisk,
            "approved after manual check",
        );
        outcome.latent_surprise = Some(0.55);
        archon_world_model::guardrail::append_guarded_action(temp.path(), &action).unwrap();
        archon_world_model::guardrail::append_guardrail_decision(temp.path(), &decision).unwrap();
        archon_world_model::guardrail::append_guardrail_outcome(temp.path(), &outcome).unwrap();

        let rendered =
            render_guard_status(&archon_core::config::ArchonConfig::default(), temp.path());

        assert!(rendered.contains("Unavailable reasons:      AdvisorUnavailable=1"));
        assert!(rendered.contains("User overrides:           1"));
        assert!(rendered.contains("High-surprise outcomes:   1"));
        assert!(rendered.contains("Latest actions:           status-action:InteractiveSession"));
    }

    #[test]
    fn guard_list_filters_by_derived_status() {
        let temp = tempfile::tempdir().unwrap();
        let mut blocked = archon_world_model::WorldGuardedAction::new(
            "s1",
            archon_world_model::integration::WorldAdvisorSurface::CodingTask,
            archon_world_model::GuardedActionKind::UserRequest,
            "blocked",
            "blocked",
        );
        blocked.action_id = "blocked-action".into();
        let blocked_decision = archon_world_model::WorldGuardrailDecision::unavailable(&blocked);
        let blocked_outcome = archon_world_model::WorldGuardrailOutcome::from_decision(
            &blocked_decision,
            archon_world_model::RuntimeTaskClass::CodingChange,
            archon_world_model::GuardrailFinalStatus::BlockedMissingVerification,
            "missing tests",
        );

        let mut complete = archon_world_model::WorldGuardedAction::new(
            "s1",
            archon_world_model::integration::WorldAdvisorSurface::CodingTask,
            archon_world_model::GuardedActionKind::UserRequest,
            "complete",
            "complete",
        );
        complete.action_id = "complete-action".into();
        let complete_decision = archon_world_model::WorldGuardrailDecision::unavailable(&complete);
        let complete_outcome = archon_world_model::WorldGuardrailOutcome::from_decision(
            &complete_decision,
            archon_world_model::RuntimeTaskClass::CodingChange,
            archon_world_model::GuardrailFinalStatus::CompletedVerified,
            "done",
        );

        let mut open = archon_world_model::WorldGuardedAction::new(
            "s1",
            archon_world_model::integration::WorldAdvisorSurface::CodingTask,
            archon_world_model::GuardedActionKind::UserRequest,
            "open",
            "open",
        );
        open.action_id = "open-action".into();
        let mut open_decision = archon_world_model::WorldGuardrailDecision::default();
        open_decision.action_id = open.action_id.clone();
        open_decision.mode = archon_world_model::WorldGuardrailMode::Guarded;
        open_decision.allowed_to_finalize = false;
        open_decision.required_actions =
            vec![archon_world_model::GuardrailRequiredAction::RunTests];

        for action in [&blocked, &complete, &open] {
            archon_world_model::guardrail::append_guarded_action(temp.path(), action).unwrap();
        }
        archon_world_model::guardrail::append_guardrail_decision(temp.path(), &blocked_decision)
            .unwrap();
        archon_world_model::guardrail::append_guardrail_decision(temp.path(), &complete_decision)
            .unwrap();
        archon_world_model::guardrail::append_guardrail_decision(temp.path(), &open_decision)
            .unwrap();
        archon_world_model::guardrail::append_guardrail_outcome(temp.path(), &blocked_outcome)
            .unwrap();
        archon_world_model::guardrail::append_guardrail_outcome(temp.path(), &complete_outcome)
            .unwrap();

        let blocked_list = render_guard_list(temp.path(), None, None, Some("blocked")).unwrap();
        let open_list = render_guard_list(temp.path(), None, None, Some("open")).unwrap();
        let complete_list = render_guard_list(temp.path(), None, None, Some("complete")).unwrap();

        assert!(blocked_list.contains("blocked-action [blocked]"));
        assert!(!blocked_list.contains("open-action"));
        assert!(open_list.contains("open-action [open]"));
        assert!(!open_list.contains("complete-action"));
        assert!(complete_list.contains("complete-action [complete]"));
        assert!(!complete_list.contains("blocked-action"));
    }

    #[test]
    fn blocked_guardrail_record_produces_forced_repair_prompt() {
        let action = archon_world_model::WorldGuardedAction::new(
            "s1",
            archon_world_model::integration::WorldAdvisorSurface::CodingTask,
            archon_world_model::GuardedActionKind::UserRequest,
            "build app",
            "build app",
        );
        let mut decision = archon_world_model::WorldGuardrailDecision::default();
        decision.action_id = action.action_id.clone();
        decision.mode = archon_world_model::WorldGuardrailMode::Guarded;
        decision.allowed_to_finalize = false;
        decision.required_actions = vec![archon_world_model::GuardrailRequiredAction::RunTests];
        let record = RuntimeGuardrailRecord {
            action,
            advisory: archon_world_model::integration::WorldAdvisorSurfaceRecord::unavailable(
                archon_world_model::integration::WorldAdvisorSurface::CodingTask,
                archon_world_model::WorldAdvisorUnavailableReason::ColdStart,
            ),
            decision,
            task_class: archon_world_model::RuntimeTaskClass::CodingChange,
        };

        let prompt = forced_repair_prompt(&record).expect("blocked record should force repair");

        assert!(prompt.contains("must not be marked complete"));
        assert!(prompt.contains("RunTests"));
    }

    #[test]
    fn classify_tool_command_detects_verification_commands() {
        let (kind, verification) = classify_tool_command("cargo test -p archon-core");
        assert_eq!(kind, archon_world_model::GuardedActionKind::TestCommand);
        assert_eq!(
            verification,
            Some(archon_world_model::VerificationKind::UnitTests)
        );

        let (kind, verification) = classify_tool_command("cargo check --bin archon");
        assert_eq!(kind, archon_world_model::GuardedActionKind::BuildCommand);
        assert_eq!(
            verification,
            Some(archon_world_model::VerificationKind::Build)
        );

        let (kind, verification) = classify_tool_command("python scripts/one_off.py");
        assert_eq!(kind, archon_world_model::GuardedActionKind::ShellCommand);
        assert_eq!(verification, None);
    }

    #[test]
    fn verification_plan_uses_stable_requirement_ids() {
        let mut decision = archon_world_model::WorldGuardrailDecision::default();
        decision.required_actions = vec![
            archon_world_model::GuardrailRequiredAction::RunTests,
            archon_world_model::GuardrailRequiredAction::RunBuild,
        ];

        let plan = verification_plan_for_decision("action-123", &decision);

        assert_eq!(plan.len(), 2);
        assert!(
            plan.iter()
                .any(|req| req.requirement_id == "world-guard-req-action-123-run-tests")
        );
        assert!(
            plan.iter()
                .any(|req| req.requirement_id == "world-guard-req-action-123-run-build")
        );
    }

    #[test]
    fn approve_records_manual_override_and_skipped_requirements() {
        let temp = tempfile::tempdir().unwrap();
        let mut action = archon_world_model::WorldGuardedAction::new(
            "s1",
            archon_world_model::integration::WorldAdvisorSurface::CodingTask,
            archon_world_model::GuardedActionKind::UserRequest,
            "build app",
            "build app",
        );
        action.action_id = "manual-approve-action".into();
        let mut decision = archon_world_model::WorldGuardrailDecision::default();
        decision.action_id = action.action_id.clone();
        decision.surface = action.surface;
        decision.mode = archon_world_model::WorldGuardrailMode::Guarded;
        decision.allowed_to_finalize = false;
        decision.required_actions = vec![
            archon_world_model::GuardrailRequiredAction::RunTests,
            archon_world_model::GuardrailRequiredAction::RunBuild,
        ];
        action.verification_plan = verification_plan_for_decision(&action.action_id, &decision);
        archon_world_model::guardrail::append_guarded_action(temp.path(), &action).unwrap();
        archon_world_model::guardrail::append_guardrail_decision(temp.path(), &decision).unwrap();

        let rendered =
            render_guard_approve(temp.path(), &action.action_id, "operator accepts risk").unwrap();
        let verifications =
            archon_world_model::guardrail::load_verification_outcomes(temp.path()).unwrap();
        let outcomes = archon_world_model::guardrail::load_guardrail_outcomes(temp.path()).unwrap();

        assert!(rendered.contains("user_approved_despite_risk"));
        assert_eq!(verifications.len(), 2);
        assert!(
            verifications.iter().all(|verification| verification.status
                == archon_world_model::VerificationStatus::Skipped)
        );
        assert!(archon_world_model::guardrail::finalization_allowed(
            &decision,
            &verifications
        ));
        assert_eq!(
            outcomes[0].final_status,
            archon_world_model::GuardrailFinalStatus::UserApprovedDespiteRisk
        );
        assert_eq!(outcomes[0].verification_outcomes.len(), 2);
    }

    #[test]
    fn skip_verification_records_skipped_outcome() {
        let temp = tempfile::tempdir().unwrap();
        let mut action = archon_world_model::WorldGuardedAction::new(
            "s1",
            archon_world_model::integration::WorldAdvisorSurface::CodingTask,
            archon_world_model::GuardedActionKind::UserRequest,
            "build app",
            "build app",
        );
        action.action_id = "manual-skip-action".into();
        let mut decision = archon_world_model::WorldGuardrailDecision::default();
        decision.action_id = action.action_id.clone();
        decision.required_actions = vec![archon_world_model::GuardrailRequiredAction::RunTests];
        action.verification_plan = verification_plan_for_decision(&action.action_id, &decision);
        let requirement_id = action.verification_plan[0].requirement_id.clone();
        archon_world_model::guardrail::append_guarded_action(temp.path(), &action).unwrap();

        let rendered =
            render_guard_skip_verification(temp.path(), &requirement_id, "test host unavailable")
                .unwrap();
        let verifications =
            archon_world_model::guardrail::load_verification_outcomes(temp.path()).unwrap();

        assert!(rendered.contains("Status:      skipped"));
        assert_eq!(verifications.len(), 1);
        assert_eq!(verifications[0].requirement_id, requirement_id);
        assert_eq!(
            verifications[0].status,
            archon_world_model::VerificationStatus::Skipped
        );
        assert!(verifications[0].summary.contains("test host unavailable"));
    }

    #[test]
    fn provider_incident_attaches_to_active_guardrail_observations() {
        let session_id = format!("s-{}", uuid::Uuid::new_v4());
        let mut action = archon_world_model::WorldGuardedAction::new(
            &session_id,
            archon_world_model::integration::WorldAdvisorSurface::CodingTask,
            archon_world_model::GuardedActionKind::UserRequest,
            "build app",
            "build app",
        );
        action.action_id = format!("a-{}", uuid::Uuid::new_v4());
        let mut decision = archon_world_model::WorldGuardrailDecision::default();
        decision.action_id = action.action_id.clone();
        let record = RuntimeGuardrailRecord {
            action,
            advisory: archon_world_model::integration::WorldAdvisorSurfaceRecord::unavailable(
                archon_world_model::integration::WorldAdvisorSurface::CodingTask,
                archon_world_model::WorldAdvisorUnavailableReason::ColdStart,
            ),
            decision,
            task_class: archon_world_model::RuntimeTaskClass::CodingChange,
        };
        remember_active_guardrail(&record);

        let attached = record_guardrail_provider_incident_for_session(
            &archon_core::config::ArchonConfig::default(),
            &session_id,
            "provider-event-1",
            "rate_limited",
        );
        let observations = observations_for(&record.action.action_id);

        assert!(attached);
        assert!(observations.provider_incident_observed);
        assert_eq!(observations.retry_count, 1);
        assert!(
            observations
                .evidence_refs
                .contains(&"provider_event:provider-event-1".to_string())
        );
        clear_active_guardrail(&session_id, &record.action.action_id);
    }

    #[test]
    fn reasoning_quality_event_attaches_to_active_guardrail_observations() {
        let session_id = format!("s-{}", uuid::Uuid::new_v4());
        let mut action = archon_world_model::WorldGuardedAction::new(
            &session_id,
            archon_world_model::integration::WorldAdvisorSurface::CodingTask,
            archon_world_model::GuardedActionKind::UserRequest,
            "build app",
            "build app",
        );
        action.action_id = format!("a-{}", uuid::Uuid::new_v4());
        let mut decision = archon_world_model::WorldGuardrailDecision::default();
        decision.action_id = action.action_id.clone();
        let record = RuntimeGuardrailRecord {
            action,
            advisory: archon_world_model::integration::WorldAdvisorSurfaceRecord::unavailable(
                archon_world_model::integration::WorldAdvisorSurface::CodingTask,
                archon_world_model::WorldAdvisorUnavailableReason::ColdStart,
            ),
            decision,
            task_class: archon_world_model::RuntimeTaskClass::CodingChange,
        };
        remember_active_guardrail(&record);
        let event = archon_reasoning_quality::ReasoningQualityEvent {
            event_id: "rqevt-1".into(),
            session_id: session_id.clone(),
            event_kind: archon_reasoning_quality::ReasoningEventKind::ClaimCorrectedByUser,
            ..archon_reasoning_quality::ReasoningQualityEvent::default()
        };

        let attached = record_guardrail_reasoning_quality_event(&event);
        let observations = observations_for(&record.action.action_id);

        assert!(attached);
        assert!(observations.user_correction_observed);
        assert!(observations.reasoning_failure_observed);
        assert!(
            observations
                .evidence_refs
                .contains(&"reasoning_quality:rqevt-1".to_string())
        );
        clear_active_guardrail(&session_id, &record.action.action_id);
    }
}
