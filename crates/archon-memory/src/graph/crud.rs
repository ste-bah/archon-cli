use std::collections::BTreeMap;

use chrono::Utc;
use cozo::{DataValue, ScriptMutability};
use tracing::{debug, warn};
use uuid::Uuid;

use super::helpers::db_err;
use super::{MemoryGraph, row_to_memory};
use crate::types::{Memory, MemoryError, MemoryType};

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
            if let Err(delete_error) = self.delete_memory(&id) {
                warn!(
                    memory_id = %id,
                    error = %delete_error,
                    "memory.embedding.rollback_failed"
                );
            }
            return Err(error);
        }
        Ok(id)
    }

    /// Retrieve a single memory by id and bump its access stats.
    pub fn get_memory(&self, id: &str) -> Result<Memory, MemoryError> {
        let mem = self.read_memory(id)?;

        // Bump access count and last_accessed.
        let now = Utc::now().to_rfc3339();
        let new_count = mem.access_count + 1;

        let mut params = BTreeMap::new();
        params.insert("id".to_string(), DataValue::from(id));
        params.insert("content".to_string(), DataValue::from(mem.content.as_str()));
        params.insert("title".to_string(), DataValue::from(mem.title.as_str()));
        params.insert(
            "memory_type".to_string(),
            DataValue::from(mem.memory_type.to_string()),
        );
        params.insert("importance".to_string(), DataValue::from(mem.importance));
        params.insert(
            "tags".to_string(),
            DataValue::from(serde_json::to_string(&mem.tags).map_err(MemoryError::from)?),
        );
        params.insert(
            "source_type".to_string(),
            DataValue::from(mem.source_type.as_str()),
        );
        params.insert(
            "project_path".to_string(),
            DataValue::from(mem.project_path.as_str()),
        );
        params.insert(
            "created_at".to_string(),
            DataValue::from(mem.created_at.to_rfc3339()),
        );
        let updated_str = mem.updated_at.map(|dt| dt.to_rfc3339()).unwrap_or_default();
        params.insert("updated_at".to_string(), DataValue::from(updated_str));
        params.insert("new_count".to_string(), DataValue::from(new_count as i64));
        params.insert("now".to_string(), DataValue::from(now.as_str()));

        self.db
            .run_script(
                "?[id, content, title, memory_type, importance, tags,
                  source_type, project_path, created_at, updated_at,
                  access_count, last_accessed] <- [[
                    $id, $content, $title, $memory_type, $importance, $tags,
                    $source_type, $project_path, $created_at, $updated_at,
                    $new_count, $now
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

        Ok(Memory {
            access_count: new_count,
            last_accessed: Some(
                chrono::DateTime::parse_from_rfc3339(&now)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            ),
            ..mem
        })
    }

    /// Update a memory's content and/or tags.
    pub fn update_memory(
        &self,
        id: &str,
        content: Option<&str>,
        tags: Option<&[String]>,
    ) -> Result<(), MemoryError> {
        let mem = self.read_memory(id)?;
        let now = Utc::now().to_rfc3339();

        let new_content = content.unwrap_or(&mem.content);
        let new_tags = if let Some(t) = tags {
            serde_json::to_string(t)?
        } else {
            serde_json::to_string(&mem.tags)?
        };

        let mut params = BTreeMap::new();
        params.insert("id".to_string(), DataValue::from(id));
        params.insert("content".to_string(), DataValue::from(new_content));
        params.insert("title".to_string(), DataValue::from(mem.title.as_str()));
        params.insert(
            "memory_type".to_string(),
            DataValue::from(mem.memory_type.to_string()),
        );
        params.insert("importance".to_string(), DataValue::from(mem.importance));
        params.insert("tags".to_string(), DataValue::from(new_tags));
        params.insert(
            "source_type".to_string(),
            DataValue::from(mem.source_type.as_str()),
        );
        params.insert(
            "project_path".to_string(),
            DataValue::from(mem.project_path.as_str()),
        );
        params.insert(
            "created_at".to_string(),
            DataValue::from(mem.created_at.to_rfc3339()),
        );
        params.insert("updated_at".to_string(), DataValue::from(now));
        params.insert(
            "access_count".to_string(),
            DataValue::from(mem.access_count as i64),
        );
        let la = mem
            .last_accessed
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default();
        params.insert("last_accessed".to_string(), DataValue::from(la));

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

        Ok(())
    }

    /// Update a memory's importance score.
    pub fn update_importance(&self, id: &str, importance: f64) -> Result<(), MemoryError> {
        let mem = self.read_memory(id)?;
        let now = Utc::now().to_rfc3339();

        let mut params = BTreeMap::new();
        params.insert("id".to_string(), DataValue::from(id));
        params.insert("content".to_string(), DataValue::from(mem.content.as_str()));
        params.insert("title".to_string(), DataValue::from(mem.title.as_str()));
        params.insert(
            "memory_type".to_string(),
            DataValue::from(mem.memory_type.to_string()),
        );
        params.insert("importance".to_string(), DataValue::from(importance));
        params.insert(
            "tags".to_string(),
            DataValue::from(serde_json::to_string(&mem.tags).map_err(MemoryError::from)?),
        );
        params.insert(
            "source_type".to_string(),
            DataValue::from(mem.source_type.as_str()),
        );
        params.insert(
            "project_path".to_string(),
            DataValue::from(mem.project_path.as_str()),
        );
        params.insert(
            "created_at".to_string(),
            DataValue::from(mem.created_at.to_rfc3339()),
        );
        params.insert("updated_at".to_string(), DataValue::from(now));
        params.insert(
            "access_count".to_string(),
            DataValue::from(mem.access_count as i64),
        );
        let la = mem
            .last_accessed
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default();
        params.insert("last_accessed".to_string(), DataValue::from(la));

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

        Ok(())
    }

    /// Delete a memory, its relationships, and its embedding (if any).
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

        // Find all relationships involving this memory
        let rels = self
            .db
            .run_script(
                "?[from_id, to_id, rel_type] := *relationships{from_id, to_id, rel_type}, from_id = $id
                 ?[from_id, to_id, rel_type] := *relationships{from_id, to_id, rel_type}, to_id = $id",
                params.clone(),
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;

        // Delete each relationship individually
        for row in &rels.rows {
            let mut rp = BTreeMap::new();
            rp.insert("from_id".to_string(), row[0].clone());
            rp.insert("to_id".to_string(), row[1].clone());
            rp.insert("rel_type".to_string(), row[2].clone());
            self.db
                .run_script(
                    "?[from_id, to_id, rel_type] <- [[$from_id, $to_id, $rel_type]]
                     :rm relationships {from_id, to_id, rel_type}",
                    rp,
                    ScriptMutability::Mutable,
                )
                .map_err(db_err)?;
        }

        // Delete the memory itself
        self.db
            .run_script(
                "?[id] <- [[$id]] :rm memories {id}",
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
