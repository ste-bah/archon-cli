//! Repository indexer — walk, chunk, embed, and atomically store code files.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use cozo::DbInstance;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use archon_memory::embedding::EmbeddingProvider;

use crate::chunker::{Chunker, Language};
use crate::embedding_pass;
use crate::index_storage::{FileState, FileStore, ReplaceFileOutcome, skip_if_contended};
use crate::language::{self, configured_excludes, configured_includes};
use crate::metadata::{CodeChunk, IndexConfig, IndexStats};

/// Which embedding backend to use for indexing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingProviderKind {
    /// fastembed local (768-dim).
    Local,
    /// OpenAI text-embedding-3-small (1536-dim).
    OpenAI,
    /// Deterministic mock for testing: each chunk hashes to its own point on
    /// the unit sphere. Reproducible, and non-degenerate so HNSW construction
    /// has something to prune on — but it encodes no semantics, so it cannot
    /// support an assertion about search *relevance*.
    Mock,
}

/// Embedding configuration for the indexer.
#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    pub provider: EmbeddingProviderKind,
    /// Vector dimension. 768 for local fastembed, 1536 for OpenAI, arbitrary for mock.
    pub dimension: usize,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: EmbeddingProviderKind::Local,
            dimension: 768,
        }
    }
}

/// Files embedded and persisted per group before the next group starts.
///
/// This is the resume granularity: a cancel loses at most this many files'
/// embeddings, and everything before it is already in the store. Small enough
/// that a Ctrl+C never costs much, large enough that the embedder still sees
/// full `EMBED_BATCH_SIZE` batches within a group rather than short ones.
const PERSIST_GROUP_FILES: usize = 32;

/// One stale file the walk found, holding only what the walk already knew.
///
/// Deliberately no content and no chunks: the walk visits the whole repository
/// before a single group is embedded, so anything stored here is multiplied by
/// the file count. Chunks are materialised one group at a time in
/// [`Indexer::chunk_group`] instead, which is what bounds peak memory by
/// `PERSIST_GROUP_FILES` rather than by repository size.
struct PendingFile {
    language_name: String,
    path: PathBuf,
}

/// A pending file with its chunks, alive only for the group being embedded.
///
/// `file_hash` is recomputed here rather than carried from the walk so that the
/// hash written to `file_states` always describes the same bytes the chunks
/// came from. The walk's hash answered a different question -- "is the stored
/// copy stale?" -- and a file edited between the walk and its group would
/// otherwise be recorded under a hash it no longer has, and never re-indexed.
pub(crate) struct ChunkedFile<'a> {
    pending: &'a PendingFile,
    file_path: String,
    file_hash: String,
    pub(crate) chunks: Vec<CodeChunk>,
}

/// Repository and single-file indexing: walk, chunk, embed, store in CozoDB HNSW.
#[derive(Clone)]
pub struct Indexer {
    db: DbInstance,
    /// Write-guard config for `db`. Carried explicitly rather than resolved
    /// from the guard registry: `Indexer` owns its `DbInstance` by value, and
    /// the registry keys on the pointer identity of the handle that was
    /// registered, which a move or clone does not preserve.
    guard: archon_cozo::CozoGuardConfig,
    embedder: Arc<dyn EmbeddingProvider>,
    chunker: Chunker,
    dimension: usize,
}

impl Indexer {
    /// Return a reference to the underlying CozoDB instance.
    pub fn db(&self) -> &DbInstance {
        &self.db
    }

    /// Return a reference to the embedding provider.
    pub fn embedder(&self) -> &Arc<dyn EmbeddingProvider> {
        &self.embedder
    }

    /// Create a new indexer over an unguarded (typically in-memory) instance.
    ///
    /// In-memory stores have no cross-process contention, so the default guard
    /// config is correct. Persistent stores should use [`Indexer::with_guard`]
    /// with a config built from the database path.
    pub fn new(
        db: DbInstance,
        config: EmbeddingConfig,
        grammar_dir: Option<PathBuf>,
    ) -> Result<Self> {
        Self::with_guard(
            db,
            archon_cozo::CozoGuardConfig::default(),
            config,
            grammar_dir,
        )
    }

