use std::collections::BTreeMap;

use chrono::Utc;
use cozo::{DataValue, ScriptMutability};
use tracing::{debug, warn};
use uuid::Uuid;

use super::helpers::db_err;
use super::{MemoryGraph, row_to_memory, row_values_to_memory};
use crate::types::{Memory, MemoryError, MemoryType, StoreMemoryOutcome};

const MAX_NON_TREND_TAGS: usize = 16;
const MAX_TREND_TAGS: usize = 1;

fn validate_tags(tags: &[String]) -> Result<(), MemoryError> {
    let trend_count = tags.iter().filter(|tag| tag.starts_with("trend:")).count();
    let non_trend_count = tags.len() - trend_count;
    if non_trend_count > MAX_NON_TREND_TAGS || trend_count > MAX_TREND_TAGS {
        return Err(MemoryError::Database(format!(
            "a memory may have at most {MAX_NON_TREND_TAGS} non-trend tags and {MAX_TREND_TAGS} trend tag"
        )));
    }
    Ok(())
}

impl MemoryGraph {
    // -- CRUD --------------------------------------------------

    /// Store a new memory and return its UUID.
    #[allow(clippy::too_many_arguments)]
    pub fn store_memory(
        &self,
        content: &str,
        title: &str,
        memory_type: MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<String, MemoryError> {
        validate_tags(tags)?;
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let tags_json = serde_json::to_string(tags)?;

        let mut params = BTreeMap::new();
        params.insert("id".to_string(), DataValue::from(id.as_str()));
        params.insert("content".to_string(), DataValue::from(content));
        params.insert("title".to_string(), DataValue::from(title));
        params.insert(
            "memory_type".to_string(),
            DataValue::from(memory_type.to_string()),
        );
        params.insert("importance".to_string(), DataValue::from(importance));
        params.insert("tags".to_string(), DataValue::from(tags_json.as_str()));
        params.insert("source_type".to_string(), DataValue::from(source_type));
        params.insert("project_path".to_string(), DataValue::from(project_path));
        params.insert("created_at".to_string(), DataValue::from(now.as_str()));
        params.insert("updated_at".to_string(), DataValue::from(""));
        params.insert("access_count".to_string(), DataValue::from(0i64));
        params.insert("last_accessed".to_string(), DataValue::from(""));

        self.db
            .run_script(
                "?[id, content, title, memory_type, importance, tags,
                  source_type, project_path, created_at, updated_at,
                  access_count, last_accessed] <- [[
                    $id, $content, $title, $memory_type, $importance, $tags,
                    $source_type, $project_path, $created_at, $updated_at,
                    $access_count, $last_accessed
                ]]
                :put memories {
                    id => content, title, memory_type, importance, tags,
                    source_type, project_path, created_at, updated_at,
                    access_count, last_accessed
                }",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        debug!(id = %id, "stored memory");
        if let Err(error) = self.embed_and_store(&id, content) {
            warn!(memory_id = %id, error = %error, "memory.embedding.store_failed");
        }
        Ok(id)
    }

    /// Create a memory with a caller-selected ID, or return the authoritative
    /// existing row after verifying its immutable identity fields.
    #[allow(clippy::too_many_arguments)]
    pub fn store_memory_with_id(
        &self,
        id: &str,
        content: &str,
        title: &str,
        memory_type: MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<Memory, MemoryError> {
        self.store_memory_with_id_outcome(
            id,
            content,
            title,
            memory_type,
            importance,
            tags,
            source_type,
            project_path,
        )
        .map(|outcome| outcome.memory)
    }

    /// Create a memory with a caller-selected ID and return whether this call
    /// created it in the same authoritative database transaction.
    #[allow(clippy::too_many_arguments)]
    pub fn store_memory_with_id_outcome(
        &self,
        id: &str,
        content: &str,
        title: &str,
        memory_type: MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<StoreMemoryOutcome, MemoryError> {
        validate_tags(tags)?;

        let now = Utc::now().to_rfc3339();
        let tags_json = serde_json::to_string(tags)?;
        let params = BTreeMap::from([
            ("id".to_string(), DataValue::from(id)),
            ("content".to_string(), DataValue::from(content)),
            ("title".to_string(), DataValue::from(title)),
            (
                "memory_type".to_string(),
                DataValue::from(memory_type.to_string()),
            ),
            ("importance".to_string(), DataValue::from(importance)),
            ("tags".to_string(), DataValue::from(tags_json.as_str())),
            ("source_type".to_string(), DataValue::from(source_type)),
            ("project_path".to_string(), DataValue::from(project_path)),
            ("created_at".to_string(), DataValue::from(now.as_str())),
            ("updated_at".to_string(), DataValue::from("")),
            ("access_count".to_string(), DataValue::from(0i64)),
            ("last_accessed".to_string(), DataValue::from("")),
        ]);
        let result = self
            .db
            .run_script(
                "{
                    ?[id, content, title, memory_type, importance, tags, source_type,
                        project_path, created_at, updated_at, access_count, last_accessed, created] :=
                        *memories{id, content, title, memory_type, importance, tags, source_type,
                            project_path, created_at, updated_at, access_count, last_accessed},
                        id = $id,
                        created = false
                } as _existing
                %if _existing
                %then %return _existing
                %end
                {
                    ?[id, content, title, memory_type, importance, tags, source_type,
                        project_path, created_at, updated_at, access_count, last_accessed] <- [[
                        $id, $content, $title, $memory_type, $importance, $tags,
                        $source_type, $project_path, $created_at, $updated_at,
                        $access_count, $last_accessed
                    ]]
                    :put memories {
                        id => content, title, memory_type, importance, tags, source_type,
                        project_path, created_at, updated_at, access_count, last_accessed
                    }
                }
                {
                    ?[id, content, title, memory_type, importance, tags, source_type,
                        project_path, created_at, updated_at, access_count, last_accessed, created] :=
                        *memories{id, content, title, memory_type, importance, tags, source_type,
                            project_path, created_at, updated_at, access_count, last_accessed},
                        id = $id,
                        created = true
                } as _created
                %return _created",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;
        let row = result
            .rows
            .first()
            .ok_or_else(|| MemoryError::NotFound(id.to_string()))?;
        let authoritative = row_values_to_memory(row)?;
        let created = row.get(12).and_then(DataValue::get_bool).ok_or_else(|| {
            MemoryError::Database("missing explicit-ID creation outcome".to_string())
        })?;
        if authoritative.memory_type != memory_type
            || authoritative.content != content
            || authoritative.source_type != source_type
        {
            return Err(MemoryError::Database(format!(
                "memory ID collision for {id}: existing type, content, or source differs"
            )));
        }
        if let Err(error) = self.embed_and_store(id, content) {
            warn!(memory_id = %id, error = %error, "memory.embedding.store_failed");
        }
        Ok(StoreMemoryOutcome {
            memory: authoritative,
            created,
        })
    }

    /// Retrieve a single memory and atomically bump its access stats.
    pub fn get_memory(&self, id: &str) -> Result<Memory, MemoryError> {
        let now = Utc::now().to_rfc3339();
        let params = BTreeMap::from([
            ("id".to_string(), DataValue::from(id)),
            ("now".to_string(), DataValue::from(now.as_str())),
        ]);
        let result = self
            .db
            .run_script(
                "{
                    ?[id, content, title, memory_type, importance, tags, source_type,
                        project_path, created_at, updated_at, access_count, last_accessed] :=
                        *memories{id, content, title, memory_type, importance, tags, source_type,
                            project_path, created_at, updated_at, access_count: current_access_count,
                            last_accessed: current_last_accessed},
                        id = $id,
                        access_count = current_access_count + 1,
                        last_accessed = $now;
                    :put memories { id => content, title, memory_type, importance, tags, source_type,
                        project_path, created_at, updated_at, access_count, last_accessed }
                }
                {
                    ?[id, content, title, memory_type, importance, tags, source_type,
                        project_path, created_at, updated_at, access_count, last_accessed] :=
                        *memories{id, content, title, memory_type, importance, tags, source_type,
                            project_path, created_at, updated_at, access_count, last_accessed},
                        id = $id
                }",
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

    /// Update a memory's content and/or tags.
    pub fn update_memory(
        &self,
        id: &str,
        content: Option<&str>,
        tags: Option<&[String]>,
    ) -> Result<(), MemoryError> {
        if let Some(tags) = tags {
            validate_tags(tags)?;
        }
        let now = Utc::now().to_rfc3339();
        let tags_json = tags.map(serde_json::to_string).transpose()?;
        let mut params = BTreeMap::new();
        params.insert("id".to_string(), DataValue::from(id));
        params.insert(
            "content".to_string(),
            DataValue::from(content.unwrap_or_default()),
        );
        params.insert(
            "replace_content".to_string(),
            DataValue::from(content.is_some()),
        );
        params.insert(
            "tags".to_string(),
            DataValue::from(tags_json.as_deref().unwrap_or_default()),
        );
        params.insert("replace_tags".to_string(), DataValue::from(tags.is_some()));
        params.insert("now".to_string(), DataValue::from(now.as_str()));

        let result = self
            .db
            .run_script(
                "{
                    ?[id] := *memories{id}, id = $id
                } as _current
                %if _current
                %then
                    {
                        ?[id, content, title, memory_type, importance, tags, source_type,
                            project_path, created_at, updated_at, access_count, last_accessed] :=
                            *memories{id, content: current_content, title, memory_type, importance,
                                tags: current_tags, source_type, project_path, created_at,
                                access_count, last_accessed},
                            id = $id,
                            content = if($replace_content, $content, current_content),
                            tags = if($replace_tags, $tags, current_tags),
                            updated_at = $now;
                        :put memories { id => content, title, memory_type, importance, tags, source_type,
                            project_path, created_at, updated_at, access_count, last_accessed }
                    }
                %end
                %return _current",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        if result.rows.is_empty() {
            return Err(MemoryError::NotFound(id.to_string()));
        }
        if content.is_some()
            && let Err(error) = self.embed_and_store(id, content.unwrap_or_default())
        {
            warn!(memory_id = %id, error = %error, "memory.embedding.store_failed");
        }

        Ok(())
    }

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

    pub fn delete_memory(&self, id: &str) -> Result<(), MemoryError> {
        // Check it exists first
        self.read_memory(id)?;

        // Delete the embedding if vector search is initialised
        let has_provider = self
            .embedding_provider
            .read()
            .map(|g| g.is_some())
            .unwrap_or(false);
        if has_provider && let Err(e) = crate::vector_search::delete_embedding(&self.db, id) {
            tracing::warn!(id, "failed to delete embedding: {e}");
        }

        let mut params = BTreeMap::new();
        params.insert("id".to_string(), DataValue::from(id));

        self.db
            .run_script(
                "{
                    ?[from_id, to_id, rel_type] :=
                        *relationships{from_id, to_id, rel_type}, from_id = $id
                    ?[from_id, to_id, rel_type] :=
                        *relationships{from_id, to_id, rel_type}, to_id = $id
                    :rm relationships {from_id, to_id, rel_type}
                }
                {
                    ?[memory_id, provenance_id] :=
                        *score_applications{memory_id, provenance_id}, memory_id = $id
                    :rm score_applications {memory_id, provenance_id}
                }
                {
                    ?[id] <- [[$id]]
                    :rm memories {id}
                }",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        Ok(())
    }

    // -- internal read helper ---------------------------------

    /// Read a single memory by id without bumping access stats.
    pub(crate) fn read_memory(&self, id: &str) -> Result<Memory, MemoryError> {
        row_to_memory(&self.db, id)
    }
}
