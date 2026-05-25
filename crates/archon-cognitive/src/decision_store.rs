use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::cozo_guard::run_script_guarded;
use crate::decision_codec::{
    decision_from_candidates, query_script, row_to_decision, rows_to_decisions,
};
use crate::schema::ensure_cognitive_schema;
use crate::{Candidate, CognitiveError, DecisionRecord, Situation};

pub struct DecisionStore<'a> {
    db: &'a DbInstance,
    ledger_path: PathBuf,
}

impl<'a> DecisionStore<'a> {
    pub fn new(db: &'a DbInstance, ledger_path: impl AsRef<Path>) -> Result<Self, CognitiveError> {
        ensure_cognitive_schema(db)?;
        let ledger_path = ledger_path.as_ref().to_path_buf();
        if let Some(parent) = ledger_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(Self { db, ledger_path })
    }

    pub fn decision_from_candidates(
        situation: &Situation,
        candidates: &[Candidate],
    ) -> Result<DecisionRecord, CognitiveError> {
        decision_from_candidates(situation, candidates)
    }

    pub fn record(&self, decision: &DecisionRecord) -> Result<String, CognitiveError> {
        self.put_decision(decision)?;
        append_ledger(&self.ledger_path, decision)?;
        Ok(decision.decision_id.clone())
    }

    pub fn get(&self, decision_id: &str) -> Result<Option<DecisionRecord>, CognitiveError> {
        let mut params = BTreeMap::new();
        params.insert("decision_id".into(), DataValue::from(decision_id));
        let rows = run_script_guarded(
            self.db,
            query_script("decision_id = $decision_id").as_str(),
            params,
            ScriptMutability::Immutable,
            "get cognitive decision",
        )?;
        rows.rows.first().map(row_to_decision).transpose()
    }

    pub fn list_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<DecisionRecord>, CognitiveError> {
        let mut params = BTreeMap::new();
        params.insert("session_id".into(), DataValue::from(session_id));
        let rows = run_script_guarded(
            self.db,
            query_script("session_id = $session_id").as_str(),
            params,
            ScriptMutability::Immutable,
            "list cognitive decisions by session",
        )?;
        let mut decisions = rows_to_decisions(rows)?;
        decisions.sort_by(|left, right| right.turn_number.cmp(&left.turn_number));
        decisions.truncate(limit);
        Ok(decisions)
    }

    pub fn list_for_situation(
        &self,
        situation_id: &str,
    ) -> Result<Vec<DecisionRecord>, CognitiveError> {
        let mut params = BTreeMap::new();
        params.insert("situation_id".into(), DataValue::from(situation_id));
        let rows = run_script_guarded(
            self.db,
            query_script("situation_id = $situation_id").as_str(),
            params,
            ScriptMutability::Immutable,
            "list cognitive decisions by situation",
        )?;
        rows_to_decisions(rows)
    }

    pub fn replay_ledger(
        &self,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<DecisionRecord>, CognitiveError> {
        if !self.ledger_path.exists() {
            return Ok(Vec::new());
        }
        let file = std::fs::File::open(&self.ledger_path)?;
        let mut decisions = Vec::new();
        for line in BufReader::new(file).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let decision: DecisionRecord = serde_json::from_str(&line)?;
            if since.is_none_or(|cutoff| decision.created_at >= cutoff) {
                decisions.push(decision);
            }
        }
        Ok(decisions)
    }

    pub fn update_policy_verdict(
        &self,
        decision_id: &str,
        verdict: &str,
    ) -> Result<(), CognitiveError> {
        let Some(mut decision) = self.get(decision_id)? else {
            return Ok(());
        };
        decision.policy_verdict = Some(verdict.to_string());
        self.put_decision(&decision)
    }

    pub fn update_verification_contract(
        &self,
        decision_id: &str,
        contract: &str,
    ) -> Result<(), CognitiveError> {
        let Some(mut decision) = self.get(decision_id)? else {
            return Ok(());
        };
        decision.verification_contract = Some(contract.to_string());
        self.put_decision(&decision)
    }

    fn put_decision(&self, decision: &DecisionRecord) -> Result<(), CognitiveError> {
        let rejected = serde_json::to_string(&decision.rejected_alternatives)?;
        let scores = serde_json::to_string(&decision.heuristic_scores)?;
        let mut params = BTreeMap::new();
        params.insert(
            "decision_id".into(),
            DataValue::from(decision.decision_id.as_str()),
        );
        params.insert(
            "situation_id".into(),
            DataValue::from(decision.situation_id.as_str()),
        );
        params.insert(
            "session_id".into(),
            DataValue::from(decision.session_id.as_str()),
        );
        params.insert(
            "turn_number".into(),
            DataValue::from(decision.turn_number as i64),
        );
        params.insert(
            "selected_candidate_id".into(),
            DataValue::from(decision.selected_candidate_id.as_str()),
        );
        params.insert(
            "rejected_candidates_json".into(),
            DataValue::from(rejected.as_str()),
        );
        params.insert(
            "heuristic_scores_json".into(),
            DataValue::from(scores.as_str()),
        );
        params.insert(
            "policy_verdict_json".into(),
            DataValue::from(decision.policy_verdict.as_deref().unwrap_or("")),
        );
        params.insert(
            "verification_contract_json".into(),
            DataValue::from(decision.verification_contract.as_deref().unwrap_or("")),
        );
        params.insert(
            "user_visible_summary".into(),
            DataValue::from(decision.user_visible_summary.as_str()),
        );
        params.insert(
            "created_at".into(),
            DataValue::from(decision.created_at.to_rfc3339().as_str()),
        );
        run_script_guarded(
            self.db,
            "?[decision_id, situation_id, session_id, turn_number, selected_candidate_id, rejected_candidates_json, heuristic_scores_json, policy_verdict_json, verification_contract_json, user_visible_summary, created_at] <- \
             [[$decision_id, $situation_id, $session_id, $turn_number, $selected_candidate_id, $rejected_candidates_json, $heuristic_scores_json, $policy_verdict_json, $verification_contract_json, $user_visible_summary, $created_at]]
             :put cognitive_decisions { decision_id => situation_id, session_id, turn_number, selected_candidate_id, rejected_candidates_json, heuristic_scores_json, policy_verdict_json, verification_contract_json, user_visible_summary, created_at }",
            params,
            ScriptMutability::Mutable,
            "put cognitive decision",
        )?;
        Ok(())
    }
}

fn append_ledger(path: &Path, decision: &DecisionRecord) -> Result<(), CognitiveError> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", serde_json::to_string(decision)?)?;
    Ok(())
}
