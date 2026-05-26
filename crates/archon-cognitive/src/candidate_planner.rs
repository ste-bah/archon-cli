use std::collections::BTreeSet;

use chrono::Utc;
use cozo::DbInstance;
use uuid::Uuid;

use crate::candidate_store::persist_candidates;
use crate::schema::ensure_cognitive_schema;
use crate::self_model::{MemoryContext, SelfModelProfile};
use crate::{
    Candidate, CandidateActionKind, CognitiveError, RiskLevel, ScoreSource, Situation,
    SituationKind,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeuristicWeights {
    pub risk_penalty: f32,
    pub evidence_bonus: f32,
    pub prior_success_bonus: f32,
    pub user_friction_penalty: f32,
}

impl Default for HeuristicWeights {
    fn default() -> Self {
        Self {
            risk_penalty: 0.30,
            evidence_bonus: 0.25,
            prior_success_bonus: 0.25,
            user_friction_penalty: 0.20,
        }
    }
}

pub struct CandidatePlanner<'a> {
    db: Option<&'a DbInstance>,
    max_candidates: usize,
    weights: HeuristicWeights,
}

impl<'a> CandidatePlanner<'a> {
    pub fn new(db: &'a DbInstance, max_candidates: usize) -> Result<Self, CognitiveError> {
        ensure_cognitive_schema(db)?;
        Ok(Self {
            db: Some(db),
            max_candidates: max_candidates.clamp(2, 5),
            weights: HeuristicWeights::default(),
        })
    }

    pub fn without_store(max_candidates: usize) -> Self {
        Self {
            db: None,
            max_candidates: max_candidates.clamp(2, 5),
            weights: HeuristicWeights::default(),
        }
    }

    pub fn generate(
        &self,
        situation: &Situation,
        self_model: &SelfModelProfile,
        memory_context: &MemoryContext,
    ) -> Result<Vec<Candidate>, CognitiveError> {
        let mut kinds = action_kinds_for(situation.kind);
        adjust_for_self_model(&mut kinds, situation, self_model);
        ensure_minimum_candidates(&mut kinds, situation.kind);

        let domain = domain_for(situation.kind);
        let trust = trust_for_domain(self_model, domain);
        let mut candidates = kinds
            .iter()
            .enumerate()
            .map(|(index, kind)| {
                let risk = adjusted_risk(*kind, memory_context, self_model);
                let mut candidate = candidate_from_kind(situation, *kind, risk);
                candidate.heuristic_score = score_candidate(&candidate, trust, self.weights);
                apply_situation_priority(situation.kind, &mut candidate);
                (index, candidate)
            })
            .collect::<Vec<_>>();

        candidates.sort_by(|left, right| compare_candidates(left, right));
        if trust < 0.3 {
            promote_low_trust_candidates(&mut candidates);
        }
        let mut ranked = candidates
            .into_iter()
            .take(self.max_candidates)
            .map(|(_, candidate)| candidate)
            .collect::<Vec<_>>();

        if !matches!(
            situation.kind,
            SituationKind::Greeting | SituationKind::SimpleQuestion
        ) {
            append_missing_fallbacks(&mut ranked, situation);
        }
        self.persist_candidates_lossy(&ranked);
        Ok(ranked)
    }

    fn persist_candidates_lossy(&self, candidates: &[Candidate]) {
        let Some(db) = self.db else {
            return;
        };
        if let Err(error) = persist_candidates(db, candidates) {
            tracing::warn!(%error, "cognitive candidate persistence skipped");
        }
    }
}

fn action_kinds_for(kind: SituationKind) -> Vec<CandidateActionKind> {
    use CandidateActionKind as A;
    match kind {
        SituationKind::Greeting | SituationKind::SimpleQuestion => vec![A::AnswerDirectly],
        SituationKind::Ambiguous => vec![A::AskClarification, A::AnswerDirectly],
        SituationKind::CodeChange => {
            vec![
                A::InspectFiles,
                A::RunTests,
                A::RunSafeShellProbe,
                A::AnswerDirectly,
            ]
        }
        SituationKind::GitMutation => {
            vec![
                A::InspectFiles,
                A::RunSafeShellProbe,
                A::AnswerDirectly,
                A::DeferOrDecline,
            ]
        }
        SituationKind::CiDebug => {
            vec![
                A::RunSafeShellProbe,
                A::InspectFiles,
                A::SearchDocs,
                A::AnswerDirectly,
            ]
        }
        SituationKind::Research => {
            vec![
                A::SearchDocs,
                A::RecallMemory,
                A::AnswerDirectly,
                A::AskClarification,
            ]
        }
        SituationKind::PipelineControl => {
            vec![
                A::InspectFiles,
                A::RunSafeShellProbe,
                A::RecallMemory,
                A::AnswerDirectly,
            ]
        }
        SituationKind::WorldModelTask => {
            vec![
                A::RunLearningTick,
                A::RunSafeShellProbe,
                A::InspectFiles,
                A::AnswerDirectly,
            ]
        }
        SituationKind::HighRisk => {
            vec![
                A::AskClarification,
                A::DeferOrDecline,
                A::InspectFiles,
                A::AnswerDirectly,
            ]
        }
    }
}

