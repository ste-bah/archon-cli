use chrono::Utc;
use cozo::DbInstance;

use crate::executive_loop::{
    ActionExecutor, ActionOutcome, ExecutiveLoop, ExecutiveRunOutcome, ExecutiveTurnInput,
};
use crate::self_model::{
    ConfidenceCalibration, DomainTrust, MemoryContext, SelfModelProfile, SelfModelStore,
};
use crate::{
    Candidate, CandidateActionKind, CognitiveError, CognitiveStore, DecisionRecord, DecisionStore,
    ExecutiveStateSnapshot, LessonSink, RiskLevel, Situation, SituationClassifier, SituationKind,
    VerificationContract, VerificationEngine, VerificationKind, VerificationVerdict,
};

pub(crate) struct SnapshotParams<'a> {
    pub situation: &'a Situation,
    pub stage: &'a str,
    pub selected: Option<&'a Candidate>,
    pub policy_summary: String,
    pub verification_summary: String,
    pub prediction_available: bool,
    pub reflection_id: Option<String>,
    pub degraded: Vec<String>,
}

pub(crate) fn store_situation(db: &DbInstance, situation: &Situation, degraded: &mut Vec<String>) {
    if let Err(error) = CognitiveStore::new(db).and_then(|store| store.put_situation(situation)) {
        degraded.push(format!("situation_store_failed:{error}"));
    }
}

pub(crate) fn load_store_context(
    store: &SelfModelStore<'_>,
    domain: &str,
    degraded: &mut Vec<String>,
) -> (SelfModelProfile, MemoryContext) {
    let profile = store
        .read_profile(&[domain.to_string()])
        .unwrap_or_else(|error| {
            degraded.push(format!("self_model_profile_unavailable:{error}"));
            neutral_profile_for_domain(domain)
        });
    let memory = store.read_memory_context(domain).unwrap_or_else(|error| {
        degraded.push(format!("memory_context_unavailable:{error}"));
        MemoryContext::default()
    });
    (profile, memory)
}

pub(crate) fn select_candidate(
    mut allowed: Vec<Candidate>,
    situation: &Situation,
) -> Option<Candidate> {
    allowed.sort_by(|left, right| right.heuristic_score.total_cmp(&left.heuristic_score));
    allowed
        .into_iter()
        .next()
        .or_else(|| fallback_candidate(situation))
}

pub(crate) fn build_decision(
    situation: &Situation,
    selected: &Candidate,
    allowed_candidates: &[Candidate],
    all_candidates: &[Candidate],
    denied: &[crate::DenyReason],
    verdict: crate::PolicyVerdict,
) -> Result<DecisionRecord, CognitiveError> {
    let mut candidates = vec![selected.clone()];
    candidates.extend(
        allowed_candidates
            .iter()
            .filter(|item| item.id != selected.id)
            .cloned(),
    );
    let mut decision = DecisionStore::decision_from_candidates(situation, &candidates)?;
    decision.selected_candidate_id = selected.id.clone();
    decision.policy_verdict = Some(serde_json::to_string(&verdict)?);
    decision
        .rejected_alternatives
        .extend(denied.iter().map(|deny| crate::RejectedCandidate {
            candidate_id: deny.candidate_id.clone(),
            action_kind: candidate_kind(all_candidates, &deny.candidate_id),
            rejection_reason: format!("policy:{}:{}", deny.rule_name, deny.reason),
        }));
    Ok(decision)
}

pub(crate) fn record_decision<B, E, S>(
    loop_: &ExecutiveLoop<'_, B, E, S>,
    decision: &DecisionRecord,
    degraded: &mut Vec<String>,
) where
    B: crate::PredictionBackend,
    E: ActionExecutor,
    S: LessonSink,
{
    if !loop_.config.record_decisions {
        return;
    }
    let ledger = loop_.ledger_dir.join("cognitive-decisions.jsonl");
    if let Err(error) =
        DecisionStore::new(loop_.db, ledger).and_then(|store| store.record(decision))
    {
        degraded.push(format!("decision_store_failed:{error}"));
    }
}

pub(crate) fn verify_action(
    verifier: &VerificationEngine,
    contract: Option<&VerificationContract>,
    action: &ActionOutcome,
) -> VerificationVerdict {
    contract
        .map(|contract| verifier.verify_evidence(contract, &action.evidence))
        .unwrap_or(VerificationVerdict::NotRun)
}

pub(crate) fn contract_json(
    contract: &Option<VerificationContract>,
) -> Result<Option<String>, CognitiveError> {
    contract
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(CognitiveError::from)
}

pub(crate) fn verification_kind(
    kind: SituationKind,
    candidate: &Candidate,
) -> Option<VerificationKind> {
    if candidate.risk_class < RiskLevel::Medium {
        return None;
    }
    Some(match kind {
        SituationKind::CiDebug => VerificationKind::CiDebug,
        SituationKind::GitMutation => VerificationKind::Commit,
        SituationKind::WorldModelTask => VerificationKind::WorldModelPromotion,
        SituationKind::PipelineControl => VerificationKind::PipelineForceContinue,
        _ => VerificationKind::CompletionClaim,
    })
}