    /// Create a new indexer whose writes are serialised by `guard`.
    pub fn with_guard(
        db: DbInstance,
        guard: archon_cozo::CozoGuardConfig,
        config: EmbeddingConfig,
        grammar_dir: Option<PathBuf>,
    ) -> Result<Self> {
        let embedder = embedding_pass::create_embedder(&config)?;
        Ok(Self {
            db,
            guard,
            embedder,
            chunker: Chunker::new(grammar_dir)?,
            dimension: config.dimension,
        })
    }

    /// Create CozoDB relations and HNSW index if not present. Idempotent.
    pub fn ensure_schema(&self) -> Result<()> {
        self.file_store().ensure_schema()
    }

    /// Index an entire repository directory tree without blocking the async executor.
    pub async fn index_repository(&self, root: &Path, config: &IndexConfig) -> Result<IndexStats> {
        let indexer = self.clone();
        let root = root.to_path_buf();
        let config = config.clone();
        tokio::task::spawn_blocking(move || indexer.index_repository_blocking(&root, &config))
            .await
            .map_err(|error| anyhow::anyhow!("repository indexing task failed: {error}"))?
    }

    /// Synchronous repository indexing for callers that explicitly offload LEANN work.
    pub fn index_repository_blocking(
        &self,
        root: &Path,
        config: &IndexConfig,
    ) -> Result<IndexStats> {
        self.index_repository_blocking_with_cancel(root, config, &AtomicBool::new(false))
    }

