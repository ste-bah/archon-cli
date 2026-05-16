//! `archon world guard` command rendering and policy helpers.

use std::collections::HashMap;
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

fn active_guardrails() -> &'static Mutex<HashMap<String, RuntimeGuardrailRecord>> {
    ACTIVE_GUARDRAILS.get_or_init(|| Mutex::new(HashMap::new()))
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
    }
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
    let _ = archon_world_model::guardrail::append_guarded_action(&root, &action);

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
    let final_status = if !completed {
        archon_world_model::GuardrailFinalStatus::Failed
    } else if archon_world_model::guardrail::finalization_allowed(
        &record.decision,
        &verification_outcomes,
    ) {
        if record.decision.required_actions.is_empty() {
            archon_world_model::GuardrailFinalStatus::CompletedWithCaveat
        } else {
            archon_world_model::GuardrailFinalStatus::CompletedVerified
        }
    } else if verification_outcomes
        .iter()
        .any(|outcome| outcome.status == archon_world_model::VerificationStatus::Failed)
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
            requirement_id: String::new(),
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
         Advisor unavailable:      {unavailable}",
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
        unavailable = counts.advisor_unavailable_decisions,
    )
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
        lines.push(format!(
            "{} {} {:?} {:?} {}",
            action.created_at.to_rfc3339(),
            action.action_id,
            action.surface,
            action.action_kind,
            action.action_summary
        ));
    }
    Ok(lines.join("\n"))
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
}
