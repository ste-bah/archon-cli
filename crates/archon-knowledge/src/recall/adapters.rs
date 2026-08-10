//! The four adapters, and the read-only ports three of them stand on.
//!
//! # One native adapter, three ported ones
//!
//! [`KnowledgeStoreAdapter`] talks to CozoDB directly: the knowledge graph is
//! this crate's own store, so reaching it costs nothing.
//!
//! Memory, docs and the code index are reached through [`StoreRecordSource`],
//! implemented in the command layer over the real handles. The reason is the
//! one already written down for [`crate::traceability::CodeSearch`]: a direct
//! dependency would pull `tokio`, `fastembed`, RocksDB, `lopdf` and tree-sitter
//! into a crate that today builds with none of them, and it would make every
//! test of the merge rules require a live store and a live embedding model. With
//! a port, the merge is testable with four hand-written vectors and no I/O at
//! all — which is exactly what `tests/unified_recall.rs` does.
//!
//! # Provenance references are the join key
//!
//! Every adapter emits references in a shared vocabulary — `chunk:<id>`,
//! `doc:<id>`, `memory:<id>`, `file:<path>` — and that vocabulary is the only
//! thing making cross-store dedupe possible without a shared database. It is
//! deliberately literal: docs and the knowledge graph read the *same*
//! `doc_chunks` relation, so a chunk reached both ways yields the same
//! `chunk:<id>` on both sides and folds into one hit. Change a prefix here and
//! two stores stop recognising each other's artifacts.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use cozo::DbInstance;

use crate::errors::Result;
use crate::hybrid_retriever::{self, SearchOptions};
use crate::recall::{RecallHit, RecallQuery, RecallSource, RecallSourceAdapter};

/// Provenance prefix for a document chunk, shared by docs and the graph.
pub const CHUNK_REF: &str = "chunk";
/// Provenance prefix for a document.
pub const DOC_REF: &str = "doc";
/// Provenance prefix for a memory node.
pub const MEMORY_REF: &str = "memory";
/// Provenance prefix for a source file.
pub const FILE_REF: &str = "file";

fn reference(prefix: &str, value: &str) -> String {
    format!("{prefix}:{value}")
}

/// One record as a store reports it, before this crate gives it an identity.
///
/// Deliberately store-shaped and not [`RecallHit`]-shaped: the command-layer
/// implementations should translate their own types and nothing else, so the
/// rules that matter — provenance vocabulary, rank, calibration — stay in one
/// place and cannot drift per store.
#[derive(Debug, Clone, PartialEq)]
pub struct StoreRecord {
    /// The store's own primary key.
    pub id: String,
    pub content: String,
    /// The store's own relevance number, where it has one. `archon-memory` has
    /// none: `recall_memories` returns an ordered `Vec<Memory>` and discards the
    /// score internally.
    pub score: Option<f64>,
    /// The store's own container for this record — document id, file path.
    pub container: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    /// The store's own confidence, not a relevance score.
    pub confidence: Option<f32>,
}

impl StoreRecord {
    pub fn new(id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            content: content.into(),
            score: None,
            container: None,
            created_at: None,
            confidence: None,
        }
    }

    pub fn with_score(mut self, score: f64) -> Self {
        self.score = Some(score);
        self
    }

    pub fn with_container(mut self, container: impl Into<String>) -> Self {
        self.container = Some(container.into());
        self
    }

    pub fn with_created_at(mut self, created_at: DateTime<Utc>) -> Self {
        self.created_at = Some(created_at);
        self
    }

    pub fn with_confidence(mut self, confidence: f32) -> Self {
        self.confidence = Some(confidence);
        self
    }
}

/// A read-only query against one store, in that store's own terms.
///
/// Implementations must not index, write, or take a write lock. Records must
/// come back in the store's own relevance order — the merge treats position as
/// the store's judgement, and it is the only judgement the merge trusts.
pub trait StoreRecordSource: Send + Sync {
    fn search(&self, text: &str, limit: usize) -> Result<Vec<StoreRecord>>;
}

/// Map records into hits, stamping rank-derived scores and provenance.
fn hits_from<F>(source: RecallSource, records: Vec<StoreRecord>, refs: F) -> Vec<RecallHit>
where
    F: Fn(&StoreRecord) -> Vec<String>,
{
    records
        .into_iter()
        .enumerate()
        .map(|(rank, record)| {
            let provenance = refs(&record);
            let mut hit = RecallHit::at_rank(source, record.id, record.content, rank)
                .with_provenance(provenance);
            hit.source_score = record.score;
            hit.created_at = record.created_at;
            hit.confidence = record.confidence;
            hit
        })
        .collect()
}

