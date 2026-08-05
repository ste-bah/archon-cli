use cozo::ScriptMutability;

use crate::search;
use crate::types::{Memory, MemoryError, SearchFilter, is_superseded};

/// Remove memories folded into another by consolidation.
///
/// Applied at every read path rather than at the storage layer, so the rows stay
/// on disk and remain reachable by id -- that is what makes a merge reversible,
/// and it is why consolidation can mark rather than delete. `get_memory` and
/// `inspect_memory` deliberately do NOT filter: someone asking for a specific id
/// is entitled to what is there, including the superseded history a `Supersedes`
/// edge points at.
fn drop_superseded(memories: Vec<Memory>) -> Vec<Memory> {
    memories
        .into_iter()
        .filter(|memory| !is_superseded(&memory.tags))
        .collect()
}

use super::helpers::{db_err, run_mutable};
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
        let results = if let Some(ref provider) = *provider {
            crate::hybrid_search::hybrid_search(
                &self.db,
                query,
                provider.as_ref(),
                self.read_hybrid_alpha(),
                limit,
            )
        } else {
            search::recall(&self.db, query, limit)
        };
        Ok(drop_superseded(results?))
    }

    /// Structured search with filters.
    ///
    /// Honours [`SearchFilter::limit`]. `None` keeps the historical unbounded
    /// contract; `Some(n)` returns at most `n` rows, newest first.
    ///
    /// The bound is enforced here as well as inside the candidate query so the
    /// public contract holds for every backend: the FTS `k` bound prunes what
    /// the database reads, and this truncation pins what the caller can observe
    /// even on the full-scan fallback path (no FTS index) where there is no `k`
    /// to push down.
    pub fn search_memories(&self, filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
        // Superseded rows are dropped before the limit is applied, so a page of
        // results is never silently shortened by memories the caller cannot see.
        let mut results = drop_superseded(search::search(&self.db, filter)?);
        if let Some(limit) = filter.limit {
            results.truncate(limit);
        }
        Ok(results)
    }

    /// List the most recently created memories (up to `limit`).
    pub fn list_recent(&self, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        let all = read_all_memories(&self.db)?;
        let mut memories: Vec<Memory> = drop_superseded(
            all.into_iter()
                .filter_map(|raw| raw_to_memory(raw).ok())
                .collect(),
        );
        // Sort descending by created_at (newest first)
        memories.sort_by_key(|b| std::cmp::Reverse(b.created_at));
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
        run_mutable(
            &self.db,
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
            "memory graph: clear all memories",
        )?;
        run_mutable(
            &self.db,
            "?[from_id, to_id, rel_type, context, strength, created_at] :=
                    *relationships{from_id, to_id, rel_type, context, strength, created_at}
                :rm relationships {
                    from_id, to_id, rel_type => context, strength, created_at
                }",
            Default::default(),
            "memory graph: clear all relationships",
        )?;
        Ok(count)
    }
}
