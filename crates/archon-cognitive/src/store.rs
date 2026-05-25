use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::Utc;
use cozo::{DataValue, DbInstance, ScriptMutability};
use uuid::Uuid;

use crate::cozo_guard::{open_sqlite_guarded, relation_count, run_script_guarded};
use crate::schema::ensure_cognitive_schema;
use crate::types::{CognitiveDecision, CognitiveError, Situation, ToolVerdict};

pub struct PersistentCognitiveStore {
    db: DbInstance,
    root: PathBuf,
    db_path: PathBuf,
}

impl PersistentCognitiveStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, CognitiveError> {
        let root = root.as_ref();
        std::fs::create_dir_all(root)?;
        let root = root.canonicalize()?;
        let db_path = root.join("cognitive.db");
        let db = open_sqlite_guarded(&db_path, "open cognitive store")?;
        ensure_cognitive_schema(&db)?;
        Ok(Self { db, root, db_path })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn db(&self) -> &DbInstance {
        &self.db
    }

    pub fn put_situation(&self, situation: &Situation) -> Result<(), CognitiveError> {
        CognitiveStore::from_validated(&self.db).put_situation(situation)
    }

    pub fn put_decision(&self, decision: &CognitiveDecision) -> Result<(), CognitiveError> {
        CognitiveStore::from_validated(&self.db).put_decision(decision)
    }

    pub fn situation_count(&self) -> Result<usize, CognitiveError> {
        relation_count(&self.db, "cognitive_situations", "situation_id")
    }

    pub fn decision_count(&self) -> Result<usize, CognitiveError> {
        relation_count(&self.db, "cognitive_tool_decisions", "id")
    }
}

pub struct CognitiveStore<'a> {
    db: &'a DbInstance,
}

impl<'a> CognitiveStore<'a> {
    pub fn new(db: &'a DbInstance) -> Result<Self, CognitiveError> {
        ensure_cognitive_schema(db)?;
        Ok(Self { db })
    }

    fn from_validated(db: &'a DbInstance) -> Self {
        Self { db }
    }

    pub fn put_situation(&self, situation: &Situation) -> Result<(), CognitiveError> {
        let mut params = BTreeMap::new();
        params.insert(
            "situation_id".into(),
            DataValue::from(situation.id.as_str()),
        );
        params.insert(
            "session_id".into(),
            DataValue::from(situation.session_id.as_str()),
        );
        params.insert(
            "turn_number".into(),
            DataValue::from(situation.turn_number as i64),
        );
        params.insert(
            "user_text_hash".into(),
            DataValue::from(situation.user_text_hash.as_str()),
        );
        params.insert("kind".into(), DataValue::from(situation.kind.as_str()));
        params.insert(
            "confidence_score".into(),
            DataValue::from(situation.confidence_score as f64),
        );
        params.insert(
            "confidence".into(),
            DataValue::from(format!("{:?}", situation.confidence).to_ascii_lowercase()),
        );
        params.insert(
            "surface".into(),
            DataValue::from(format!("{:?}", situation.surface).to_ascii_lowercase()),
        );
        params.insert("evidence_refs".into(), DataValue::from("[]"));
        params.insert(
            "reason_summary".into(),
            DataValue::from(situation.reason.as_str()),
        );
        params.insert(
            "created_at".into(),
            DataValue::from(situation.created_at.to_rfc3339().as_str()),
        );

        run_script_guarded(
            self.db,
            "?[situation_id, session_id, turn_number, user_text_hash, surface, kind, confidence_score, confidence, evidence_refs, reason_summary, created_at] <- \
                 [[$situation_id, $session_id, $turn_number, $user_text_hash, $surface, $kind, $confidence_score, $confidence, $evidence_refs, $reason_summary, $created_at]]
                 :put cognitive_situations { situation_id => session_id, turn_number, user_text_hash, surface, kind, confidence_score, confidence, evidence_refs, reason_summary, created_at }",
            params,
            ScriptMutability::Mutable,
            "put cognitive situation",
        )?;
        Ok(())
    }

    pub fn put_decision(&self, decision: &CognitiveDecision) -> Result<(), CognitiveError> {
        let verdict_json = serde_json::to_string(&decision.verdict)?;
        let mut params = BTreeMap::new();
        params.insert("id".into(), DataValue::from(decision.id.as_str()));
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
            "tool_name".into(),
            DataValue::from(decision.tool_name.as_deref().unwrap_or("")),
        );
        params.insert(
            "verdict_json".into(),
            DataValue::from(verdict_json.as_str()),
        );
        params.insert("reason".into(), DataValue::from(decision.reason.as_str()));
        params.insert(
            "created_at".into(),
            DataValue::from(decision.created_at.to_rfc3339().as_str()),
        );

        run_script_guarded(
            self.db,
            "?[id, situation_id, session_id, turn_number, tool_name, verdict_json, reason, created_at] <- \
                 [[$id, $situation_id, $session_id, $turn_number, $tool_name, $verdict_json, $reason, $created_at]]
                 :put cognitive_tool_decisions { id => situation_id, session_id, turn_number, tool_name, verdict_json, reason, created_at }",
            params,
            ScriptMutability::Mutable,
            "put cognitive tool decision",
        )?;
        Ok(())
    }
}

impl CognitiveDecision {
    pub fn for_tool(situation: &Situation, tool_name: &str, verdict: ToolVerdict) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            situation_id: situation.id.clone(),
            session_id: situation.session_id.clone(),
            turn_number: situation.turn_number,
            tool_name: Some(tool_name.to_owned()),
            reason: verdict.reason().to_owned(),
            verdict,
            created_at: Utc::now(),
        }
    }
}
