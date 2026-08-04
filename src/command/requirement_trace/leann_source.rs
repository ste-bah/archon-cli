//! The `archon-leann` adapter for [`CodeSearch`], and the reason it never
//! writes.
//!
//! # Indexing runs out of band. This is where that is enforced.
//!
//! `archon-leann`'s `replace_file_with_cancel` and `remove_file` hold the Cozo
//! write lock across an entire `multi_transaction` — the longest critical
//! section in the repository. A report that indexed would serialize every other
//! writer in the process for the duration, and the guarded retry budget parks a
//! contending thread for up to ~19 seconds.
//!
//! So this adapter opens the index and constructs [`Search`] directly, and
//! deliberately does **not** go through `CodeIndex::new`, which calls
//! `ensure_schema()` — a write. If the index has never been built, the schema is
//! absent, the query fails, and that failure is reported as a named gap telling
//! the operator to index out of band. Building it silently would be the
//! contention this design exists to avoid.
//!
//! `Search::new` needs an `EmbeddingProvider`, which is why indexing is a
//! genuine one-off cost and why the port exists at all: no test in
//! `archon-knowledge` needs one.

use std::path::Path;

use anyhow::{Result, anyhow};
use archon_knowledge::errors::KnowledgeError;
use archon_knowledge::traceability::{CodeHit, CodeSearch};
use archon_leann::search::Search;
use archon_memory::embedding::{EmbeddingConfig, create_provider};

/// A read-only view of an already-built code index.
pub(super) struct LeannCodeSearch {
    search: Search,
}

impl LeannCodeSearch {
    /// Open an existing index. Never creates one, never writes.
    ///
    /// Errors when the database file is absent, so the caller can report "no
    /// code index" as a gap rather than as a crash.
    pub(super) fn open(db_path: &Path, embedding: EmbeddingConfig) -> Result<Self> {
        if !db_path.exists() {
            return Err(anyhow!(
                "no code index at {}; build it out of band before tracing — \
                 indexing holds the Cozo write lock across a whole multi_transaction \
                 and must never run inside a report",
                db_path.display()
            ));
        }
        let guard = archon_cozo::CozoGuardConfig::for_db_path(db_path);
        let db = archon_cozo::open_sqlite_guarded(
            db_path.to_string_lossy().as_ref(),
            "open leann code index for requirement trace (read-only)",
            &guard,
        )
        .map_err(|e| anyhow!("opening code index at {}: {e}", db_path.display()))?;
        let embedder = create_provider(&embedding)
            .map_err(|e| anyhow!("embedding provider unavailable for requirement trace: {e}"))?;
        Ok(Self::with_embedder(db, embedder))
    }

    /// Wrap an already-open index and an already-chosen embedder.
    ///
    /// Exists so a test can drive the real adapter — the real
    /// `search_with_filter`, the real `SearchResult` → [`CodeHit`] mapping —
    /// against fixture chunks and a constant embedder. Without it the only way
    /// to exercise this seam would be a live index and a live provider, which
    /// means either a network call or a model download on every test run.
    pub(super) fn with_embedder(
        db: cozo::DbInstance,
        embedder: std::sync::Arc<dyn archon_memory::embedding::EmbeddingProvider>,
    ) -> Self {
        Self {
            search: Search::new(db, embedder),
        }
    }
}

impl CodeSearch for LeannCodeSearch {
    fn search(
        &self,
        query: &str,
        limit: usize,
        path_pattern: Option<&str>,
    ) -> std::result::Result<Vec<CodeHit>, KnowledgeError> {
        // `language: None` — a requirement may be satisfied in any language the
        // indexer understands, and narrowing by language here would silently
        // drop anchors in a polyglot tree.
        let hits = self
            .search
            .search_with_filter(query, limit, None, path_pattern)
            .map_err(|e| KnowledgeError::Traceability(format!("code index query failed: {e}")))?;
        Ok(hits
            .into_iter()
            .map(|hit| CodeHit {
                file_path: hit.file_path.to_string_lossy().replace('\\', "/"),
                language: hit.language,
                line_start: hit.line_start,
                line_end: hit.line_end,
                relevance_score: hit.relevance_score,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests;
