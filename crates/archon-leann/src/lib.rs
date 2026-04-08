//! Archon LeANN — native semantic code search and indexing.

pub mod chunker;
pub mod indexer;
pub mod language;
pub mod metadata;
pub mod queue;
pub mod search;
pub mod stats;

pub use metadata::{CodeChunk, CodeMetadata, IndexConfig, IndexStats, QueueResult, SearchResult};

use anyhow::Result;
use std::path::{Path, PathBuf};

use indexer::{EmbeddingConfig, Indexer};

/// Native semantic code search index.
///
/// Delegates indexing operations to [`Indexer`] and search operations to
/// [`search::Search`].
pub struct CodeIndex {
    db_path: PathBuf,
    indexer: Indexer,
    search: search::Search,
}

impl CodeIndex {
    /// Create a new `CodeIndex` backed by the given database path.
    ///
    /// Uses the provided embedding configuration for vector generation.
    /// Creates the CozoDB instance and ensures the schema exists.
    pub fn new(db_path: impl Into<PathBuf>, embedding_config: EmbeddingConfig) -> Result<Self> {
        let db_path = db_path.into();
        let db = cozo::DbInstance::new(
            "sqlite",
            db_path.to_string_lossy().as_ref(),
            Default::default(),
        )
        .map_err(|e| anyhow::anyhow!("failed to open CozoDB at {}: {}", db_path.display(), e))?;
        let indexer = Indexer::new(db, embedding_config, None)?;
        indexer.ensure_schema()?;
        let search = search::Search::new(indexer.db().clone(), indexer.embedder().clone());
        Ok(CodeIndex {
            db_path,
            indexer,
            search,
        })
    }

    /// Create a `CodeIndex` from an existing CozoDB instance (for testing).
    pub fn from_db(db: cozo::DbInstance, embedding_config: EmbeddingConfig) -> Result<Self> {
        let indexer = Indexer::new(db, embedding_config, None)?;
        indexer.ensure_schema()?;
        let search = search::Search::new(indexer.db().clone(), indexer.embedder().clone());
        Ok(CodeIndex {
            db_path: PathBuf::new(),
            indexer,
            search,
        })
    }

    /// Search the index for code matching the given natural-language query.
    pub fn search_code(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        self.search.search_code(query, limit)
    }

    /// Index an entire repository according to the given configuration.
    pub async fn index_repository(&self, path: &Path, config: &IndexConfig) -> Result<IndexStats> {
        self.indexer.index_repository(path, config).await
    }

    /// Index a single file.
    pub async fn index_file(&self, path: &Path) -> Result<()> {
        self.indexer.index_file(path).await
    }

    /// Remove all chunks for a file from the index.
    pub async fn remove_file(&self, path: &Path) -> Result<()> {
        self.indexer.remove_file(path).await
    }

    /// Find code chunks similar to the given source snippet.
    pub fn find_similar_code(&self, code: &str, limit: usize) -> Result<Vec<SearchResult>> {
        self.search.find_similar_code(code, limit)
    }

    /// Process queued indexing work from the given queue path.
    pub async fn process_queue(&self, queue_path: &Path) -> Result<QueueResult> {
        let processor = queue::QueueProcessor::new(&self.indexer);
        processor.process_queue(queue_path).await
    }

    /// Append file paths to the indexing queue.
    pub fn add_to_queue(&self, queue_path: &Path, file_paths: &[PathBuf]) -> Result<()> {
        queue::QueueProcessor::add_to_queue(queue_path, file_paths)
    }

    /// Return the status of the indexing queue without processing it.
    pub fn queue_status(&self, queue_path: &Path) -> Result<queue::QueueStatus> {
        queue::QueueProcessor::queue_status(queue_path)
    }

    /// Return statistics about the current index.
    pub async fn stats(&self) -> Result<IndexStats> {
        // Full stats from DB is deferred
        Ok(IndexStats::default())
    }

    /// Return the database path backing this index.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
}
