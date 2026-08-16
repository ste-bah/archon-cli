use archon_completion::{
    RequiredEvidence, RequiredEvidenceKind, RequiredEvidenceStatus, check_required_evidence,
};

pub(crate) fn turn_requirements_for_action(session_id: &str, action_id: &str) -> Option<String> {
    if let Some(failure) = reclassification_failure(action_id) {
        return Some(format!(
            "Action {action_id} cannot finalize because {failure}"
        ));
    }
    let record = active_guardrail_for_action(session_id, action_id)?;
    if record.decision.required_actions.is_empty() {
        return None;
    }
    Some(format!(
        "Action {} cannot finalize until these guardrail requirements pass: {:?}. Run the required verification and do not claim completion before it passes.",
        record.action.action_id, record.decision.required_actions
    ))
}

pub(crate) fn turn_finalization_verdict_for_action(
    session_id: &str,
    action_id: &str,
) -> archon_core::agent::TurnFinalizationVerdict {
    if action_id.is_empty() {
        return archon_core::agent::TurnFinalizationVerdict::Allowed;
    }
    let root = match super::world_model_root() {
        Ok(root) => root,
        Err(error) => {
            return blocked_verdict(format!(
                "Guardrail verification storage is unavailable: {error}"
            ));
        }
    };
    turn_finalization_verdict_at_root(&root, session_id, action_id)
}

fn turn_finalization_verdict_at_root(
    root: &std::path::Path,
    session_id: &str,
    action_id: &str,
) -> archon_core::agent::TurnFinalizationVerdict {
    if action_id.is_empty() {
        return archon_core::agent::TurnFinalizationVerdict::Allowed;
    }
    if let Some(failure) = reclassification_failure(action_id) {
        return blocked_verdict(format!(
            "Action {action_id} cannot finalize because {failure}"
        ));
    }
    let Some(record) = active_guardrail_for_action(session_id, action_id) else {
        return blocked_verdict(format!(
            "Guardrail action {action_id} is not available for finalization."
        ));
    };
    if record.decision.allowed_to_finalize || !record.decision.mode.can_block() {
        return archon_core::agent::TurnFinalizationVerdict::Allowed;
    }

    let outcomes = match archon_world_model::guardrail::load_verification_outcomes(root) {
        Ok(outcomes) => {
            let mut outcomes = outcomes
                .into_iter()
                .filter(|outcome| outcome.action_id == action_id)
                .collect::<Vec<_>>();
            outcomes.sort_by(|left, right| {
                left.created_at
                    .cmp(&right.created_at)
                    .then_with(|| left.idempotency_key.cmp(&right.idempotency_key))
            });
            outcomes
        }
        Err(error) => {
            return blocked_verdict(format!(
                "Guardrail verification evidence could not be loaded: {error}"
            ));
        }
    };
    let required = record
        .decision
        .required_actions
        .iter()
        .copied()
        .map(required_evidence_kind)
        .collect::<Vec<_>>();
    let evidence = completion_evidence(&outcomes);
    let check = check_required_evidence(&required, &evidence);
    if check.allowed {
        archon_core::agent::TurnFinalizationVerdict::Allowed
    } else {
        blocked_verdict(format!(
            "Action {action_id} cannot finalize. Missing verification: {:?}; failed verification: {:?}. Run the required checks before finalizing.",
            check.missing, check.failed
        ))
    }
}

fn required_evidence_kind(
    required: archon_world_model::GuardrailRequiredAction,
) -> RequiredEvidenceKind {
    match required {
        archon_world_model::GuardrailRequiredAction::RunTests => RequiredEvidenceKind::Tests,
        archon_world_model::GuardrailRequiredAction::RunBuild => RequiredEvidenceKind::Build,
        archon_world_model::GuardrailRequiredAction::RunLint => RequiredEvidenceKind::Lint,
        archon_world_model::GuardrailRequiredAction::RunTypecheck => {
            RequiredEvidenceKind::Typecheck
        }
        archon_world_model::GuardrailRequiredAction::RunVerifier => RequiredEvidenceKind::Verifier,
        archon_world_model::GuardrailRequiredAction::ReviewPlanAgainstUserGoal => {
            RequiredEvidenceKind::PlanReview
        }
        archon_world_model::GuardrailRequiredAction::CheckSourceEvidence => {
            RequiredEvidenceKind::SourceEvidence
        }
        archon_world_model::GuardrailRequiredAction::RecordManualOutcome => {
            RequiredEvidenceKind::ManualOutcome
        }
        archon_world_model::GuardrailRequiredAction::RequireUserApproval => {
            RequiredEvidenceKind::HumanApproval
        }
    }
}

