use cozo::ScriptMutability;

use crate::search;
use crate::types::{Memory, MemoryError, SearchFilter};

use super::helpers::db_err;
use super::{MemoryGraph, raw_to_memory, read_all_memories};

impl MemoryGraph {
    // -- search / recall ---------------------------------------

    /// Recall memories by relevance.
    ///
    /// When an embedding provider is attached, uses hybrid (keyword + vector)
    /// search. Otherwise falls back to keyword-only search.
    pub fn recall_memories(&self, query: &str, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        let provider = self
            .embedding_provider
            .read()
            .map_err(|e| MemoryError::Database(format!("embedding provider lock poisoned: {e}")))?;
        if let Some(ref provider) = *provider {
            crate::hybrid_search::hybrid_search(
                &self.db,
                query,
                provider.as_ref(),
                self.read_hybrid_alpha(),
                limit,
            )
        } else {
            search::recall(&self.db, query, limit)
        }
    }

    /// Structured search with filters.
    pub fn search_memories(&self, filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
        search::search(&self.db, filter)
    }

    /// List the most recently created memories (up to `limit`).
    pub fn list_recent(&self, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        let all = read_all_memories(&self.db)?;
        let mut memories: Vec<Memory> = all
            .into_iter()
            .filter_map(|raw| raw_to_memory(raw).ok())
            .collect();
        // Sort descending by created_at (newest first)
        memories.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        memories.truncate(limit);
        Ok(memories)
    }

    /// Return the total number of memories in the graph.
    pub fn memory_count(&self) -> Result<usize, MemoryError> {
        let result = self
            .db
            .run_script(
                "?[count(id)] := *memories{id}",
                Default::default(),
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;
        let count = result
            .rows
            .first()
            .and_then(|row| row[0].get_int())
            .unwrap_or(0);
        Ok(count as usize)
    }

    /// Delete all memories and relationships from the graph.
    pub fn clear_all(&self) -> Result<usize, MemoryError> {
        let count = self.memory_count()?;
        self.db
            .run_script(
                "?[id, content, title, memory_type, importance, tags,
                  source_type, project_path, created_at, updated_at,
                  access_count, last_accessed] :=
                    *memories{id, content, title, memory_type, importance, tags,
                              source_type, project_path, created_at, updated_at,
                              access_count, last_accessed}
                :rm memories {
                    id => content, title, memory_type, importance, tags,
                    source_type, project_path, created_at, updated_at,
                    access_count, last_accessed
                }",
                Default::default(),
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;
        self.db
            .run_script(
                "?[from_id, to_id, rel_type, context, strength, created_at] :=
                    *relationships{from_id, to_id, rel_type, context, strength, created_at}
                :rm relationships {
                    from_id, to_id, rel_type => context, strength, created_at
                }",
                Default::default(),
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;
        Ok(count)
    }
}
