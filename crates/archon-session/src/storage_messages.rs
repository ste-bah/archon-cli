use std::collections::BTreeMap;

use chrono::Utc;
use cozo::{DataValue, ScriptMutability};

use super::storage_session_ops::clamp_u64_to_i64;
use super::{SessionError, SessionStore, db_err, extract_str};

impl SessionStore {
    pub fn save_message(
        &self,
        session_id: &str,
        message_index: u64,
        content: &str,
    ) -> Result<(), SessionError> {
        let session = self.get_session(session_id)?;
        if message_index > session.message_count {
            return Err(SessionError::MessageIndexGap {
                index: message_index,
                message_count: session.message_count,
            });
        }
        let mut params = BTreeMap::new();
        params.insert("session_id".to_string(), DataValue::from(session_id));
        params.insert(
            "message_index".to_string(),
            DataValue::from(clamp_u64_to_i64(message_index)),
        );
        params.insert("content".to_string(), DataValue::from(content));
        self.db
            .run_mutable(
                "?[session_id, message_index, content] <- [[$session_id, $message_index, $content]]
                 :put messages {session_id, message_index => content}",
                params,
                "session store: put session message",
            )
            .map_err(db_err)?;
        self.set_message_count(
            session_id,
            session.message_count.max(message_index.saturating_add(1)),
        )
    }

    pub fn replace_messages(
        &self,
        session_id: &str,
        messages: &[String],
    ) -> Result<(), SessionError> {
        if messages.is_empty() {
            return Err(SessionError::EmptyReplaceRefused);
        }
        let session = self.get_session(session_id)?;
        let transaction = self.db.multi_transaction(true);
        let result = self.replace_messages_in_transaction(&transaction, &session, messages);
        if result.is_ok() {
            transaction.commit().map_err(db_err)?;
        } else {
            let _ = transaction.abort();
        }
        result
    }