pub(crate) fn reflection_outcome(
    outcome: crate::OutcomeSummary,
    verification: &VerificationVerdict,
) -> crate::OutcomeSummary {
    match verification {
        VerificationVerdict::Failed { .. } => crate::OutcomeSummary::Failure,
        VerificationVerdict::Skipped { .. } | VerificationVerdict::Inconclusive => {
            crate::OutcomeSummary::Degraded
        }
        _ => outcome,
    }
}

pub(crate) fn verification_summary(verdict: &VerificationVerdict) -> String {
    match verdict {
        VerificationVerdict::Passed => "passed".into(),
        VerificationVerdict::Failed { reason } => format!("failed:{reason}"),
        VerificationVerdict::Skipped { reason } => format!("skipped:{reason}"),
        VerificationVerdict::NotRun => "not_run".into(),
        VerificationVerdict::Inconclusive => "inconclusive".into(),
    }
}

pub(crate) fn direct_outcome(
    situation: &Situation,
    stage: &str,
    degraded: Vec<String>,
) -> ExecutiveRunOutcome {
    ExecutiveRunOutcome {
        snapshot: snapshot(SnapshotParams {
            situation,
            stage,
            selected: None,
            policy_summary: "not_required".into(),
            verification_summary: "not_run".into(),
            prediction_available: false,
            reflection_id: None,
            degraded,
        }),
        decision: None,
        action_message: crate::direct_response_for(situation.kind)
            .unwrap_or("")
            .into(),
        verification: VerificationVerdict::NotRun,
    }
}

pub(crate) fn disabled_outcome(input: ExecutiveTurnInput) -> ExecutiveRunOutcome {
    let situation = SituationClassifier.classify(crate::ClassifyInput {
        user_text: &input.user_text,
        session_id: &input.session_id,
        turn_number: input.turn_number,
        surface: input.surface,
    });
    direct_outcome(
        &situation,
        "disabled",
        vec!["cognitive_loop_disabled".into()],
    )
}

pub(crate) fn snapshot(params: SnapshotParams<'_>) -> ExecutiveStateSnapshot {
    ExecutiveStateSnapshot {
        session_id: params.situation.session_id.clone(),
        turn_number: params.situation.turn_number,
        stage: params.stage.into(),
        situation_id: params.situation.id.clone(),
        user_text_hash: params.situation.user_text_hash.clone(),
        situation_kind: params.situation.kind,
        selected_candidate_id: params.selected.map(|candidate| candidate.id.clone()),
        selected_action: params.selected.map(|candidate| candidate.action_kind),
        policy_summary: params.policy_summary,
        verification_summary: params.verification_summary,
        prediction_available: params.prediction_available,
        reflection_id: params.reflection_id,
        degraded: params.degraded,
        created_at: Utc::now(),
    }
}

pub(crate) fn neutral_profile(kind: SituationKind) -> SelfModelProfile {
    neutral_profile_for_domain(domain_for(kind))
}

pub(crate) fn domain_for(kind: SituationKind) -> &'static str {
    match kind {
        SituationKind::CiDebug => "ci",
        SituationKind::CodeChange => "coding",
        SituationKind::GitMutation => "git",
        SituationKind::PipelineControl => "pipeline",
        SituationKind::Research => "research",
        SituationKind::WorldModelTask => "world_model",
        SituationKind::HighRisk => "safety",
        SituationKind::Greeting | SituationKind::SimpleQuestion | SituationKind::Ambiguous => {
            "general"
        }
    }
}

fn fallback_candidate(situation: &Situation) -> Option<Candidate> {
    Some(Candidate {
        id: format!("fallback-{}", situation.id),
        situation_id: situation.id.clone(),
        action_kind: CandidateActionKind::AnswerDirectly,
        tool_name: None,
        expected_evidence: "policy fallback".into(),
        expected_user_output: "safe direct answer".into(),
        risk_class: RiskLevel::Low,
        rollback_path: None,
        heuristic_score: 0.1,
        score_source: crate::ScoreSource::Heuristic,
        created_at: Utc::now(),
    })
}

fn candidate_kind(candidates: &[Candidate], id: &str) -> CandidateActionKind {
    candidates
        .iter()
        .find(|candidate| candidate.id == id)
        .map(|candidate| candidate.action_kind)
        .unwrap_or(CandidateActionKind::DeferOrDecline)
}

fn neutral_profile_for_domain(domain: &str) -> SelfModelProfile {
    SelfModelProfile {
        domain_trust: vec![DomainTrust {
            domain: domain.into(),
            trust_score: 0.5,
            evidence_count: 0,
            last_correction_at: None,
            failure_cluster_ids: Vec::new(),
        }],
        active_failure_clusters: Vec::new(),
        confidence_calibration: ConfidenceCalibration::default(),
        caution_rules: Vec::new(),
        generated_at: Utc::now(),
    }
}
