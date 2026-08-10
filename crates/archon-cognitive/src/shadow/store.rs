//! Persistence for shadow observations.
//!
//! The join key is `(session_id, turn_number)` rather than in-memory state, so
//! a shadow plan survives a restart between the turn starting and finishing,
//! and the observer holds nothing the live path could be blocked on.

use std::collections::BTreeMap;

use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::cozo_guard::run_script_guarded;
use crate::shadow::types::{LiveTurnOutcome, ShadowObservation};
use crate::{CandidateActionKind, CognitiveError, SituationKind};

const COLUMNS: &str = "shadow_decision_id, session_id, turn_number, decision_id, situation_id, situation_kind, selected_action, candidate_id, candidate_rank, degraded_json, joined, live_action, live_outcome_status, agreed, surprise, created_at, joined_at";

const KEYED: &str = "shadow_decision_id => session_id, turn_number, decision_id, situation_id, situation_kind, selected_action, candidate_id, candidate_rank, degraded_json, joined, live_action, live_outcome_status, agreed, surprise, created_at, joined_at";

pub(crate) fn put_pending(
    db: &DbInstance,
    observation: &ShadowObservation,
) -> Result<(), CognitiveError> {
    let mut params = BTreeMap::new();
    params.insert(
        "shadow_decision_id".into(),
        DataValue::from(observation.shadow_decision_id.as_str()),
    );
    params.insert(
        "session_id".into(),
        DataValue::from(observation.session_id.as_str()),
    );
    params.insert(
        "turn_number".into(),
        DataValue::from(observation.turn_number as i64),
    );
    params.insert(
        "decision_id".into(),
        DataValue::from(observation.decision_id.as_str()),
    );
    params.insert(
        "situation_id".into(),
        DataValue::from(observation.situation_id.as_str()),
    );
    params.insert(
        "situation_kind".into(),
        DataValue::from(observation.situation_kind.as_str()),
    );
    params.insert(
        "selected_action".into(),
        DataValue::from(
            observation
                .selected_action
                .map(CandidateActionKind::as_str)
                .unwrap_or(""),
        ),
    );
    params.insert(
        "candidate_id".into(),
        DataValue::from(observation.candidate_id.as_str()),
    );
    params.insert(
        "candidate_rank".into(),
        DataValue::from(observation.candidate_rank as i64),
    );
    params.insert(
        "degraded_json".into(),
        DataValue::from(serde_json::to_string(&observation.degraded)?.as_str()),
    );
    params.insert("joined".into(), DataValue::from(false));
    params.insert("live_action".into(), DataValue::from(""));
    params.insert("live_outcome_status".into(), DataValue::from(""));
    params.insert("agreed".into(), DataValue::Null);
    params.insert("surprise".into(), DataValue::Null);
    params.insert(
        "created_at".into(),
        DataValue::from(observation.created_at.to_rfc3339().as_str()),
    );
    params.insert("joined_at".into(), DataValue::from(""));

    run_script_guarded(
        db,
        &format!(
            "?[{COLUMNS}] <- [[$shadow_decision_id, $session_id, $turn_number, $decision_id, $situation_id, $situation_kind, $selected_action, $candidate_id, $candidate_rank, $degraded_json, $joined, $live_action, $live_outcome_status, $agreed, $surprise, $created_at, $joined_at]]
             :put cognitive_shadow_decisions {{ {KEYED} }}"
        ),
        params,
        ScriptMutability::Mutable,
        "put pending shadow decision",
    )?;
    Ok(())
}