fn adjust_for_self_model(
    kinds: &mut Vec<CandidateActionKind>,
    situation: &Situation,
    self_model: &SelfModelProfile,
) {
    if trust_for_domain(self_model, domain_for(situation.kind)) >= 0.3 {
        return;
    }
    promote_unique(kinds, CandidateActionKind::DeferOrDecline);
    promote_unique(kinds, CandidateActionKind::AskClarification);
}

fn promote_unique(kinds: &mut Vec<CandidateActionKind>, kind: CandidateActionKind) {
    kinds.retain(|candidate| *candidate != kind);
    kinds.insert(0, kind);
}

fn ensure_minimum_candidates(kinds: &mut Vec<CandidateActionKind>, kind: SituationKind) {
    if matches!(
        kind,
        SituationKind::Greeting | SituationKind::SimpleQuestion
    ) {
        return;
    }
    let mut seen = BTreeSet::new();
    kinds.retain(|kind| seen.insert(kind.as_str()));
    for fallback in [
        CandidateActionKind::AnswerDirectly,
        CandidateActionKind::AskClarification,
    ] {
        if kinds.len() >= 2 {
            break;
        }
        if !kinds.contains(&fallback) {
            kinds.push(fallback);
        }
    }
}

fn append_missing_fallbacks(ranked: &mut Vec<Candidate>, situation: &Situation) {
    for kind in [
        CandidateActionKind::AnswerDirectly,
        CandidateActionKind::AskClarification,
    ] {
        if ranked.len() >= 2 {
            break;
        }
        if !ranked.iter().any(|candidate| candidate.action_kind == kind) {
            ranked.push(candidate_from_kind(situation, kind, RiskLevel::Low));
        }
    }
}

fn candidate_from_kind(
    situation: &Situation,
    action_kind: CandidateActionKind,
    risk_class: RiskLevel,
) -> Candidate {
    let template = template_for(action_kind);
    Candidate {
        id: Uuid::new_v4().to_string(),
        situation_id: situation.id.clone(),
        action_kind,
        tool_name: template.tool_name.map(str::to_string),
        expected_evidence: template.expected_evidence.to_string(),
        expected_user_output: template.expected_user_output.to_string(),
        risk_class,
        rollback_path: template.rollback_path.map(str::to_string),
        heuristic_score: 0.0,
        score_source: ScoreSource::Heuristic,
        created_at: Utc::now(),
    }
}

struct ActionTemplate {
    tool_name: Option<&'static str>,
    expected_evidence: &'static str,
    expected_user_output: &'static str,
    rollback_path: Option<&'static str>,
}

fn template_for(kind: CandidateActionKind) -> ActionTemplate {
    match kind {
        CandidateActionKind::AnswerDirectly => template(None, "current context", "concise answer"),
        CandidateActionKind::RecallMemory => template(
            Some("memory_recall"),
            "memory facts",
            "brief recall summary",
        ),
        CandidateActionKind::InspectFiles => template(
            Some("Read/Grep"),
            "file contents",
            "evidence-backed findings",
        ),
        CandidateActionKind::SearchDocs => template(
            Some("DocSearch"),
            "document citations",
            "cited source summary",
        ),
        CandidateActionKind::RunSafeShellProbe => template(
            Some("Bash"),
            "read-only command output",
            "compact command result",
        ),
        CandidateActionKind::AskClarification => {
            template(None, "user clarification", "one focused question")
        }
        CandidateActionKind::RunTests => {
            template(Some("Bash"), "test output", "pass/fail evidence")
        }
        CandidateActionKind::DeferOrDecline => template(
            None,
            "policy or uncertainty rationale",
            "safe refusal or deferral",
        ),
        CandidateActionKind::CreateGovernedProposal => {
            template(Some("memory_store"), "proposal evidence", "proposal id")
        }
        CandidateActionKind::RunLearningTick => {
            template(Some("cognitive_tick"), "learning status", "tick summary")
        }
    }
}

fn template(
    tool_name: Option<&'static str>,
    expected_evidence: &'static str,
    expected_user_output: &'static str,
) -> ActionTemplate {
    ActionTemplate {
        tool_name,
        expected_evidence,
        expected_user_output,
        rollback_path: None,
    }
}

