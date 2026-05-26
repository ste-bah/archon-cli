use std::collections::BTreeMap;

use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::cozo_guard::run_script_guarded;
use crate::{Candidate, CognitiveError};

pub(crate) fn persist_candidates(
    db: &DbInstance,
    candidates: &[Candidate],
) -> Result<(), CognitiveError> {
    for candidate in candidates {
        persist_candidate(db, candidate)?;
    }
    Ok(())
}

fn persist_candidate(db: &DbInstance, candidate: &Candidate) -> Result<(), CognitiveError> {
    let mut params = BTreeMap::new();
    params.insert(
        "candidate_id".into(),
        DataValue::from(candidate.id.as_str()),
    );
    params.insert(
        "situation_id".into(),
        DataValue::from(candidate.situation_id.as_str()),
    );
    params.insert(
        "action_kind".into(),
        DataValue::from(candidate.action_kind.as_str()),
    );
    params.insert(
        "tool_name".into(),
        DataValue::from(candidate.tool_name.as_deref().unwrap_or("")),
    );
    params.insert(
        "risk".into(),
        DataValue::from(candidate.risk_class.as_str()),
    );
    params.insert(
        "expected_evidence".into(),
        DataValue::from(candidate.expected_evidence.as_str()),
    );
    params.insert(
        "expected_user_output".into(),
        DataValue::from(candidate.expected_user_output.as_str()),
    );
    params.insert(
        "score".into(),
        DataValue::from(candidate.heuristic_score as f64),
    );
    params.insert(
        "score_source".into(),
        DataValue::from(candidate.score_source.as_str()),
    );
    params.insert(
        "rollback_path".into(),
        DataValue::from(candidate.rollback_path.as_deref().unwrap_or("")),
    );
    params.insert("rejected_reason".into(), DataValue::from(""));
    params.insert(
        "created_at".into(),
        DataValue::from(candidate.created_at.to_rfc3339().as_str()),
    );
    run_script_guarded(
        db,
        "?[candidate_id, situation_id, action_kind, tool_name, risk, expected_evidence, expected_user_output, score, score_source, rollback_path, rejected_reason, created_at] <- \
         [[$candidate_id, $situation_id, $action_kind, $tool_name, $risk, $expected_evidence, $expected_user_output, $score, $score_source, $rollback_path, $rejected_reason, $created_at]]
         :put cognitive_action_candidates { candidate_id => situation_id, action_kind, tool_name, risk, expected_evidence, expected_user_output, score, score_source, rollback_path, rejected_reason, created_at }",
        params,
        ScriptMutability::Mutable,
        "put cognitive action candidate",
    )?;
    Ok(())
}