fn completion_evidence(
    outcomes: &[archon_world_model::VerificationOutcome],
) -> Vec<RequiredEvidence> {
    outcomes
        .iter()
        .enumerate()
        .flat_map(|(sequence, outcome)| {
            evidence_kinds_for_verification(&outcome.kind)
                .iter()
                .copied()
                .map(move |kind| RequiredEvidence {
                    kind,
                    status: completion_evidence_status(outcome),
                    sequence: sequence as u64,
                    evidence_id: None,
                    run_id: None,
                })
        })
        .collect()
}

fn evidence_kinds_for_verification(
    kind: &archon_world_model::VerificationKind,
) -> &'static [RequiredEvidenceKind] {
    match kind {
        archon_world_model::VerificationKind::Build => {
            &[RequiredEvidenceKind::Build, RequiredEvidenceKind::Verifier]
        }
        archon_world_model::VerificationKind::UnitTests
        | archon_world_model::VerificationKind::IntegrationTests => {
            &[RequiredEvidenceKind::Tests, RequiredEvidenceKind::Verifier]
        }
        archon_world_model::VerificationKind::Lint => {
            &[RequiredEvidenceKind::Lint, RequiredEvidenceKind::Verifier]
        }
        archon_world_model::VerificationKind::Typecheck => &[
            RequiredEvidenceKind::Typecheck,
            RequiredEvidenceKind::Verifier,
        ],
        archon_world_model::VerificationKind::SourceEvidenceCheck => &[
            RequiredEvidenceKind::SourceEvidence,
            RequiredEvidenceKind::Verifier,
        ],
        archon_world_model::VerificationKind::HumanApproval => &[
            RequiredEvidenceKind::HumanApproval,
            RequiredEvidenceKind::Verifier,
        ],
        archon_world_model::VerificationKind::FormatCheck
        | archon_world_model::VerificationKind::StaticAnalysis => &[RequiredEvidenceKind::Verifier],
        archon_world_model::VerificationKind::Custom(value) if value == "plan_review" => {
            &[RequiredEvidenceKind::PlanReview]
        }
        archon_world_model::VerificationKind::Custom(value) if value == "manual_outcome" => {
            &[RequiredEvidenceKind::ManualOutcome]
        }
        archon_world_model::VerificationKind::Custom(value) if value == "verifier" => {
            &[RequiredEvidenceKind::Verifier]
        }
        archon_world_model::VerificationKind::Custom(_) => &[],
    }
}

fn completion_evidence_status(
    outcome: &archon_world_model::VerificationOutcome,
) -> RequiredEvidenceStatus {
    match outcome.status {
        archon_world_model::VerificationStatus::Passed => RequiredEvidenceStatus::Passed,
        archon_world_model::VerificationStatus::Failed => RequiredEvidenceStatus::Failed,
        archon_world_model::VerificationStatus::Skipped
            if outcome
                .evidence_refs
                .iter()
                .any(|reference| reference.starts_with("manual_override:")) =>
        {
            RequiredEvidenceStatus::Passed
        }
        archon_world_model::VerificationStatus::Skipped
        | archon_world_model::VerificationStatus::NotRun
        | archon_world_model::VerificationStatus::Inconclusive => RequiredEvidenceStatus::Missing,
    }
}

fn blocked_verdict(repair_prompt: String) -> archon_core::agent::TurnFinalizationVerdict {
    archon_core::agent::TurnFinalizationVerdict::Blocked { repair_prompt }
}
