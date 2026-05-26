use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

use crate::cozo_guard::{relation_count, run_script_guarded};
use crate::schema::ensure_cognitive_schema;
use crate::self_model::SelfModelBriefing;
use crate::self_model::SelfModelStore;
use crate::{CognitiveError, DecisionRecord, DecisionStore, TickReport};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CognitiveInspectionStatus {
    pub situation_count: usize,
    pub tool_decision_count: usize,
    pub executive_decision_count: usize,
    pub reflection_count: usize,
    pub proposal_count: usize,
    pub apply_result_count: usize,
    pub self_model_fact_count: usize,
    pub latest_tick: Option<TickSummary>,
    pub recent_decisions: Vec<DecisionSummary>,
    pub recent_reflections: Vec<ReflectionSummary>,
    pub pending_proposals: Vec<ProposalSummary>,
    pub self_model: SelfModelBriefing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionSummary {
    pub decision_id: String,
    pub session_id: String,
    pub turn_number: u64,
    pub selected_candidate_id: String,
    pub rejected_count: usize,
    pub user_visible_summary: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReflectionSummary {
    pub reflection_id: String,
    pub session_id: String,
    pub turn_number: u64,
    pub decision_id: String,
    pub situation_kind: String,
    pub outcome: String,
    pub lesson: String,
    pub should_propose: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalSummary {
    pub proposal_id: String,
    pub manifest_kind: String,
    pub risk_level: String,
    pub evidence_count: u64,
    pub domain: String,
    pub lesson_tag: String,
    pub latest_result: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TickSummary {
    pub tick_id: String,
    pub proposals_evaluated: u64,
    pub proposals_auto_applied: u64,
    pub proposals_denied: u64,
    pub self_model_updated: bool,
    pub error_count: usize,
    pub duration_ms: u64,
    pub created_at: DateTime<Utc>,
}

pub struct CognitiveInspection<'a> {
    db: &'a DbInstance,
    ledger_dir: std::path::PathBuf,
}

impl<'a> CognitiveInspection<'a> {
    pub fn new(
        db: &'a DbInstance,
        ledger_dir: impl AsRef<std::path::Path>,
    ) -> Result<Self, CognitiveError> {
        ensure_cognitive_schema(db)?;
        Ok(Self {
            db,
            ledger_dir: ledger_dir.as_ref().to_path_buf(),
        })
    }

    pub fn status(&self) -> Result<CognitiveInspectionStatus, CognitiveError> {
        Ok(CognitiveInspectionStatus {
            situation_count: count(self.db, "cognitive_situations", "situation_id"),
            tool_decision_count: count(self.db, "cognitive_tool_decisions", "id"),
            executive_decision_count: count(self.db, "cognitive_decisions", "decision_id"),
            reflection_count: count(self.db, "cognitive_reflections", "reflection_id"),
            proposal_count: count(self.db, "governed_proposals", "proposal_id"),
            apply_result_count: count(self.db, "autonomous_apply_results", "apply_id"),
            self_model_fact_count: count(self.db, "self_model_facts", "fact_id"),
            latest_tick: self.latest_tick()?,
            recent_decisions: self.recent_decisions(5)?,
            recent_reflections: self.reflections(None, 5)?,
            pending_proposals: self.pending_proposals(5)?,
            self_model: SelfModelStore::new(self.db)?.export_briefing()?,
        })
    }

    pub fn inspect_decision(
        &self,
        decision_id: &str,
    ) -> Result<Option<DecisionRecord>, CognitiveError> {
        DecisionStore::new(self.db, self.ledger_dir.join("cognitive-decisions.jsonl"))?
            .get(decision_id)
    }

    pub fn decisions_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<DecisionRecord>, CognitiveError> {
        DecisionStore::new(self.db, self.ledger_dir.join("cognitive-decisions.jsonl"))?
            .list_for_session(session_id, limit)
    }

    pub fn recent_decisions(&self, limit: usize) -> Result<Vec<DecisionSummary>, CognitiveError> {
        let rows = run_script_guarded(
            self.db,
            "?[decision_id, session_id, turn_number, selected_candidate_id, rejected_candidates_json, user_visible_summary, created_at] := \
             *cognitive_decisions{decision_id, situation_id, session_id, turn_number, selected_candidate_id, rejected_candidates_json, heuristic_scores_json, policy_verdict_json, verification_contract_json, user_visible_summary, created_at}",
            Default::default(),
            ScriptMutability::Immutable,
            "query recent cognitive decisions",
        )?;
        let mut values: Vec<_> = rows
            .rows
            .iter()
            .map(|row| row_to_decision_summary(row))
            .collect();
        values.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        values.truncate(limit);
        Ok(values)
    }

    pub fn reflections(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ReflectionSummary>, CognitiveError> {
        let mut params = BTreeMap::new();
        let query = if let Some(session_id) = session_id {
            params.insert("session_id".into(), DataValue::from(session_id));
            "?[reflection_id, session_id, turn_number, decision_id, situation_kind, outcome, lesson, should_propose, created_at] := \
             *cognitive_reflections{reflection_id, session_id, turn_number, decision_id, situation_kind, attempted, worked, failed, outcome, lesson, should_propose, proposed_rule_id, created_at}, session_id = $session_id".to_string()
        } else {
            "?[reflection_id, session_id, turn_number, decision_id, situation_kind, outcome, lesson, should_propose, created_at] := \
             *cognitive_reflections{reflection_id, session_id, turn_number, decision_id, situation_kind, attempted, worked, failed, outcome, lesson, should_propose, proposed_rule_id, created_at}".to_string()
        };
        let rows = run_script_guarded(
            self.db,
            query.as_str(),
            params,
            ScriptMutability::Immutable,
            "query cognitive reflections",
        )?;
        let mut values: Vec<_> = rows
            .rows
            .iter()
            .map(|row| row_to_reflection_summary(row))
            .collect();
        values.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        values.truncate(limit);
        Ok(values)
    }

    pub fn pending_proposals(&self, limit: usize) -> Result<Vec<ProposalSummary>, CognitiveError> {
        let rows = run_script_guarded(
            self.db,
            "?[proposal_id, manifest_kind, risk_level, evidence_count, lesson_tag, domain, created_at] := \
             *governed_proposals{proposal_id, reflection_ids_json, manifest_kind, risk_level, evidence_count, lesson_tag, domain, diff_summary, rollback_plan, created_at}",
            Default::default(),
            ScriptMutability::Immutable,
            "query governed proposals",
        )?;
        let mut values: Vec<_> = rows
            .rows
            .iter()
            .map(|row| self.row_to_proposal(row))
            .collect();
        values.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        values.truncate(limit);
        Ok(values)
    }

    pub fn latest_tick(&self) -> Result<Option<TickSummary>, CognitiveError> {
        let rows = run_script_guarded(
            self.db,
            "?[tick_id, proposals_evaluated, proposals_auto_applied, proposals_denied, self_model_updated, errors_json, duration_ms, created_at] := \
             *cognitive_tick_audit{tick_id, dead_letters_replayed, proposals_evaluated, proposals_auto_applied, proposals_denied, self_model_updated, errors_json, duration_ms, created_at}",
            Default::default(),
            ScriptMutability::Immutable,
            "query cognitive tick audit",
        )?;
        let mut ticks: Vec<_> = rows.rows.iter().map(|row| row_to_tick(row)).collect();
        ticks.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        Ok(ticks.into_iter().next())
    }

    fn row_to_proposal(&self, row: &[DataValue]) -> ProposalSummary {
        let proposal_id = str_col(row, 0);
        ProposalSummary {
            proposal_id: proposal_id.clone(),
            manifest_kind: str_col(row, 1),
            risk_level: str_col(row, 2),
            evidence_count: int_col(row, 3),
            lesson_tag: str_col(row, 4),
            domain: str_col(row, 5),
            latest_result: self.latest_apply_result(&proposal_id).ok().flatten(),
            created_at: time_col(row, 6),
        }
    }

    fn latest_apply_result(&self, proposal_id: &str) -> Result<Option<String>, CognitiveError> {
        let mut params = BTreeMap::new();
        params.insert("proposal_id".into(), DataValue::from(proposal_id));
        let rows = run_script_guarded(
            self.db,
            "?[result_kind, created_at] := *autonomous_apply_results{proposal_id: $proposal_id, result_kind, created_at}",
            params,
            ScriptMutability::Immutable,
            "query latest autonomous apply result",
        )?;
        Ok(rows
            .rows
            .iter()
            .max_by_key(|row| time_col(row, 1))
            .map(|row| str_col(row, 0)))
    }
}

impl From<&TickReport> for TickSummary {
    fn from(report: &TickReport) -> Self {
        Self {
            tick_id: report.tick_id.clone(),
            proposals_evaluated: report.proposals_evaluated,
            proposals_auto_applied: report.proposals_auto_applied,
            proposals_denied: report.proposals_denied,
            self_model_updated: report.self_model_updated,
            error_count: report.errors.len(),
            duration_ms: report.duration_ms,
            created_at: report.created_at,
        }
    }
}

fn count(db: &DbInstance, relation: &str, field: &str) -> usize {
    relation_count(db, relation, field).unwrap_or(0)
}

fn row_to_decision_summary(row: &[DataValue]) -> DecisionSummary {
    DecisionSummary {
        decision_id: str_col(row, 0),
        session_id: str_col(row, 1),
        turn_number: int_col(row, 2),
        selected_candidate_id: str_col(row, 3),
        rejected_count: rejected_count(&str_col(row, 4)),
        user_visible_summary: str_col(row, 5),
        created_at: time_col(row, 6),
    }
}

fn row_to_reflection_summary(row: &[DataValue]) -> ReflectionSummary {
    ReflectionSummary {
        reflection_id: str_col(row, 0),
        session_id: str_col(row, 1),
        turn_number: int_col(row, 2),
        decision_id: str_col(row, 3),
        situation_kind: str_col(row, 4),
        outcome: str_col(row, 5),
        lesson: str_col(row, 6),
        should_propose: row[7].get_bool().unwrap_or(false),
        created_at: time_col(row, 8),
    }
}

fn row_to_tick(row: &[DataValue]) -> TickSummary {
    TickSummary {
        tick_id: str_col(row, 0),
        proposals_evaluated: int_col(row, 1),
        proposals_auto_applied: int_col(row, 2),
        proposals_denied: int_col(row, 3),
        self_model_updated: row[4].get_bool().unwrap_or(false),
        error_count: json_array_len(&str_col(row, 5)),
        duration_ms: int_col(row, 6),
        created_at: time_col(row, 7),
    }
}

fn str_col(row: &[DataValue], index: usize) -> String {
    row.get(index)
        .and_then(DataValue::get_str)
        .unwrap_or("")
        .to_string()
}

fn int_col(row: &[DataValue], index: usize) -> u64 {
    row.get(index)
        .and_then(DataValue::get_int)
        .unwrap_or(0)
        .max(0) as u64
}

fn time_col(row: &[DataValue], index: usize) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&str_col(row, index))
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn rejected_count(value: &str) -> usize {
    serde_json::from_str::<Vec<serde_json::Value>>(value)
        .map(|values| values.len())
        .unwrap_or(0)
}

fn json_array_len(value: &str) -> usize {
    serde_json::from_str::<Vec<serde_json::Value>>(value)
        .map(|values| values.len())
        .unwrap_or(0)
}