    fn replace_messages_in_transaction(
        &self,
        transaction: &cozo::MultiTransaction,
        session: &super::SessionMetadata,
        messages: &[String],
    ) -> Result<(), SessionError> {
        let logical_count = clamp_u64_to_i64(messages.len() as u64);
        let rows = DataValue::List(
            messages
                .iter()
                .enumerate()
                .map(|(index, content)| {
                    DataValue::List(vec![
                        DataValue::from(session.id.as_str()),
                        DataValue::from(index as i64),
                        DataValue::from(content.as_str()),
                    ])
                })
                .collect(),
        );
        let mut params = BTreeMap::new();
        params.insert("rows".to_string(), rows);
        transaction
            .run_script(
                "?[session_id, message_index, content] <- $rows
                 :put messages {session_id, message_index => content}",
                params,
            )
            .map_err(db_err)?;
        #[cfg(test)]
        if self
            .fail_next_replace_after_rows
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            transaction
                .run_script("?[bad] := invalid_relation{bad}", BTreeMap::new())
                .map_err(db_err)?;
        }
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session.id.as_str()));
        params.insert("count".to_string(), DataValue::from(logical_count));
        transaction
            .run_script(
                "?[session_id, message_index] := *messages{session_id, message_index}, session_id = $sid, message_index >= $count
                 :rm messages {session_id, message_index}",
                params.clone(),
            )
            .map_err(db_err)?;
        params.insert(
            "last_active".to_string(),
            DataValue::from(Utc::now().to_rfc3339()),
        );
        transaction
            .run_script(
                "?[id, created_at, last_active, working_directory, git_branch, model, message_count, total_tokens, total_cost, schema_version] <- [[
                    $sid, $created_at, $last_active, $working_directory, $git_branch, $model, $count, $total_tokens, $total_cost, $schema_version
                ]]
                :put sessions {id => created_at, last_active, working_directory, git_branch, model, message_count, total_tokens, total_cost, schema_version}",
                Self::session_params(session, params),
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn delete_all_messages(&self, session_id: &str) -> Result<(), SessionError> {
        self.get_session(session_id)?;
        self.delete_messages_from(session_id, 0)?;
        self.set_message_count(session_id, 0)
    }

    pub fn truncate_messages_after(
        &self,
        session_id: &str,
        keep_up_to: u64,
    ) -> Result<(), SessionError> {
        let session = self.get_session(session_id)?;
        let new_count = session.message_count.min(keep_up_to.saturating_add(1));
        let transaction = self.db.multi_transaction(true);
        let result =
            self.truncate_messages_in_transaction(&transaction, &session, keep_up_to, new_count);
        if result.is_ok() {
            transaction.commit().map_err(db_err)?;
        } else {
            let _ = transaction.abort();
        }
        result
    }

    fn truncate_messages_in_transaction(
        &self,
        transaction: &cozo::MultiTransaction,
        session: &super::SessionMetadata,
        keep_up_to: u64,
        new_count: u64,
    ) -> Result<(), SessionError> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session.id.as_str()));
        params.insert(
            "keep".to_string(),
            DataValue::from(clamp_u64_to_i64(keep_up_to)),
        );
        transaction
            .run_script(
                "?[session_id, message_index] := *messages{session_id, message_index}, session_id = $sid, message_index > $keep
                 :rm messages {session_id, message_index}",
                params.clone(),
            )
            .map_err(db_err)?;
        params.insert(
            "count".to_string(),
            DataValue::from(clamp_u64_to_i64(new_count)),
        );
        params.insert(
            "last_active".to_string(),
            DataValue::from(Utc::now().to_rfc3339()),
        );
        transaction
            .run_script(
                "?[id, created_at, last_active, working_directory, git_branch, model, message_count, total_tokens, total_cost, schema_version] <- [[
                    $sid, $created_at, $last_active, $working_directory, $git_branch, $model, $count, $total_tokens, $total_cost, $schema_version
                ]]
                :put sessions {id => created_at, last_active, working_directory, git_branch, model, message_count, total_tokens, total_cost, schema_version}",
                Self::session_params(session, params),
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn load_messages(&self, session_id: &str) -> Result<Vec<String>, SessionError> {
        let session = match self.get_session(session_id) {
            Ok(session) => session,
            Err(SessionError::NotFound(_)) => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));
        params.insert(
            "message_count".to_string(),
            DataValue::from(clamp_u64_to_i64(session.message_count)),
        );
        let result = self
            .db
            .run_script(
                "?[message_index, content] := *messages{session_id, message_index, content}, session_id = $sid, message_index < $message_count :sort message_index",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;
        Ok(result.rows.iter().map(|row| extract_str(&row[1])).collect())
    }

    fn delete_messages_from(&self, session_id: &str, start: u64) -> Result<(), SessionError> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));
        params.insert(
            "start".to_string(),
            DataValue::from(clamp_u64_to_i64(start)),
        );
        self.db
            .run_mutable(
                "?[session_id, message_index] := *messages{session_id, message_index}, session_id = $sid, message_index >= $start
                 :rm messages {session_id, message_index}",
                params,
                "session store: delete messages from index",
            )
            .map_err(db_err)?;
        Ok(())
    }

    fn set_message_count(&self, session_id: &str, message_count: u64) -> Result<(), SessionError> {
        let mut session = self.get_session(session_id)?;
        session.message_count = message_count;
        session.last_active = Utc::now().to_rfc3339();
        self.put_session(&session)
    }

    fn session_params(
        session: &super::SessionMetadata,
        mut params: BTreeMap<String, DataValue>,
    ) -> BTreeMap<String, DataValue> {
        params.insert(
            "created_at".to_string(),
            DataValue::from(session.created_at.as_str()),
        );
        params.insert(
            "working_directory".to_string(),
            DataValue::from(session.working_directory.as_str()),
        );
        params.insert(
            "git_branch".to_string(),
            DataValue::from(session.git_branch.as_deref().unwrap_or("")),
        );
        params.insert("model".to_string(), DataValue::from(session.model.as_str()));
        params.insert(
            "total_tokens".to_string(),
            DataValue::from(clamp_u64_to_i64(session.total_tokens)),
        );
        params.insert(
            "total_cost".to_string(),
            DataValue::from(session.total_cost),
        );
        params.insert(
            "schema_version".to_string(),
            DataValue::from(i64::from(session.schema_version)),
        );
        params
    }
}
