//! Repository indexer — walk, chunk, embed, store in CozoDB HNSW.
//!
//! Implements REQ-LEANN-003, REQ-LEANN-004.
//! NFR-PIPE-007: 500 files indexed within 60 seconds on local fastembed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use cozo::{DataValue, DbInstance, ScriptMutability, Vector};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use archon_memory::embedding::{self, EmbeddingProvider};

use crate::chunker::{Chunker, Language};
use crate::language;
use crate::metadata::{CodeChunk, IndexConfig, IndexStats};

/// Convert a CozoDB error to anyhow.
fn cozo_err(ctx: &str) -> impl FnOnce(cozo::Error) -> anyhow::Error + '_ {
    move |e| anyhow::anyhow!("{}: {}", ctx, e)
}

// ---------------------------------------------------------------------------
// Configuration types
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Mock embedding provider (for tests)
// ---------------------------------------------------------------------------

/// A deterministic embedding provider that returns zero vectors.
struct MockEmbeddingProvider {
    dim: usize,
}

impl EmbeddingProvider for MockEmbeddingProvider {
    fn embed(
        &self,
        texts: &[String],
    ) -> std::result::Result<Vec<Vec<f32>>, archon_memory::types::MemoryError> {
        Ok(texts.iter().map(|_| vec![0.0f32; self.dim]).collect())
    }

    fn dimensions(&self) -> usize {
        self.dim
    }
}

// ---------------------------------------------------------------------------
// Indexer
// ---------------------------------------------------------------------------

/// Maximum chunks per embedding batch.
const EMBED_BATCH_SIZE: usize = 64;