    /// Synchronously index a repository with cooperative cancellation checks.
    pub fn index_repository_blocking_with_cancel(
        &self,
        root: &Path,
        config: &IndexConfig,
        cancel: &AtomicBool,
    ) -> Result<IndexStats> {
        let mut stats = IndexStats::default();
        let mut pending = Vec::new();
        let exclude = configured_excludes(config);
        let include = configured_includes(config);

        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            // Relative to `root`, never the absolute path: the exclusions are
            // component names, so an absolute path lets the directories the
            // checkout happens to live under veto the whole walk.
            .filter_entry(|entry| !language::is_excluded_under_root(entry.path(), root, &exclude))
        {
            if is_cancelled(cancel) {
                return Ok(stats);
            }
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let Some(language_name) = language::detect_language(path) else {
                continue;
            };
            if !language::is_code_language(&language_name) {
                continue;
            }
            if !language::is_included(path.strip_prefix(root).unwrap_or(path), &include) {
                continue;
            }
            // Reading here is not wasted work even though the content is
            // dropped again: the staleness check is a content hash, so the
            // bytes have to be in hand to decide whether this file belongs in
            // the work-list at all. Only the chunks are deferred.
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            match self
                .file_store()
                .file_state(path.to_string_lossy().as_ref(), &sha256_hex(&content))?
            {
                FileState::Current => continue,
                // Counted, not silently dropped: the store never answered for
                // this file, so the pass cannot claim to have covered it.
                FileState::Contended => {
                    stats.skipped_files += 1;
                    continue;
                }
                FileState::Stale => {}
            }
            pending.push(PendingFile {
                language_name,
                path: path.to_path_buf(),
            });
        }

        let total_pending = pending.len();
        if total_pending == 0 {
            tracing::info!("LEANN index up to date; no changed files to embed");
            return Ok(stats);
        }
        tracing::info!(
            files = total_pending,
            group = PERSIST_GROUP_FILES,
            "LEANN indexing started"
        );

        // Chunk, embed and persist in groups rather than doing any one of those
        // phases across the whole repository before starting the next.
        //
        // An earlier shape embedded every file first, so a cancel during the
        // embedding pass returned having written nothing -- the entire pass was
        // discarded and the next run started from zero. On a large corpus that
        // is an unbounded amount of work you can never bank, and because
        // `file_hash_matches` skips already-stored files at the top of the
        // walk, persisting as we go is exactly what makes a re-run resume.
        //
        // Chunking stayed all-up-front through that change and kept the cost
        // grouping was meant to remove: every chunk of every changed file, with
        // its full source text, resident before the first embedding call. Doing
        // it inside the group loop makes peak memory a function of
        // `PERSIST_GROUP_FILES`, not of how big the repository is.
        //
        // Groups rather than single files so the embedder keeps a batch worth
        // of work per call; `EMBED_BATCH_SIZE` still governs the model batch.
        for (group_index, group) in pending.chunks(PERSIST_GROUP_FILES).enumerate() {
            if is_cancelled(cancel) {
                tracing::info!(
                    files_indexed = stats.total_files,
                    chunks_indexed = stats.total_chunks,
                    "LEANN indexing cancelled; progress so far is persisted"
                );
                return Ok(stats);
            }
            let Some(chunked) = self.chunk_group(group, cancel) else {
                tracing::info!(
                    files_indexed = stats.total_files,
                    chunks_indexed = stats.total_chunks,
                    "LEANN indexing cancelled during chunking; progress so far is persisted"
                );
                return Ok(stats);
            };
            let Some(prepared) =
                embedding_pass::prepare_repository_files(&self.embedder, &chunked, &|| {
                    is_cancelled(cancel)
                })?
            else {
                tracing::info!(
                    files_indexed = stats.total_files,
                    chunks_indexed = stats.total_chunks,
                    "LEANN indexing cancelled during embedding; progress so far is persisted"
                );
                return Ok(stats);
            };
            for (file, chunks) in chunked.iter().zip(prepared) {
                if is_cancelled(cancel) {
                    return Ok(stats);
                }
                let outcome = self.file_store().replace_file_with_cancel(
                    &file.file_path,
                    &file.file_hash,
                    &chunks,
                    || is_cancelled(cancel),
                );
                let Some(outcome) = skip_if_contended(&file.file_path, outcome, &mut stats)? else {
                    continue;
                };
                if matches!(outcome, ReplaceFileOutcome::Cancelled) {
                    return Ok(stats);
                }
                if !chunks.is_empty() {
                    stats.total_files += 1;
                    stats.total_chunks += chunks.len();
                    *stats
                        .languages
                        .entry(file.pending.language_name.clone())
                        .or_insert(0) += 1;
                }
            }
            let done = ((group_index + 1) * PERSIST_GROUP_FILES).min(total_pending);
            tracing::info!(
                done,
                total = total_pending,
                chunks = stats.total_chunks,
                "LEANN indexing progress"
            );
        }
        // Skips are reported here as well as warned about per file: a pass that
        // lost four hundred files to a peer and still returned `Ok` otherwise
        // looks exactly like a complete one, and the difference is a search
        // corpus with holes in it.
        tracing::info!(
            files_indexed = stats.total_files,
            chunks_indexed = stats.total_chunks,
            skipped = stats.skipped_files,
            "LEANN indexing finished"
        );
        Ok(stats)
    }

