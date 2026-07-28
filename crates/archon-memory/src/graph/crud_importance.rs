use std::collections::BTreeMap;

use chrono::Utc;
use cozo::{DataValue, ScriptMutability};

use super::super::{MemoryGraph, row_values_to_memory};
use super::db_err;
use crate::types::{Memory, MemoryError};

impl MemoryGraph {
    /// Atomically compare the authoritative importance to a prior snapshot,
    /// replace the current trend tag, and return the reconciled row.
    pub fn reconcile_importance_trend(
        &self,
        id: &str,
        previous_importance: f64,
    ) -> Result<Memory, MemoryError> {
        if !previous_importance.is_finite() {
            return Err(MemoryError::Database(
                "previous importance must be finite".to_string(),
            ));
        }

        let now = Utc::now().to_rfc3339();
        let params = BTreeMap::from([
            ("id".to_string(), DataValue::from(id)),
            (
                "previous_importance".to_string(),
                DataValue::from(previous_importance),
            ),
            ("now".to_string(), DataValue::from(now.as_str())),
        ]);
        let result = self
            .db
            .run_script(
                "{
                    parsed_tags[id, parsed] :=
                        *memories{id, tags: stored_tags}, id = $id,
                        parsed = parse_json(stored_tags);
                    tag_capacity_valid[id, valid] :=
                        parsed_tags[id, parsed], valid = assert(is_null(maybe_get(parsed, 17)));
                    retained_tags[id, collect(tag)] :=
                        parsed_tags[id, parsed], tag_capacity_valid[id, _], index in int_range(17),
                        tag = maybe_get(parsed, index), tag != null,
                        not starts_with(tag, 'trend:');
                    retained_capacity_valid[id, valid] :=
                        retained_tags[id, tags], valid = assert(length(tags) <= 16);
                    has_retained_tags[id] := retained_tags[id, _];
                    tags_to_write[id, tags] :=
                        retained_tags[id, tags], retained_capacity_valid[id, _];
                    tags_to_write[id, tags] :=
                        parsed_tags[id, _], not has_retained_tags[id], tags = [];
                    ?[id, content, title, memory_type, importance, tags, source_type,
                        project_path, created_at, updated_at, access_count, last_accessed] :=
                        *memories{id, content, title, memory_type, importance,
                            tags: stored_tags, source_type, project_path, created_at,
                            updated_at: current_updated_at, access_count, last_accessed},
                        id = $id, tags_to_write[id, retained],
                        trend = if(importance > $previous_importance, 'trend:rising',
                            if(importance < $previous_importance, 'trend:declining', 'trend:stable')),
                        tags = dump_json(json(append(retained, trend))), updated_at = $now;
                    :put memories { id => content, title, memory_type, importance, tags, source_type,
                        project_path, created_at, updated_at, access_count, last_accessed }
                }
                {
                    ?[id, content, title, memory_type, importance, tags, source_type,
                        project_path, created_at, updated_at, access_count, last_accessed] :=
                        *memories{id, content, title, memory_type, importance, tags, source_type,
                            project_path, created_at, updated_at, access_count, last_accessed},
                        id = $id
                } as _result
                %return _result",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;
        let row = result
            .rows
            .first()
            .ok_or_else(|| MemoryError::NotFound(id.to_string()))?;
        row_values_to_memory(row)
    }

