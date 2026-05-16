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
        .find(|requirement| requirement_matches_verification(&requirement.kind, kind))
        .map(|requirement| requirement.requirement_id.clone())
        .unwrap_or_default()
}

fn requirement_matches_verification(
    requirement_kind: &archon_world_model::VerificationKind,
    actual_kind: &archon_world_model::VerificationKind,
) -> bool {
    if requirement_kind == actual_kind {
        return true;
    }
    match (requirement_kind, actual_kind) {
        (
            archon_world_model::VerificationKind::UnitTests,
            archon_world_model::VerificationKind::IntegrationTests,
        ) => true,
        (archon_world_model::VerificationKind::Custom(expected), actual)
            if expected == "verifier" =>
        {
            matches!(
                actual,
                archon_world_model::VerificationKind::UnitTests
                    | archon_world_model::VerificationKind::IntegrationTests
                    | archon_world_model::VerificationKind::Build
                    | archon_world_model::VerificationKind::Lint
                    | archon_world_model::VerificationKind::Typecheck
                    | archon_world_model::VerificationKind::FormatCheck
                    | archon_world_model::VerificationKind::StaticAnalysis
                    | archon_world_model::VerificationKind::SourceEvidenceCheck
                    | archon_world_model::VerificationKind::HumanApproval
            )
        }
        _ => false,
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
         Unavailable decisions:    {unavailable_count}\n\
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