/// `archon-memory`, through a port.
pub struct MemoryAdapter {
    store: Arc<dyn StoreRecordSource>,
}

impl MemoryAdapter {
    pub fn new(store: Arc<dyn StoreRecordSource>) -> Self {
        Self { store }
    }
}

impl RecallSourceAdapter for MemoryAdapter {
    fn source(&self) -> RecallSource {
        RecallSource::Memory
    }

    fn recall(&self, query: &RecallQuery) -> Result<Vec<RecallHit>> {
        let records = self
            .store
            .search(&query.text, query.quota_for(RecallSource::Memory))?;
        // A memory's container is its project path, which is a *scope*, not an
        // artifact: two unrelated memories in one project must not dedupe
        // against each other, so it is not emitted as provenance.
        Ok(hits_from(RecallSource::Memory, records, |record| {
            vec![reference(MEMORY_REF, &record.id)]
        }))
    }
}

/// `archon-docs`, through a port.
pub struct DocsAdapter {
    store: Arc<dyn StoreRecordSource>,
}

impl DocsAdapter {
    pub fn new(store: Arc<dyn StoreRecordSource>) -> Self {
        Self { store }
    }
}

impl RecallSourceAdapter for DocsAdapter {
    fn source(&self) -> RecallSource {
        RecallSource::Docs
    }

    fn recall(&self, query: &RecallQuery) -> Result<Vec<RecallHit>> {
        let records = self
            .store
            .search(&query.text, query.quota_for(RecallSource::Docs))?;
        Ok(hits_from(RecallSource::Docs, records, |record| {
            chunk_refs(&record.id, record.container.as_deref())
        }))
    }
}

/// `archon-leann`, through a port.
pub struct CodeIndexAdapter {
    store: Arc<dyn StoreRecordSource>,
}

impl CodeIndexAdapter {
    pub fn new(store: Arc<dyn StoreRecordSource>) -> Self {
        Self { store }
    }
}

impl RecallSourceAdapter for CodeIndexAdapter {
    fn source(&self) -> RecallSource {
        RecallSource::Code
    }

    fn recall(&self, query: &RecallQuery) -> Result<Vec<RecallHit>> {
        let records = self
            .store
            .search(&query.text, query.quota_for(RecallSource::Code))?;
        // The file, not the span, is the artifact: two spans of one file that
        // return the same text are the same evidence, and a caller chasing a
        // citation opens the file either way.
        Ok(hits_from(RecallSource::Code, records, |record| {
            record
                .container
                .as_deref()
                .map(|path| vec![reference(FILE_REF, &path.replace('\\', "/"))])
                .unwrap_or_default()
        }))
    }
}

/// The knowledge graph's own retrieval, with no port in between.
pub struct KnowledgeStoreAdapter {
    db: Arc<DbInstance>,
    options: SearchOptions,
}

impl KnowledgeStoreAdapter {
    /// `options` carries the caller's mode and any query embedding, because
    /// embedding a query needs a provider this crate does not own. The `top_k`
    /// it holds is overwritten per query by the source's quota.
    pub fn new(db: Arc<DbInstance>, options: SearchOptions) -> Self {
        Self { db, options }
    }
}

impl RecallSourceAdapter for KnowledgeStoreAdapter {
    fn source(&self) -> RecallSource {
        RecallSource::Knowledge
    }

    fn recall(&self, query: &RecallQuery) -> Result<Vec<RecallHit>> {
        let options = SearchOptions {
            top_k: query.quota_for(RecallSource::Knowledge),
            ..self.options.clone()
        };
        let results = hybrid_retriever::search(&self.db, &query.text, &options)?;
        // `combined_score` is kept as `source_score` and NOT used for ordering
        // across sources — it is a weighted sum with this retriever's own ad hoc
        // weights (0.55/0.45), which is precisely the kind of number the R7
        // slice refuses to treat as comparable.
        Ok(hits_from(
            RecallSource::Knowledge,
            results
                .into_iter()
                .map(|result| {
                    StoreRecord::new(result.artifact_id, result.content)
                        .with_score(result.combined_score)
                        .with_container(result.document_id)
                })
                .collect(),
            |record| chunk_refs(&record.id, record.container.as_deref()),
        ))
    }
}

/// `chunk:<id>` plus `doc:<id>` — the vocabulary docs and the graph share.
fn chunk_refs(chunk_id: &str, document_id: Option<&str>) -> Vec<String> {
    let mut refs = vec![reference(CHUNK_REF, chunk_id)];
    if let Some(document_id) = document_id {
        refs.push(reference(DOC_REF, document_id));
    }
    refs
}

#[cfg(test)]
#[path = "adapters/tests.rs"]
mod tests;
