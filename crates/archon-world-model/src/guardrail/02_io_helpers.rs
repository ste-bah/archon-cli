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

