use std::collections::BTreeMap;

use chrono::Utc;
use cozo::{DataValue, ScriptMutability};
use uuid::Uuid;

use super::{SessionError, SessionMetadata, SessionStore, db_err};

impl SessionStore {
    pub fn register_session(
        &self,
        id: &str,
        working_dir: &str,
        git_branch: Option<&str>,
        model: &str,
    ) -> Result<SessionMetadata, SessionError> {
        self.create_session_with_id(id, working_dir, git_branch, model)
    }

    pub fn create_session(
        &self,
        working_dir: &str,
        git_branch: Option<&str>,
        model: &str,
    ) -> Result<SessionMetadata, SessionError> {
        self.create_session_with_id(&Uuid::new_v4().to_string(), working_dir, git_branch, model)
    }

    fn create_session_with_id(
        &self,
        id: &str,
        working_dir: &str,
        git_branch: Option<&str>,
        model: &str,
    ) -> Result<SessionMetadata, SessionError> {
        let now = Utc::now().to_rfc3339();
        let metadata = SessionMetadata {
            id: id.to_string(),
            created_at: now.clone(),
            last_active: now,
            working_directory: working_dir.to_string(),
            git_branch: git_branch.map(str::to_string),
            model: model.to_string(),
            message_count: 0,
            total_tokens: 0,
            total_cost: 0.0,
            schema_version: 1,
            name: None,
            parent_session_id: None,
        };
        self.put_session(&metadata)?;
        Ok(metadata)
    }

