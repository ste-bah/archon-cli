use chrono::{DateTime, Utc};
use cozo::{DataValue, NamedRows};
use uuid::Uuid;

use crate::{
    Candidate, CandidateScore, CognitiveError, DecisionRecord, RejectedCandidate, Situation,
};

pub(crate) fn decision_from_candidates(
    situation: &Situation,
    candidates: &[Candidate],
) -> Result<DecisionRecord, CognitiveError> {
    let selected = candidates
        .iter()
        .max_by(|left, right| left.heuristic_score.total_cmp(&right.heuristic_score))
        .ok_or_else(|| CognitiveError::Store("cannot decide without candidates".into()))?;
    let rejected_alternatives = candidates
        .iter()
        .filter(|candidate| candidate.id != selected.id)
        .map(|candidate| rejected_candidate(candidate, selected))
        .collect();
    Ok(DecisionRecord {
        decision_id: Uuid::new_v4().to_string(),
        situation_id: situation.id.clone(),
        session_id: situation.session_id.clone(),
        turn_number: situation.turn_number,
        selected_candidate_id: selected.id.clone(),
        rejected_alternatives,
        heuristic_scores: candidates.iter().map(candidate_score).collect(),
        policy_verdict: None,
        verification_contract: None,
        user_visible_summary: summary_for(situation, selected),
        created_at: Utc::now(),
    })
}

pub(crate) fn query_script(filter: &str) -> String {
    format!(
        "?[decision_id, situation_id, session_id, turn_number, selected_candidate_id, rejected_candidates_json, heuristic_scores_json, policy_verdict_json, verification_contract_json, user_visible_summary, created_at] := \
         *cognitive_decisions{{decision_id, situation_id, session_id, turn_number, selected_candidate_id, rejected_candidates_json, heuristic_scores_json, policy_verdict_json, verification_contract_json, user_visible_summary, created_at}}, {filter}"
    )
}

pub(crate) fn rows_to_decisions(rows: NamedRows) -> Result<Vec<DecisionRecord>, CognitiveError> {
    rows.rows.iter().map(row_to_decision).collect()
}

pub(crate) fn row_to_decision(row: &Vec<DataValue>) -> Result<DecisionRecord, CognitiveError> {
    Ok(DecisionRecord {
        decision_id: str_col(row, 0),
        situation_id: str_col(row, 1),
        session_id: str_col(row, 2),
        turn_number: row[3].get_int().unwrap_or(0).max(0) as u64,
        selected_candidate_id: str_col(row, 4),
        rejected_alternatives: serde_json::from_str(&str_col(row, 5))?,
        heuristic_scores: serde_json::from_str(&str_col(row, 6))?,
        policy_verdict: non_empty(str_col(row, 7)),
        verification_contract: non_empty(str_col(row, 8)),
        user_visible_summary: str_col(row, 9),
        created_at: parse_time(&str_col(row, 10)),
    })
}

fn rejected_candidate(candidate: &Candidate, selected: &Candidate) -> RejectedCandidate {
    RejectedCandidate {
        candidate_id: candidate.id.clone(),
        action_kind: candidate.action_kind,
        rejection_reason: format!(
            "score {:.2} below selected {:.2}",
            candidate.heuristic_score, selected.heuristic_score
        ),
    }
}

fn candidate_score(candidate: &Candidate) -> CandidateScore {
    CandidateScore {
        candidate_id: candidate.id.clone(),
        score: candidate.heuristic_score,
        score_source: candidate.score_source,
        rationale: format!(
            "{} risk with {} evidence",
            candidate.risk_class.as_str(),
            candidate.expected_evidence
        ),
    }
}

fn summary_for(situation: &Situation, selected: &Candidate) -> String {
    format!(
        "{} -> {} (risk={}, score={:.2}, evidence={})",
        situation.kind.as_str(),
        selected.action_kind.as_str(),
        selected.risk_class.as_str(),
        selected.heuristic_score,
        selected.expected_evidence
    )
}

fn str_col(row: &[DataValue], index: usize) -> String {
    row[index].get_str().unwrap_or("").to_string()
}

fn non_empty(value: String) -> Option<String> {
    if value.is_empty() { None } else { Some(value) }
}

fn parse_time(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}