    /// Return whether an immutable provenance record confirms this memory's
    /// importance delta was applied.
    pub fn has_importance_application(
        &self,
        memory_id: &str,
        provenance_id: &str,
    ) -> Result<bool, MemoryError> {
        if provenance_id.is_empty() {
            return Err(MemoryError::Database(
                "provenance_id must not be empty".to_string(),
            ));
        }
        let params = BTreeMap::from([
            ("memory_id".to_string(), DataValue::from(memory_id)),
            ("provenance_id".to_string(), DataValue::from(provenance_id)),
        ]);
        let result = self
            .db
            .run_script(
                "?[memory_id] := *score_applications{memory_id, provenance_id},
                    memory_id = $memory_id, provenance_id = $provenance_id",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;
        Ok(!result.rows.is_empty())
    }

    /// Apply an idempotent importance delta and replace the current trend tag.
    pub fn apply_importance_delta(
        &self,
        id: &str,
        delta: f64,
        provenance_id: &str,
    ) -> Result<Memory, MemoryError> {
        if !delta.is_finite() {
            return Err(MemoryError::Database("delta must be finite".to_string()));
        }
        if provenance_id.is_empty() {
            return Err(MemoryError::Database(
                "provenance_id must not be empty".to_string(),
            ));
        }

        let now = Utc::now().to_rfc3339();
        let mut params = BTreeMap::new();
        params.insert("id".to_string(), DataValue::from(id));
        params.insert("provenance_id".to_string(), DataValue::from(provenance_id));
        params.insert("delta".to_string(), DataValue::from(delta));
        params.insert("now".to_string(), DataValue::from(now.as_str()));

        let result = self
            .db
            .run_script(
                "{
                    ?[memory_id] := *score_applications{memory_id, provenance_id},
                        memory_id = $id, provenance_id = $provenance_id
                } as _already_applied
                %if_not _already_applied
                %then
                    {
                        parsed_tags[id, parsed] :=
                            *memories{id, tags: stored_tags}, id = $id,
                            parsed = parse_json(stored_tags);
                        tag_capacity_valid[id, valid] :=
                            parsed_tags[id, parsed], valid = assert(is_null(maybe_get(parsed, 17)));
                        retained_tags[id, collect(tag)] :=
                            parsed_tags[id, parsed], tag_capacity_valid[id, _], index in int_range(17),
                            tag = maybe_get(parsed, index), tag != null,
                            not starts_with(tag, 'trend:');
                        retained_capacity_valid[id, valid] :=
                            retained_tags[id, tags], valid = assert(length(tags) <= 16);
                        has_retained_tags[id] := retained_tags[id, _];
                        tags_to_write[id, tags] :=
                            retained_tags[id, tags], retained_capacity_valid[id, _];
                        tags_to_write[id, tags] :=
                            parsed_tags[id, _], not has_retained_tags[id], tags = [];
                        ?[id, content, title, memory_type, importance, tags, source_type,
                            project_path, created_at, updated_at, access_count, last_accessed] :=
                            *memories{id, content, title, memory_type, importance: current_importance,
                                tags: stored_tags, source_type, project_path, created_at,
                                updated_at: current_updated_at, access_count, last_accessed},
                            id = $id, tags_to_write[id, retained],
                            next_importance = min(100.0, max(0.0, current_importance + $delta)),
                            importance = next_importance,
                            trend = if(next_importance > current_importance, 'trend:rising',
                                if(next_importance < current_importance, 'trend:declining', 'trend:stable')),
                            tags = dump_json(json(append(retained, trend))), updated_at = $now;
                        :put memories { id => content, title, memory_type, importance, tags, source_type,
                            project_path, created_at, updated_at, access_count, last_accessed }
                    }
                    {
                        has_memory[id] := *memories{id}, id = $id;
                        ?[memory_id, provenance_id, applied_at] :=
                            has_memory[id],
                            memory_id = $id, provenance_id = $provenance_id, applied_at = $now;
                        :put score_applications { memory_id, provenance_id => applied_at }
                    }
                %end
                {
                    ?[id, content, title, memory_type, importance, tags, source_type,
                        project_path, created_at, updated_at, access_count, last_accessed] :=
                        *memories{id, content, title, memory_type, importance, tags, source_type,
                            project_path, created_at, updated_at, access_count, last_accessed},
                        id = $id
                } as _result
                %return _result",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        let row = result
            .rows
            .first()
            .ok_or_else(|| MemoryError::NotFound(id.to_string()))?;
        row_values_to_memory(row)
    }
}
