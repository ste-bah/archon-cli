//! Repository indexer — walk, chunk, embed, and atomically store code files.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use cozo::DbInstance;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use archon_memory::embedding::{self, EmbeddingProvider};

use crate::chunker::{Chunker, Language};
use crate::index_storage::{FileStore, PreparedChunk, ReplaceFileOutcome};
use crate::language;
use crate::metadata::{CodeChunk, IndexConfig, IndexStats};

/// Which embedding backend to use for indexing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingProviderKind {
    /// fastembed local (768-dim).
    Local,
    /// OpenAI text-embedding-3-small (1536-dim).
    OpenAI,
    /// Deterministic mock for testing (generates fixed-size zero vectors).
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

/// A deterministic embedding provider that returns zero vectors.
struct MockEmbeddingProvider {
    dim: usize,
}

impl EmbeddingProvider for MockEmbeddingProvider {
    fn embed(
        &self,
        texts: &[String],
    ) -> std::result::Result<Vec<Vec<f32>>, archon_memory::types::MemoryError> {
        Ok(texts.iter().map(|_| vec![0.0; self.dim]).collect())
    }

    fn dimensions(&self) -> usize {
        self.dim
    }
}

/// Maximum chunks per embedding batch.
const EMBED_BATCH_SIZE: usize = 64;

struct PendingFile {
    language_name: String,
    file_path: String,
    file_hash: String,
    chunks: Vec<CodeChunk>,
}

/// Repository and single-file indexing: walk, chunk, embed, store in CozoDB HNSW.
#[derive(Clone)]
pub struct Indexer {
    db: DbInstance,
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