    pub fn get_session(&self, session_id: &str) -> Result<SessionMetadata, SessionError> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));
        let exact = self
            .db
            .run_script(
                "?[id, created_at, last_active, working_directory, git_branch, model, message_count, total_tokens, total_cost, schema_version] :=
                    *sessions{id, created_at, last_active, working_directory, git_branch, model, message_count, total_tokens, total_cost, schema_version},
                    id = $sid",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;
        let result = if exact.rows.is_empty() && session_id.len() < 36 {
            self.resolve_session_prefix(session_id)?
        } else {
            exact
        };
        let row = result
            .rows
            .first()
            .ok_or_else(|| SessionError::NotFound(format!("session '{session_id}' not found")))?;
        let mut metadata = self.metadata_from_row(row);
        metadata.name = self.get_name(&metadata.id).unwrap_or(None);
        metadata.parent_session_id = self.get_parent(&metadata.id).unwrap_or(None);
        Ok(metadata)
    }

    fn resolve_session_prefix(&self, prefix: &str) -> Result<cozo::NamedRows, SessionError> {
        let all = self
            .db
            .run_script(
                "?[id, created_at, last_active, working_directory, git_branch, model, message_count, total_tokens, total_cost, schema_version] :=
                    *sessions{id, created_at, last_active, working_directory, git_branch, model, message_count, total_tokens, total_cost, schema_version}",
                BTreeMap::new(),
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;
        let matches = all
            .rows
            .iter()
            .filter(|row| super::extract_str(&row[0]).starts_with(prefix))
            .cloned()
            .collect::<Vec<_>>();
        match matches.len() {
            0 => Err(SessionError::NotFound(format!(
                "session '{prefix}' not found"
            ))),
            1 => Ok(cozo::NamedRows {
                rows: matches,
                headers: all.headers,
                ..Default::default()
            }),
            count => Err(SessionError::NotFound(format!(
                "ambiguous session prefix '{prefix}' matches {count} sessions — use more characters"
            ))),
        }
    }

    pub fn update_usage(
        &self,
        session_id: &str,
        total_tokens: u64,
        total_cost: f64,
    ) -> Result<(), SessionError> {
        let mut metadata = self.get_session(session_id)?;
        metadata.last_active = Utc::now().to_rfc3339();
        metadata.message_count = metadata.message_count.saturating_add(1);
        metadata.total_tokens = total_tokens;
        metadata.total_cost = total_cost;
        self.put_session(&metadata)
    }

    pub fn delete_session(&self, session_id: &str) -> Result<(), SessionError> {
        let transaction = self.db.multi_transaction(true);
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));
        let result = (|| {
            for script in [
                "?[id] := *compaction_segments{id, session_id}, session_id = $sid :rm compaction_segment_bodies {id}",
                "?[id] := *compaction_segments{id, session_id}, session_id = $sid :rm compaction_segments {id}",
                "?[id] := *compaction_ledger{id, session_id}, session_id = $sid :rm compaction_ledger {id}",
                "?[id] := *compaction_telemetry{id, session_id}, session_id = $sid :rm compaction_telemetry {id}",
            ] {
                transaction
                    .run_script(script, params.clone())
                    .map_err(db_err)?;
            }
            #[cfg(test)]
            if self
                .fail_next_delete_after_compaction
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                return Err(SessionError::DbError(
                    "injected delete failure after compaction cleanup".into(),
                ));
            }
            for script in [
                "?[session_id, message_index] := *messages{session_id, message_index}, session_id = $sid :rm messages {session_id, message_index}",
                "?[session_id, tag] := *session_tags{session_id, tag}, session_id = $sid :rm session_tags {session_id, tag}",
                "?[id] := id = $sid :rm sessions {id}",
            ] {
                transaction
                    .run_script(script, params.clone())
                    .map_err(db_err)?;
            }
            Ok(())
        })();
        if result.is_ok() {
            transaction.commit().map_err(db_err)?;
        } else {
            let _ = transaction.abort();
        }
        result
    }

    pub fn verify_wal_mode(&self) -> Result<bool, SessionError> {
        Ok(true)
    }

    pub(crate) fn put_session(&self, metadata: &SessionMetadata) -> Result<(), SessionError> {
        let mut params = BTreeMap::new();
        params.insert("id".to_string(), DataValue::from(metadata.id.as_str()));
        params.insert(
            "created_at".to_string(),
            DataValue::from(metadata.created_at.as_str()),
        );
        params.insert(
            "last_active".to_string(),
            DataValue::from(metadata.last_active.as_str()),
        );
        params.insert(
            "working_directory".to_string(),
            DataValue::from(metadata.working_directory.as_str()),
        );
        params.insert(
            "git_branch".to_string(),
            DataValue::from(metadata.git_branch.as_deref().unwrap_or("")),
        );
        params.insert(
            "model".to_string(),
            DataValue::from(metadata.model.as_str()),
        );
        params.insert(
            "message_count".to_string(),
            DataValue::from(clamp_u64_to_i64(metadata.message_count)),
        );
        params.insert(
            "total_tokens".to_string(),
            DataValue::from(clamp_u64_to_i64(metadata.total_tokens)),
        );
        params.insert(
            "total_cost".to_string(),
            DataValue::from(metadata.total_cost),
        );
        params.insert(
            "schema_version".to_string(),
            DataValue::from(i64::from(metadata.schema_version)),
        );
        self.db
            .run_mutable(
                "?[id, created_at, last_active, working_directory, git_branch, model, message_count, total_tokens, total_cost, schema_version] <- [[
                    $id, $created_at, $last_active, $working_directory, $git_branch, $model,
                    $message_count, $total_tokens, $total_cost, $schema_version
                ]]
                :put sessions {id => created_at, last_active, working_directory, git_branch, model, message_count, total_tokens, total_cost, schema_version}",
                params,
                "session store: put session metadata",
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub(crate) fn metadata_from_row(&self, row: &[DataValue]) -> SessionMetadata {
        let branch = super::extract_str(&row[4]);
        let id = super::extract_str(&row[0]);
        SessionMetadata {
            id,
            created_at: super::extract_str(&row[1]),
            last_active: super::extract_str(&row[2]),
            working_directory: super::extract_str(&row[3]),
            git_branch: (!branch.is_empty()).then_some(branch),
            model: super::extract_str(&row[5]),
            message_count: super::extract_i64(&row[6]) as u64,
            total_tokens: super::extract_i64(&row[7]) as u64,
            total_cost: super::extract_f64(&row[8]),
            schema_version: super::extract_i64(&row[9]) as u32,
            name: None,
            parent_session_id: None,
        }
    }
}

pub(crate) fn clamp_u64_to_i64(value: u64) -> i64 {
    value.min(i64::MAX as u64) as i64
}
