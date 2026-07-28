use std::collections::BTreeMap;

use cozo::ScriptMutability;

use super::{SessionError, SessionStore, db_err, extract_str};

impl SessionStore {
    pub fn list_sessions(&self, limit: u32) -> Result<Vec<super::SessionMetadata>, SessionError> {
        let sessions = self.run_listing_script(limit)?;
        sessions
            .rows
            .iter()
            .map(|row| {
                let mut metadata = self.metadata_from_row(row);
                metadata.name = optional_string(&row[10]);
                metadata.parent_session_id = optional_string(&row[11]);
                Ok(metadata)
            })
            .collect()
    }

    fn run_listing_script(&self, limit: u32) -> Result<cozo::NamedRows, SessionError> {
        self.db
            .run_script(
                &format!(
                    "name[id, name] := *session_names{{session_id: id, name}}
                    name[id, name] := *sessions{{id}}, not *session_names{{session_id: id}}, name = null
                    parent[id, parent_session_id] := *session_parents{{session_id: id, parent_session_id}}
                    parent[id, parent_session_id] := *sessions{{id}}, not *session_parents{{session_id: id}}, parent_session_id = null
                    ?[id, created_at, last_active, working_directory, git_branch, model, message_count, total_tokens, total_cost, schema_version, name, parent_session_id] :=
                        *sessions{{id, created_at, last_active, working_directory, git_branch, model, message_count, total_tokens, total_cost, schema_version}},
                        name[id, name],
                        parent[id, parent_session_id]
                    :sort -last_active
                    :limit {limit}"
                ),
                BTreeMap::new(),
                ScriptMutability::Immutable,
            )
            .map_err(db_err)
    }
}

fn optional_string(value: &cozo::DataValue) -> Option<String> {
    match value {
        cozo::DataValue::Null => None,
        value => Some(extract_str(value)),
    }
}