fn adjusted_risk(
    kind: CandidateActionKind,
    memory_context: &MemoryContext,
    self_model: &SelfModelProfile,
) -> RiskLevel {
    let mut risk = base_risk(kind);
    let action = kind.as_str();
    let has_pattern = memory_context
        .failure_pattern_labels
        .iter()
        .chain(self_model.caution_rules.iter())
        .any(|value| value.contains(action));
    if has_pattern {
        risk = elevate(risk);
    }
    risk
}

fn base_risk(kind: CandidateActionKind) -> RiskLevel {
    match kind {
        CandidateActionKind::RunTests
        | CandidateActionKind::RunSafeShellProbe
        | CandidateActionKind::CreateGovernedProposal => RiskLevel::Medium,
        _ => RiskLevel::Low,
    }
}

fn elevate(risk: RiskLevel) -> RiskLevel {
    match risk {
        RiskLevel::Low => RiskLevel::Medium,
        RiskLevel::Medium => RiskLevel::High,
        RiskLevel::High | RiskLevel::Critical => RiskLevel::Critical,
    }
}

fn score_candidate(candidate: &Candidate, domain_trust: f32, weights: HeuristicWeights) -> f32 {
    let mut score = 0.5 + risk_delta(candidate.risk_class) * weights.risk_penalty;
    if !candidate.expected_evidence.trim().is_empty() {
        score += weights.evidence_bonus * 0.5;
    }
    if has_verifiable_evidence(&candidate.expected_evidence) {
        score += weights.evidence_bonus * 0.5;
    }
    score += domain_trust.clamp(0.0, 1.0) * weights.prior_success_bonus;
    score -= friction(candidate.action_kind) * weights.user_friction_penalty;
    score.clamp(0.0, 1.0)
}

fn apply_situation_priority(kind: SituationKind, candidate: &mut Candidate) {
    if !matches!(kind, SituationKind::CiDebug) {
        return;
    }
    let delta = match candidate.action_kind {
        CandidateActionKind::RunSafeShellProbe => 0.25,
        CandidateActionKind::InspectFiles => 0.05,
        CandidateActionKind::SearchDocs => -0.20,
        CandidateActionKind::AnswerDirectly => -0.30,
        _ => 0.0,
    };
    candidate.heuristic_score = (candidate.heuristic_score + delta).clamp(0.0, 1.0);
}

fn risk_delta(risk: RiskLevel) -> f32 {
    match risk {
        RiskLevel::Low => 0.0,
        RiskLevel::Medium => -0.15,
        RiskLevel::High => -0.30,
        RiskLevel::Critical => -0.50,
    }
}

fn has_verifiable_evidence(value: &str) -> bool {
    ["file", "document", "command", "test", "memory", "learning"]
        .iter()
        .any(|needle| value.contains(needle))
}

fn compare_candidates(left: &(usize, Candidate), right: &(usize, Candidate)) -> std::cmp::Ordering {
    right
        .1
        .heuristic_score
        .total_cmp(&left.1.heuristic_score)
        .then_with(|| left.1.risk_class.cmp(&right.1.risk_class))
        .then_with(|| friction(left.1.action_kind).total_cmp(&friction(right.1.action_kind)))
        .then_with(|| left.0.cmp(&right.0))
}

fn promote_low_trust_candidates(candidates: &mut Vec<(usize, Candidate)>) {
    promote_ranked(candidates, CandidateActionKind::DeferOrDecline);
    promote_ranked(candidates, CandidateActionKind::AskClarification);
}

fn promote_ranked(candidates: &mut Vec<(usize, Candidate)>, kind: CandidateActionKind) {
    if let Some(index) = candidates
        .iter()
        .position(|(_, candidate)| candidate.action_kind == kind)
    {
        let candidate = candidates.remove(index);
        candidates.insert(0, candidate);
    }
}

fn friction(kind: CandidateActionKind) -> f32 {
    match kind {
        CandidateActionKind::AnswerDirectly => 0.0,
        CandidateActionKind::AskClarification => 0.05,
        CandidateActionKind::RecallMemory => 0.1,
        CandidateActionKind::SearchDocs => 0.2,
        CandidateActionKind::InspectFiles => 0.3,
        CandidateActionKind::RunSafeShellProbe => 0.45,
        CandidateActionKind::RunTests => 0.55,
        CandidateActionKind::RunLearningTick => 0.65,
        CandidateActionKind::CreateGovernedProposal => 0.75,
        CandidateActionKind::DeferOrDecline => 0.8,
    }
}

fn trust_for_domain(self_model: &SelfModelProfile, domain: &str) -> f32 {
    self_model
        .domain_trust
        .iter()
        .find(|trust| trust.domain == domain)
        .map(|trust| trust.trust_score)
        .unwrap_or(0.5)
}

fn domain_for(kind: SituationKind) -> &'static str {
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