    /// Create a new indexer.
    pub fn new(
        db: DbInstance,
        config: EmbeddingConfig,
        grammar_dir: Option<PathBuf>,
    ) -> Result<Self> {
        let embedder = create_embedder(&config)?;
        Ok(Self {
            db,
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

        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| !language::is_excluded(entry.path(), &exclude))
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
            if !is_code_language(&language_name) {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            let file_path = path.to_string_lossy().into_owned();
            let file_hash = sha256_hex(&content);
            if self
                .file_store()
                .file_hash_matches(&file_path, &file_hash)?
            {
                continue;
            }
            let chunks =
                self.chunker
                    .chunk_file(path, &content, str_to_chunker_language(&language_name));
            pending.push(PendingFile {
                language_name,
                file_path,
                file_hash,
                chunks,
            });
        }

        let Some(prepared) = self.prepare_repository_files(&pending, cancel)? else {
            return Ok(stats);
        };
        for (file, chunks) in pending.iter().zip(prepared) {
            if is_cancelled(cancel) {
                return Ok(stats);
            }
            let outcome = self.file_store().replace_file_with_cancel(
                &file.file_path,
                &file.file_hash,
                &chunks,
                || is_cancelled(cancel),
            )?;
            if matches!(outcome, ReplaceFileOutcome::Cancelled) {
                return Ok(stats);
            }
            if !chunks.is_empty() {
                stats.total_files += 1;
                stats.total_chunks += chunks.len();
                *stats
                    .languages
                    .entry(file.language_name.clone())
                    .or_insert(0) += 1;
            }
        }
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
        if self
            .file_store()
            .file_hash_matches(&file_path, &file_hash)?
        {
            return Ok(Some(0));
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
        let Some(prepared) = self.prepare_chunks(chunks, cancel)? else {
            return Ok(None);
        };
        if cancelled(cancel) {
            return Ok(None);
        }
        let outcome =
            self.file_store()
                .replace_file_with_cancel(file_path, file_hash, &prepared, || cancelled(cancel))?;
        match outcome {
            ReplaceFileOutcome::Committed => Ok(Some(prepared.len())),
            ReplaceFileOutcome::Cancelled => Ok(None),
        }
    }

    fn prepare_repository_files(
        &self,
        files: &[PendingFile],
        cancel: &AtomicBool,
    ) -> Result<Option<Vec<Vec<PreparedChunk>>>> {
        let mut prepared = files
            .iter()
            .map(|file| Vec::with_capacity(file.chunks.len()))
            .collect::<Vec<_>>();
        let mut batch = Vec::with_capacity(EMBED_BATCH_SIZE);

        for (file_index, file) in files.iter().enumerate() {
            for chunk in &file.chunks {
                if is_cancelled(cancel) {
                    return Ok(None);
                }
                batch.push((file_index, chunk));
                if batch.len() == EMBED_BATCH_SIZE
                    && self
                        .embed_repository_batch(&mut prepared, &mut batch, cancel)?
                        .is_none()
                {
                    return Ok(None);
                }
            }
        }
        if !batch.is_empty()
            && self
                .embed_repository_batch(&mut prepared, &mut batch, cancel)?
                .is_none()
        {
            return Ok(None);
        }
        Ok(Some(prepared))
    }

    fn embed_repository_batch(
        &self,
        prepared: &mut [Vec<PreparedChunk>],
        batch: &mut Vec<(usize, &CodeChunk)>,
        cancel: &AtomicBool,
    ) -> Result<Option<()>> {
        if is_cancelled(cancel) {
            return Ok(None);
        }
        let texts = batch
            .iter()
            .map(|(_, chunk)| chunk.metadata.chunk_content.clone())
            .collect::<Vec<_>>();
        let embeddings = self
            .embedder
            .embed(&texts)
            .map_err(|error| anyhow::anyhow!("embedding failed: {error}"))?;
        if embeddings.len() != batch.len() {
            anyhow::bail!(
                "embedding count mismatch: got {} for {} chunks",
                embeddings.len(),
                batch.len()
            );
        }
        if is_cancelled(cancel) {
            return Ok(None);
        }
        for ((file_index, chunk), embedding) in batch.drain(..).zip(embeddings) {
            prepared[file_index].push(PreparedChunk {
                chunk: chunk.clone(),
                embedding,
            });
        }
        Ok(Some(()))
    }

    fn prepare_chunks(
        &self,
        chunks: Vec<CodeChunk>,
        cancel: Option<&AtomicBool>,
    ) -> Result<Option<Vec<PreparedChunk>>> {
        let mut prepared = Vec::with_capacity(chunks.len());
        for batch in chunks.chunks(EMBED_BATCH_SIZE) {
            if cancelled(cancel) {
                return Ok(None);
            }
            let texts = batch
                .iter()
                .map(|chunk| chunk.metadata.chunk_content.clone())
                .collect::<Vec<_>>();
            let embeddings = self
                .embedder
                .embed(&texts)
                .map_err(|error| anyhow::anyhow!("embedding failed: {error}"))?;
            if embeddings.len() != batch.len() {
                anyhow::bail!(
                    "embedding count mismatch: got {} for {} chunks",
                    embeddings.len(),
                    batch.len()
                );
            }
            if cancelled(cancel) {
                return Ok(None);
            }
            prepared.extend(
                batch
                    .iter()
                    .cloned()
                    .zip(embeddings)
                    .map(|(chunk, embedding)| PreparedChunk { chunk, embedding }),
            );
        }
        Ok(Some(prepared))
    }

    fn file_store(&self) -> FileStore<'_> {
        FileStore::new(&self.db, self.dimension)
    }

    #[cfg(test)]
    fn remove_file_chunks(&self, file_path: &str) -> Result<()> {
        self.file_store().remove_file_chunks(file_path)
    }
}

fn create_embedder(config: &EmbeddingConfig) -> Result<Arc<dyn EmbeddingProvider>> {
    match config.provider {
        EmbeddingProviderKind::Mock => Ok(Arc::new(MockEmbeddingProvider {
            dim: config.dimension,
        })),
        EmbeddingProviderKind::Local => embedding::create_provider(&embedding::EmbeddingConfig {
            provider: embedding::EmbeddingProviderKind::Local,
            ..Default::default()
        })
        .context("failed to create local embedding provider"),
        EmbeddingProviderKind::OpenAI => embedding::create_provider(&embedding::EmbeddingConfig {
            provider: embedding::EmbeddingProviderKind::OpenAI,
            ..Default::default()
        })
        .context("failed to create OpenAI embedding provider"),
    }
}

fn configured_excludes(config: &IndexConfig) -> Vec<String> {
    if config.exclude_patterns.is_empty() {
        language::default_exclude_patterns()
    } else {
        config.exclude_patterns.clone()
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

fn is_code_language(language: &str) -> bool {
    matches!(
        language,
        "rust"
            | "python"
            | "typescript"
            | "typescriptreact"
            | "javascript"
            | "javascriptreact"
            | "go"
            | "java"
            | "c"
            | "cpp"
            | "ruby"
            | "php"
            | "swift"
            | "kotlin"
            | "scala"
            | "csharp"
            | "lua"
            | "shell"
            | "r"
            | "dart"
            | "elixir"
            | "erlang"
            | "haskell"
            | "ocaml"
            | "perl"
            | "zig"
            | "nim"
            | "v"
    )
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
