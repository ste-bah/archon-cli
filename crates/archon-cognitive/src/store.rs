use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use chrono::Utc;
use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};
use uuid::Uuid;

use crate::schema::ensure_cognitive_schema;
use crate::types::{CognitiveDecision, CognitiveError, Situation, ToolVerdict};

const MAX_ATTEMPTS: usize = 4;
static COZO_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

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

fn relation_count(db: &DbInstance, relation: &str, field: &str) -> Result<usize, CognitiveError> {
    let query = format!("?[{field}] := *{relation}{{{field}}}");
    let rows = run_script_guarded(
        db,
        query.as_str(),
        Default::default(),
        ScriptMutability::Immutable,
        "count cognitive relation",
    )?;
    Ok(rows.rows.len())
}

fn run_script_guarded(
    db: &DbInstance,
    script: &str,
    params: BTreeMap<String, DataValue>,
    mutability: ScriptMutability,
    context: &str,
) -> Result<NamedRows, CognitiveError> {
    let mut last_error = String::new();
    for attempt in 0..MAX_ATTEMPTS {
        let lock = COZO_LOCK.get_or_init(|| Mutex::new(()));
        let guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let result = catch_unwind(AssertUnwindSafe(|| {
            db.run_script(script, params.clone(), mutability)
        }));
        drop(guard);

        match result {
            Ok(Ok(rows)) => return Ok(rows),
            Ok(Err(error)) => {
                last_error = error.to_string();
                if retryable_cozo_error(&last_error) && attempt + 1 < MAX_ATTEMPTS {
                    backoff(attempt);
                    continue;
                }
                return Err(CognitiveError::Store(format!("{context}: {last_error}")));
            }
            Err(payload) => {
                last_error = panic_payload_message(payload.as_ref());
                if retryable_cozo_error(&last_error) && attempt + 1 < MAX_ATTEMPTS {
                    backoff(attempt);
                    continue;
                }
                return Err(CognitiveError::Store(format!(
                    "{context}: cozo sqlite backend panicked: {last_error}"
                )));
            }
        }
    }
    Err(CognitiveError::Store(format!(
        "{context}: cozo sqlite backend stayed busy after {MAX_ATTEMPTS} attempts: {last_error}"
    )))
}

fn open_sqlite_guarded(path: &Path, context: &str) -> Result<DbInstance, CognitiveError> {
    let mut last_error = String::new();
    let path = path.to_string_lossy().to_string();
    for attempt in 0..MAX_ATTEMPTS {
        let lock = COZO_LOCK.get_or_init(|| Mutex::new(()));
        let guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let result = catch_unwind(AssertUnwindSafe(|| DbInstance::new("sqlite", &path, "")));
        drop(guard);

        match result {
            Ok(Ok(db)) => return Ok(db),
            Ok(Err(error)) => {
                last_error = error.to_string();
                if retryable_cozo_error(&last_error) && attempt + 1 < MAX_ATTEMPTS {
                    backoff(attempt);
                    continue;
                }
                return Err(CognitiveError::Store(format!("{context}: {last_error}")));
            }
            Err(payload) => {
                last_error = panic_payload_message(payload.as_ref());
                if retryable_cozo_error(&last_error) && attempt + 1 < MAX_ATTEMPTS {
                    backoff(attempt);
                    continue;
                }
                return Err(CognitiveError::Store(format!(
                    "{context}: cozo sqlite backend panicked: {last_error}"
                )));
            }
        }
    }
    Err(CognitiveError::Store(format!(
        "{context}: cozo sqlite backend stayed busy after {MAX_ATTEMPTS} attempts: {last_error}"
    )))
}

fn retryable_cozo_error(message: &str) -> bool {
    message.contains("database is locked") || message.contains("code: Some(5)")
}

fn backoff(attempt: usize) {
    thread::sleep(Duration::from_millis(25 * (attempt as u64 + 1)));
}

fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else {
        "unknown panic payload".to_string()
    }
}
