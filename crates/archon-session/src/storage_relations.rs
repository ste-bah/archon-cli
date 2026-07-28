use std::collections::BTreeMap;

use cozo::{DataValue, ScriptMutability};

use super::{SessionError, SessionStore, db_err, extract_str};

impl SessionStore {
    pub fn set_name(&self, session_id: &str, name: &str) -> Result<(), SessionError> {
        let mut params = BTreeMap::new();
        params.insert("session_id".to_string(), DataValue::from(session_id));
        params.insert("name".to_string(), DataValue::from(name));
        self.db
            .run_script(
                "?[session_id, name] <- [[$session_id, $name]] :put session_names {session_id => name}",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn get_name(&self, session_id: &str) -> Result<Option<String>, SessionError> {
        self.get_optional_relation_value(
            session_id,
            "?[name] := *session_names{session_id, name}, session_id = $sid",
        )
    }

    pub fn find_sessions_by_name_prefix(
        &self,
        prefix: &str,
    ) -> Result<Vec<(String, String)>, SessionError> {
        let result = self
            .db
            .run_script(
                "?[session_id, name] := *session_names{session_id, name}",
                BTreeMap::new(),
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;
        Ok(result
            .rows
            .iter()
            .filter_map(|row| {
                let name = extract_str(&row[1]);
                name.starts_with(prefix)
                    .then(|| (extract_str(&row[0]), name))
            })
            .collect())
    }

    pub fn set_parent(&self, session_id: &str, parent_id: &str) -> Result<(), SessionError> {
        let mut params = BTreeMap::new();
        params.insert("session_id".to_string(), DataValue::from(session_id));
        params.insert("parent_session_id".to_string(), DataValue::from(parent_id));
        self.db
            .run_script(
                "?[session_id, parent_session_id] <- [[$session_id, $parent_session_id]] :put session_parents {session_id => parent_session_id}",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn get_parent(&self, session_id: &str) -> Result<Option<String>, SessionError> {
        self.get_optional_relation_value(
            session_id,
            "?[parent_session_id] := *session_parents{session_id, parent_session_id}, session_id = $sid",
        )
    }

    pub fn put_tag(&self, session_id: &str, tag: &str) -> Result<(), SessionError> {
        let mut params = BTreeMap::new();
        params.insert("session_id".to_string(), DataValue::from(session_id));
        params.insert("tag".to_string(), DataValue::from(tag));
        self.db
            .run_script(
                "?[session_id, tag] <- [[$session_id, $tag]] :put session_tags {session_id, tag}",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn delete_tag(&self, session_id: &str, tag: &str) -> Result<(), SessionError> {
        let mut params = BTreeMap::new();
        params.insert("session_id".to_string(), DataValue::from(session_id));
        params.insert("tag".to_string(), DataValue::from(tag));
        self.db
            .run_script(
                "?[session_id, tag] <- [[$session_id, $tag]] :rm session_tags {session_id, tag}",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn list_tags(&self, session_id: &str) -> Result<Vec<String>, SessionError> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));
        let result = self
            .db
            .run_script(
                "?[tag] := *session_tags{session_id, tag}, session_id = $sid :sort tag",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;
        Ok(result.rows.iter().map(|row| extract_str(&row[0])).collect())
    }

    fn get_optional_relation_value(
        &self,
        session_id: &str,
        script: &str,
    ) -> Result<Option<String>, SessionError> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));
        let result = self
            .db
            .run_script(script, params, ScriptMutability::Immutable)
            .map_err(db_err)?;
        Ok(result.rows.first().and_then(|row| {
            let value = extract_str(&row[0]);
            (!value.is_empty()).then_some(value)
        }))
    }
}
