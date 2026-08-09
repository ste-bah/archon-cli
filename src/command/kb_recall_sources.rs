//! The command-layer half of R7: real stores behind `StoreRecordSource`.
//!
//! `archon-knowledge` owns the merge rules and the shape of a hit; it does not
//! own an edge onto `archon-memory`, `archon-docs` or `archon-leann`. Each of
//! those crates drags in `tokio`, `fastembed`, RocksDB, `lopdf` or tree-sitter,
//! and a recall test that had to open one of them could not run without a model
//! on disk. So the wiring lives here, next to the CLI that already opens these
//! stores, exactly like [`crate::command::requirement_trace`]'s
//! `LeannCodeSearch` does for the traceability port.
//!
//! Each implementation does one thing: run the store's own query and translate
//! its own result type. Ranking, provenance vocabulary, quotas and scoring stay
//! in `archon-knowledge` so they cannot drift per store.

use std::sync::Arc;

use archon_knowledge::errors::{KnowledgeError, Result};
use archon_knowledge::recall::adapters::{StoreRecord, StoreRecordSource};
use cozo::DbInstance;

/// `archon-memory`'s recall, which reports order and no score.
///
/// `MemoryGraph::recall_memories` re-ranks internally and then returns a plain
/// `Vec<Memory>` — the relevance value never leaves the crate. That is the
/// single strongest reason the unified score is rank-derived: there is no memory
/// score to fuse, so any score-based fusion would have to invent one.
pub(crate) struct MemoryStore {
    graph: archon_memory::MemoryGraph,
}

impl MemoryStore {
    /// Open the user's default memory graph. Read-only use only.
    pub(crate) fn open_default() -> anyhow::Result<Self> {
        let graph = archon_memory::MemoryGraph::open_default()
            .map_err(|error| anyhow::anyhow!("open memory graph: {error}"))?;
        Ok(Self::new(graph))
    }

    /// Wrap an already-open graph, so a test can drive the real
    /// `recall_memories` → [`StoreRecord`] mapping against an in-memory store.
    pub(crate) fn new(graph: archon_memory::MemoryGraph) -> Self {
        Self { graph }
    }
}

impl StoreRecordSource for MemoryStore {
    fn search(&self, text: &str, limit: usize) -> Result<Vec<StoreRecord>> {
        let memories = self
            .graph
            .recall_memories(text, limit)
            .map_err(|error| KnowledgeError::Store(format!("memory recall failed: {error}")))?;
        Ok(memories
            .into_iter()
            .map(|memory| {
                // `project_path` is carried as the container so the adapter can
                // decide what counts as provenance; it deliberately does not
                // treat a project scope as an artifact identity.
                StoreRecord::new(memory.id, memory.content)
                    .with_container(memory.project_path)
                    .with_created_at(memory.created_at)
            })
            .collect())
    }
}

/// `archon-docs`' own hybrid retrieval over the evidence database.
///
/// A genuinely separate read from the knowledge graph's `hybrid_retriever`,
/// despite both living in one CozoDB file: different weights, a different vector
/// path (RocksDB raw vectors through `DocVectorStore`, not Cozo HNSW), different
/// fallbacks. Two answers over one corpus is precisely the case the dedupe and
/// conflict rules exist for.
pub(crate) struct DocsStore {
    db: Arc<DbInstance>,
}

impl DocsStore {
    pub(crate) fn new(db: Arc<DbInstance>) -> Self {
        Self { db }
    }
}

impl StoreRecordSource for DocsStore {
    fn search(&self, text: &str, limit: usize) -> Result<Vec<StoreRecord>> {
        let found = archon_docs::retrieval::search(&self.db, text, limit)
            .map_err(|error| KnowledgeError::Store(format!("docs retrieval failed: {error}")))?;
        Ok(found
            .results
            .into_iter()
            .map(|result| {
                StoreRecord::new(result.chunk_id, result.content)
                    .with_container(result.document_id)
                    .with_score(result.score)
            })
            .collect())
    }
}

/// `archon-leann`'s semantic code index, opened read-only.
///
/// Constructed through [`archon_leann::search::Search`] rather than
/// `CodeIndex::new`, which calls `ensure_schema()` — a write. Indexing holds the
/// Cozo write lock across a whole `multi_transaction`, so a recall must never be
/// the thing that builds an index. A missing index is an error the facade
/// reports as a per-source failure, not a silent zero.
pub(crate) struct CodeIndexStore {
    search: archon_leann::search::Search,
}

impl CodeIndexStore {
    pub(crate) fn open(db_path: &std::path::Path) -> anyhow::Result<Self> {
        if !db_path.exists() {
            anyhow::bail!(
                "no code index at {}; build it out of band (`archon index`) — \
                 indexing takes the Cozo write lock and must not run inside a recall",
                db_path.display()
            );
        }
        let guard = archon_cozo::CozoGuardConfig::for_db_path(db_path);
        let db = archon_cozo::open_sqlite_guarded(
            db_path.to_string_lossy().as_ref(),
            "open leann code index for unified recall (read-only)",
            &guard,
        )
        .map_err(|error| anyhow::anyhow!("opening code index at {}: {error}", db_path.display()))?;
        let embedder = archon_memory::embedding::create_provider(
            &archon_memory::embedding::EmbeddingConfig::default(),
        )
        .map_err(|error| anyhow::anyhow!("embedding provider unavailable for recall: {error}"))?;
        Ok(Self::with_embedder(db, embedder))
    }

    /// Wrap an open index and a chosen embedder, so a test can drive the real
    /// `SearchResult` → [`StoreRecord`] mapping without a model on disk.
    pub(crate) fn with_embedder(
        db: DbInstance,
        embedder: Arc<dyn archon_memory::embedding::EmbeddingProvider>,
    ) -> Self {
        Self {
            search: archon_leann::search::Search::new(db, embedder),
        }
    }
}

impl StoreRecordSource for CodeIndexStore {
    fn search(&self, text: &str, limit: usize) -> Result<Vec<StoreRecord>> {
        let hits = self
            .search
            .search_code(text, limit)
            .map_err(|error| KnowledgeError::Store(format!("code index query failed: {error}")))?;
        Ok(hits
            .into_iter()
            .map(|hit| {
                // The span is the record id; the file is the container, and the
                // adapter turns only the file into provenance — two spans of one
                // file that return the same text are one piece of evidence.
                let path = hit.file_path.to_string_lossy().replace('\\', "/");
                StoreRecord::new(
                    format!("{path}:{}-{}", hit.line_start, hit.line_end),
                    hit.content,
                )
                .with_container(path)
                .with_score(hit.relevance_score)
            })
            .collect())
    }
}

#[cfg(test)]
mod tests;