    /// Index a single file (detect language, chunk, embed, store).
    /// Replaces existing chunks for that file if content has changed.
    pub async fn index_file(&self, path: &Path) -> Result<()> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let language_name =
            language::detect_language(path).unwrap_or_else(|| "unknown".to_string());
        self.index_changed_file(path, &content, &language_name, None)?;
        Ok(())
    }

    /// Remove all chunks and cached file state for a file from the index.
    pub async fn remove_file(&self, path: &Path) -> Result<()> {
        self.file_store()
            .remove_file(path.to_string_lossy().as_ref())
    }

    fn index_changed_file(
        &self,
        path: &Path,
        content: &str,
        language_name: &str,
        cancel: Option<&AtomicBool>,
    ) -> Result<Option<usize>> {
        let file_path = path.to_string_lossy();
        let file_hash = sha256_hex(content);
        match self.file_store().file_state(&file_path, &file_hash)? {
            FileState::Current => return Ok(Some(0)),
            // Nothing was written, so the caller's next attempt still sees the
            // file as stale. `None` is the existing "did no work" answer.
            FileState::Contended => return Ok(None),
            FileState::Stale => {}
        }
        self.prepare_and_commit(path, content, language_name, &file_path, &file_hash, cancel)
    }

    fn prepare_and_commit(
        &self,
        path: &Path,
        content: &str,
        language_name: &str,
        file_path: &str,
        file_hash: &str,
        cancel: Option<&AtomicBool>,
    ) -> Result<Option<usize>> {
        if cancelled(cancel) {
            return Ok(None);
        }
        let chunks = self
            .chunker
            .chunk_file(path, content, str_to_chunker_language(language_name));
        let Some(prepared) =
            embedding_pass::prepare_chunks(&self.embedder, chunks, &|| cancelled(cancel))?
        else {
            return Ok(None);
        };
        if cancelled(cancel) {
            return Ok(None);
        }
        let outcome =
            self.file_store()
                .replace_file_with_cancel(file_path, file_hash, &prepared, || cancelled(cancel));
        // Single-file indexing has no walk to protect, but it shares the rule:
        // a contended file is left stale for the next attempt rather than
        // raised as a failure the caller cannot act on.
        let mut ignored = IndexStats::default();
        match skip_if_contended(file_path, outcome, &mut ignored)? {
            Some(ReplaceFileOutcome::Committed) => Ok(Some(prepared.len())),
            Some(ReplaceFileOutcome::Cancelled) | None => Ok(None),
        }
    }

    /// Read and chunk one group's files. `None` means cancelled.
    ///
    /// A file that has vanished or become unreadable since the walk is dropped
    /// from the group rather than failing the run: the walk's snapshot is
    /// advisory by the time we get here, and a deleted file is not an error the
    /// caller can do anything about.
    fn chunk_group<'a>(
        &self,
        group: &'a [PendingFile],
        cancel: &AtomicBool,
    ) -> Option<Vec<ChunkedFile<'a>>> {
        let mut chunked = Vec::with_capacity(group.len());
        for pending in group {
            if is_cancelled(cancel) {
                return None;
            }
            let Ok(content) = std::fs::read_to_string(&pending.path) else {
                continue;
            };
            chunked.push(ChunkedFile {
                file_path: pending.path.to_string_lossy().into_owned(),
                file_hash: sha256_hex(&content),
                chunks: self.chunker.chunk_file(
                    &pending.path,
                    &content,
                    str_to_chunker_language(&pending.language_name),
                ),
                pending,
            });
        }
        Some(chunked)
    }

    fn file_store(&self) -> FileStore<'_> {
        FileStore::new(&self.db, self.dimension, &self.guard)
    }

    #[cfg(test)]
    fn remove_file_chunks(&self, file_path: &str) -> Result<()> {
        self.file_store().remove_file_chunks(file_path)
    }
}

/// Compute SHA-256 hash of content as hex string.
fn sha256_hex(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

/// Map a language string to the chunker's Language enum.
fn str_to_chunker_language(language: &str) -> Language {
    match language {
        "rust" => Language::Rust,
        "python" => Language::Python,
        "typescript" | "typescriptreact" => Language::TypeScript,
        "go" => Language::Go,
        _ => Language::Unknown,
    }
}

fn cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(is_cancelled)
}

fn is_cancelled(cancel: &AtomicBool) -> bool {
    cancel.load(Ordering::Relaxed)
}

#[cfg(test)]
#[path = "indexer_atomicity_tests.rs"]
mod indexer_atomicity_tests;
#[cfg(test)]
#[path = "indexer_persistence_evidence_tests.rs"]
mod indexer_persistence_evidence_tests;
