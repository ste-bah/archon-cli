use std::collections::BTreeMap;

use cozo::{DataValue, ScriptMutability};

use super::{SessionError, SessionStore, db_err, extract_str};

impl SessionStore {
    pub fn list_sessions(&self, limit: u32) -> Result<Vec<super::SessionMetadata>, SessionError> {
        #[cfg(test)]
        self.list_query_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let sessions = self
            .db
            .run_script(
                &format!(
                    "?[id, created_at, last_active, working_directory, git_branch, model, message_count, total_tokens, total_cost, schema_version] :=
                        *sessions{{id, created_at, last_active, working_directory, git_branch, model, message_count, total_tokens, total_cost, schema_version}}
                    :sort -last_active
                    :limit {limit}"
                ),
                BTreeMap::new(),
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;
        let ids = DataValue::List(
            sessions
                .rows
                .iter()
                .map(|row| DataValue::from(extract_str(&row[0])))
                .collect(),
        );
        let names = self.bulk_optional_values("session_names", "name", &ids)?;
        let parents = self.bulk_optional_values("session_parents", "parent_session_id", &ids)?;
        sessions
            .rows
            .iter()
            .map(|row| {
                let mut metadata = self.metadata_from_row(row);
                metadata.name = names.get(&metadata.id).cloned();
                metadata.parent_session_id = parents.get(&metadata.id).cloned();
                Ok(metadata)
            })
            .collect()
    }

    fn bulk_optional_values(
        &self,
        relation: &str,
        field: &str,
        ids: &DataValue,
    ) -> Result<BTreeMap<String, String>, SessionError> {
        #[cfg(test)]
        self.list_query_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut params = BTreeMap::new();
        params.insert("ids".to_string(), ids.clone());
        let result = self
            .db
            .run_script(
                &format!(
                    "?[session_id, value] := *{relation}{{session_id, {field}: value}}, session_id in $ids"
                ),
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;
        Ok(result
            .rows
            .iter()
            .map(|row| (extract_str(&row[0]), extract_str(&row[1])))
            .collect())
    }
}