/// Repository and single-file indexing: walk, chunk, embed, store in CozoDB HNSW.
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
    ///
    /// `grammar_dir` is passed through to the tree-sitter `Chunker`.
    pub fn new(
        db: DbInstance,
        config: EmbeddingConfig,
        grammar_dir: Option<PathBuf>,
    ) -> Result<Self> {
        let embedder: Arc<dyn EmbeddingProvider> = match config.provider {
            EmbeddingProviderKind::Mock => Arc::new(MockEmbeddingProvider {
                dim: config.dimension,
            }),
            EmbeddingProviderKind::Local => {
                let emb_config = embedding::EmbeddingConfig {
                    provider: embedding::EmbeddingProviderKind::Local,
                    ..Default::default()
                };
                embedding::create_provider(&emb_config)
                    .context("failed to create local embedding provider")?
            }
            EmbeddingProviderKind::OpenAI => {
                let emb_config = embedding::EmbeddingConfig {
                    provider: embedding::EmbeddingProviderKind::OpenAI,
                    ..Default::default()
                };
                embedding::create_provider(&emb_config)
                    .context("failed to create OpenAI embedding provider")?
            }
        };

        let chunker = Chunker::new(grammar_dir)?;

        Ok(Self {
            db,
            embedder,
            chunker,
            dimension: config.dimension,
        })
    }

    /// Create CozoDB relations and HNSW index if not present. Idempotent.
    pub fn ensure_schema(&self) -> Result<()> {
        let dim = self.dimension;

        // Create stored relation with vector column
        let create_rel = format!(
            ":create code_chunks {{
                chunk_id: String
                =>
                file_path: String,
                language: String,
                line_start: Int,
                line_end: Int,
                chunk_content: String,
                file_hash: String,
                indexed_at: Float,
                embedding: <F32; {dim}>
            }}"
        );
        self.run_idempotent(&create_rel)?;

        // Create HNSW index
        let create_idx = format!(
            "::hnsw create code_chunks:chunk_embedding_idx {{
                dim: {dim},
                m: 50,
                dtype: F32,
                fields: [embedding],
                distance: Cosine,
                ef_construction: 200
            }}"
        );
        self.run_idempotent(&create_idx)?;

        Ok(())
    }

    /// Index an entire repository directory tree.
    ///
    /// Respects include/exclude patterns. Skips unchanged files (file hash match).
    /// Returns aggregate statistics.
    pub async fn index_repository(&self, root: &Path, config: &IndexConfig) -> Result<IndexStats> {
        let mut stats = IndexStats::default();

        let exclude = if config.exclude_patterns.is_empty() {
            language::default_exclude_patterns()
        } else {
            config.exclude_patterns.clone()
        };

        // Collect all candidate files
        let mut files_to_index: Vec<(PathBuf, String)> = Vec::new(); // (path, language_str)

        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !language::is_excluded(e.path(), &exclude))
        {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            if !entry.file_type().is_file() {
                continue;
            }

            let path = entry.path();

            // Detect language — skip unrecognized files
            let lang_str = match language::detect_language(path) {
                Some(l) => l,
                None => continue,
            };

            // Skip non-code files (markdown, json, yaml, toml, etc.)
            if !is_code_language(&lang_str) {
                continue;
            }

            files_to_index.push((path.to_path_buf(), lang_str));
        }

        // Process files in batches for embedding efficiency
        let mut all_chunks: Vec<(CodeChunk, String)> = Vec::new(); // (chunk, file_path_str)

        for (path, lang_str) in &files_to_index {
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue, // skip unreadable files
            };

            if content.is_empty() {
                continue;
            }

            let file_hash = sha256_hex(&content);
            let file_path_str = path.to_string_lossy().to_string();

            // Check if file is unchanged
            if self.file_hash_matches(&file_path_str, &file_hash)? {
                continue; // skip unchanged file
            }

            // Remove old chunks for this file
            self.remove_file_chunks(&file_path_str)?;

            let language = str_to_chunker_language(lang_str);
            let chunks = self.chunker.chunk_file(path, &content, language);

            if chunks.is_empty() {
                continue;
            }

            stats.total_files += 1;
            *stats.languages.entry(lang_str.clone()).or_insert(0) += 1;

            for chunk in chunks {
                all_chunks.push((chunk, file_path_str.clone()));
            }
        }

        // Batch embed and store
        self.embed_and_store_chunks(&all_chunks)?;
        stats.total_chunks = all_chunks.len();

        Ok(stats)
    }

    /// Index a single file (detect language, chunk, embed, store).
    /// Replaces existing chunks for that file if content has changed.
    pub async fn index_file(&self, path: &Path) -> Result<()> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;

        if content.is_empty() {
            return Ok(());
        }

        let lang_str = match language::detect_language(path) {
            Some(l) if is_code_language(&l) => l,
            Some(l) => l, // recognized but non-code: still index for single-file requests
            None => "unknown".to_string(),
        };

        let file_hash = sha256_hex(&content);
        let file_path_str = path.to_string_lossy().to_string();

        // Check if unchanged
        if self.file_hash_matches(&file_path_str, &file_hash)? {
            return Ok(());
        }

        // Remove old chunks
        self.remove_file_chunks(&file_path_str)?;

        let language = str_to_chunker_language(&lang_str);
        let chunks = self.chunker.chunk_file(path, &content, language);

        if chunks.is_empty() {
            return Ok(());
        }

        let paired: Vec<(CodeChunk, String)> = chunks
            .into_iter()
            .map(|c| (c, file_path_str.clone()))
            .collect();

        self.embed_and_store_chunks(&paired)?;
        Ok(())
    }

    /// Remove all chunks for a file from the index.
    pub async fn remove_file(&self, path: &Path) -> Result<()> {
        let file_path_str = path.to_string_lossy().to_string();
        self.remove_file_chunks(&file_path_str)?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Run a CozoScript, ignoring "already exists" / "conflicts" errors.
    fn run_idempotent(&self, script: &str) -> Result<()> {
        match self
            .db
            .run_script(script, Default::default(), ScriptMutability::Mutable)
        {
            Ok(_) => Ok(()),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("already exists")
                    || msg.contains("conflicts")
                    || msg.contains("index with the same name")
                {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("CozoDB script failed: {}", msg))
                }
            }
        }
    }

    /// Check if the stored file hash matches the given hash.
    fn file_hash_matches(&self, file_path: &str, file_hash: &str) -> Result<bool> {
        let mut params = BTreeMap::new();
        params.insert("fp".to_string(), DataValue::from(file_path));
        params.insert("fh".to_string(), DataValue::from(file_hash));

        let result = self
            .db
            .run_script(
                "?[chunk_id] := *code_chunks{chunk_id, file_path, file_hash}, \
             file_path = $fp, file_hash = $fh",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(cozo_err("hash check query"))?;

        Ok(!result.rows.is_empty())
    }

    /// Delete all chunks for a given file path.
    fn remove_file_chunks(&self, file_path: &str) -> Result<()> {
        let mut params = BTreeMap::new();
        params.insert("fp".to_string(), DataValue::from(file_path));

        // Single query: select matching rows and remove them.
        // :rm on an empty result set is a no-op, so no existence check needed.
        self.db.run_script(
            "?[chunk_id, file_path, language, line_start, line_end, chunk_content, file_hash, indexed_at, embedding] := \
             *code_chunks{chunk_id, file_path, language, line_start, line_end, chunk_content, file_hash, indexed_at, embedding}, \
             file_path = $fp
             :rm code_chunks { chunk_id => file_path, language, line_start, line_end, chunk_content, file_hash, indexed_at, embedding }",
            params,
            ScriptMutability::Mutable,
        ).map_err(cozo_err("remove file chunks"))?;

        Ok(())
    }

    /// Embed chunks in batches and store them in CozoDB.
    fn embed_and_store_chunks(&self, chunks: &[(CodeChunk, String)]) -> Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();

        // Process in batches
        for batch in chunks.chunks(EMBED_BATCH_SIZE) {
            let texts: Vec<String> = batch
                .iter()
                .map(|(chunk, _)| chunk.metadata.chunk_content.clone())
                .collect();

            let embeddings = self
                .embedder
                .embed(&texts)
                .map_err(|e| anyhow::anyhow!("embedding failed: {}", e))?;

            if embeddings.len() != batch.len() {
                return Err(anyhow::anyhow!(
                    "embedding count mismatch: got {} for {} chunks",
                    embeddings.len(),
                    batch.len()
                ));
            }

            // Bulk insert this batch
            for (i, (chunk, _file_path_str)) in batch.iter().enumerate() {
                let chunk_id = uuid::Uuid::new_v4().to_string();
                let emb = &embeddings[i];
                let arr = ndarray::Array1::from_vec(emb.clone());

                let mut params = BTreeMap::new();
                params.insert("id".to_string(), DataValue::from(chunk_id.as_str()));
                params.insert(
                    "fp".to_string(),
                    DataValue::from(chunk.metadata.file_path.to_string_lossy().as_ref()),
                );
                params.insert(
                    "lang".to_string(),
                    DataValue::from(chunk.metadata.language.as_str()),
                );
                params.insert(
                    "ls".to_string(),
                    DataValue::from(chunk.metadata.line_start as i64),
                );
                params.insert(
                    "le".to_string(),
                    DataValue::from(chunk.metadata.line_end as i64),
                );
                params.insert(
                    "cc".to_string(),
                    DataValue::from(chunk.metadata.chunk_content.as_str()),
                );
                params.insert(
                    "fh".to_string(),
                    DataValue::from(chunk.metadata.file_hash.as_str()),
                );
                params.insert("ts".to_string(), DataValue::from(now));
                params.insert("emb".to_string(), DataValue::Vec(Vector::F32(arr)));

                self.db.run_script(
                    "?[chunk_id, file_path, language, line_start, line_end, chunk_content, file_hash, indexed_at, embedding] \
                     <- [[$id, $fp, $lang, $ls, $le, $cc, $fh, $ts, $emb]]
                     :put code_chunks { chunk_id => file_path, language, line_start, line_end, chunk_content, file_hash, indexed_at, embedding }",
                    params,
                    ScriptMutability::Mutable,
                ).map_err(cozo_err("insert chunk"))?;
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// Compute SHA-256 hash of content as hex string.
fn sha256_hex(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

/// Map a language string to the chunker's Language enum.
fn str_to_chunker_language(lang: &str) -> Language {
    match lang {
        "rust" => Language::Rust,
        "python" => Language::Python,
        "typescript" | "typescriptreact" => Language::TypeScript,
        "go" => Language::Go,
        _ => Language::Unknown,
    }
}

/// Returns true if the language string represents a programming language
/// (as opposed to config/data formats like json, yaml, toml, markdown).
fn is_code_language(lang: &str) -> bool {
    matches!(
        lang,
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