/// Newest unjoined observation for a turn, or `None` when the turn had no
/// shadow plan (disabled, trivial, or policy-blocked).
pub(crate) fn take_pending(
    db: &DbInstance,
    session_id: &str,
    turn_number: u64,
) -> Result<Option<ShadowObservation>, CognitiveError> {
    let mut params = BTreeMap::new();
    params.insert("session_id".into(), DataValue::from(session_id));
    params.insert("turn_number".into(), DataValue::from(turn_number as i64));
    let rows = run_script_guarded(
        db,
        "?[shadow_decision_id, decision_id, situation_id, situation_kind, selected_action, candidate_id, candidate_rank, degraded_json, created_at] := \
         *cognitive_shadow_decisions{shadow_decision_id, session_id: $session_id, turn_number: $turn_number, decision_id, situation_id, situation_kind, selected_action, candidate_id, candidate_rank, degraded_json, joined: false, created_at}",
        params,
        ScriptMutability::Immutable,
        "read pending shadow decision",
    )?;
    let mut observations: Vec<ShadowObservation> = rows
        .rows
        .iter()
        .map(|row| ShadowObservation {
            shadow_decision_id: str_col(row, 0),
            session_id: session_id.to_string(),
            turn_number,
            decision_id: str_col(row, 1),
            situation_id: str_col(row, 2),
            situation_kind: situation_kind(&str_col(row, 3)),
            selected_action: str_col(row, 4).parse().ok(),
            candidate_id: str_col(row, 5),
            candidate_rank: row[6].get_int().unwrap_or(0).max(0) as u64,
            degraded: serde_json::from_str(&str_col(row, 7)).unwrap_or_default(),
            created_at: parse_time(&str_col(row, 8)),
        })
        .collect();
    observations.sort_by(|left, right| left.shadow_decision_id.cmp(&right.shadow_decision_id));
    Ok(observations.pop())
}

pub(crate) fn mark_joined(
    db: &DbInstance,
    observation: &ShadowObservation,
    live: &LiveTurnOutcome,
    agreed: Option<bool>,
    surprise: Option<f32>,
) -> Result<(), CognitiveError> {
    let mut params = BTreeMap::new();
    params.insert(
        "shadow_decision_id".into(),
        DataValue::from(observation.shadow_decision_id.as_str()),
    );
    params.insert(
        "live_action".into(),
        DataValue::from(
            live.observed_action
                .map(CandidateActionKind::as_str)
                .unwrap_or(""),
        ),
    );
    params.insert(
        "live_outcome_status".into(),
        DataValue::from(live.outcome_status()),
    );
    params.insert(
        "agreed".into(),
        agreed.map(DataValue::from).unwrap_or(DataValue::Null),
    );
    params.insert(
        "surprise".into(),
        surprise
            .map(|value| DataValue::from(value as f64))
            .unwrap_or(DataValue::Null),
    );
    params.insert(
        "joined_at".into(),
        DataValue::from(crate::shadow::types::now().to_rfc3339().as_str()),
    );
    run_script_guarded(
        db,
        "?[shadow_decision_id, session_id, turn_number, decision_id, situation_id, situation_kind, selected_action, candidate_id, candidate_rank, degraded_json, joined, live_action, live_outcome_status, agreed, surprise, created_at, joined_at] := \
         *cognitive_shadow_decisions{shadow_decision_id, session_id, turn_number, decision_id, situation_id, situation_kind, selected_action, candidate_id, candidate_rank, degraded_json, created_at}, \
         shadow_decision_id = $shadow_decision_id, \
         joined = true, live_action = $live_action, live_outcome_status = $live_outcome_status, agreed = $agreed, surprise = $surprise, joined_at = $joined_at
         :put cognitive_shadow_decisions { shadow_decision_id => session_id, turn_number, decision_id, situation_id, situation_kind, selected_action, candidate_id, candidate_rank, degraded_json, joined, live_action, live_outcome_status, agreed, surprise, created_at, joined_at }",
        params,
        ScriptMutability::Mutable,
        "mark shadow decision joined",
    )?;
    Ok(())
}

fn str_col(row: &[DataValue], index: usize) -> String {
    row.get(index)
        .and_then(DataValue::get_str)
        .unwrap_or("")
        .to_string()
}

fn parse_time(value: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&chrono::Utc))
        .unwrap_or_else(|_| crate::shadow::types::now())
}

fn situation_kind(value: &str) -> SituationKind {
    match value {
        "ci_debug" => SituationKind::CiDebug,
        "code_change" => SituationKind::CodeChange,
        "git_mutation" => SituationKind::GitMutation,
        "pipeline_control" => SituationKind::PipelineControl,
        "research" => SituationKind::Research,
        "world_model_task" => SituationKind::WorldModelTask,
        "high_risk" => SituationKind::HighRisk,
        "simple_question" => SituationKind::SimpleQuestion,
        "ambiguous" => SituationKind::Ambiguous,
        _ => SituationKind::Greeting,
    }
}
