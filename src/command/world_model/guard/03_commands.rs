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
            if matches!(
                *reason,
                archon_world_model::GuardrailReasonCode::AdvisorUnavailable
                    | archon_world_model::GuardrailReasonCode::GuardrailOverheadExceeded
            ) {
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
         max_guardrail_overhead_ms = {overhead_ms} # policy/decision overhead cap after prediction\n\
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

