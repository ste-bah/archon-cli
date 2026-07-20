//! KB document ingest — markdown, text, directory scanning.
//!
//! Implements REQ-KB-001. Heading-aware chunking, SHA-256 deduplication,
//! batch storage in CozoDB.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use archon_docs::embed::LocalEmbeddingProvider;
use cozo::DbInstance;
use sha2::{Digest, Sha256};

use super::ingest_storage::{ChunkData, ChunkStorage};
use super::{IngestResult, IngestSource};

// ---------------------------------------------------------------------------
// Ingester
// ---------------------------------------------------------------------------

/// Document ingester for the knowledge base.
pub struct Ingester {
    storage: ChunkStorage,
    embedder: Option<Arc<dyn LocalEmbeddingProvider>>,
}

impl Ingester {
    /// Create a new ingester backed by the given CozoDB instance.
    ///
    /// Assumes `ensure_kb_schema()` has already been called.
    pub fn new(db: DbInstance) -> Result<Self> {
        Ok(Self {
            storage: ChunkStorage::new(db),
            embedder: None,
        })
    }

    pub fn with_embedder(
        db: DbInstance,
        embedder: Arc<dyn LocalEmbeddingProvider>,
    ) -> Result<Self> {
        let _guard = super::schema::lock_embedding_state()?;
        let existing = read_node_content(&db)?;
        let content: Vec<_> = existing
            .iter()
            .map(|(_, content)| content.clone())
            .collect();
        let vectors = embedder.embed_chunks(&content)?;
        if vectors.len() != existing.len() {
            anyhow::bail!(
                "KB embedder returned {} vectors for {} existing nodes",
                vectors.len(),
                existing.len()
            );
        }
        let embeddings: Vec<_> = existing
            .into_iter()
            .zip(vectors)
            .map(|((node_id, _), embedding)| (node_id, embedding))
            .collect();
        super::schema::ensure_kb_embedding_schema_locked(
            &db,
            &embedder.embedding_space_id(),
            embedder.dimension(),
            Some(&embeddings),
        )?;
        let storage = ChunkStorage::new(db.clone());
        super::schema::assert_embedding_space(
            &db,
            &embedder.embedding_space_id(),
            embedder.dimension(),
        )?;
        backfill_missing_embeddings(&db, embedder.as_ref())?;
        Ok(Self {
            storage,
            embedder: Some(embedder),
        })
    }

    /// Dispatch to source-specific handler.
    pub async fn ingest(
        &self,
        source: &IngestSource,
        domain_tag: Option<&str>,
    ) -> Result<IngestResult> {
        let tag = domain_tag.unwrap_or("default");
        match source {
            IngestSource::FilePath(path) => {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                match ext {
                    "md" | "markdown" => self.ingest_markdown(path, tag).await,
                    "pdf" => self.ingest_pdf(path, tag).await,
                    "txt" => self.ingest_text(path, tag).await,
                    _ => self.ingest_text(path, tag).await, // fallback to text
                }
            }
            IngestSource::Url(url) => self.ingest_url(url, tag).await,
            IngestSource::Directory(dir) => self.ingest_directory(dir, tag, None).await,
        }
    }

    /// Ingest a markdown file using heading-aware chunking.
    ///
    /// Splits at `#` headings: each chunk = heading + content until next
    /// heading of same or higher level.
    pub async fn ingest_markdown(&self, path: &Path, domain_tag: &str) -> Result<IngestResult> {
        let content = std::fs::read_to_string(path)?;
        if content.is_empty() {
            return Ok(IngestResult::default());
        }

        let source = path.to_string_lossy().to_string();
        let chunks = chunk_markdown(&content);

        self.store_chunks(&chunks, &source, domain_tag).await
    }

    /// Ingest a `.pdf` path only when its contents are valid UTF-8 text.
    /// Binary PDF extraction is not implemented and returns a read error.
    pub async fn ingest_pdf(&self, path: &Path, domain_tag: &str) -> Result<IngestResult> {
        let content = std::fs::read_to_string(path)
            .map_err(|error| anyhow::anyhow!("PDF text ingestion failed: {error}"))?;
        if content.is_empty() {
            return Ok(IngestResult::default());
        }

        let source = path.to_string_lossy().to_string();
        let chunks = chunk_text_paragraphs(&content);

        self.store_chunks(&chunks, &source, domain_tag).await
    }

    /// Return an explicit error because URL ingestion is not implemented.
    pub async fn ingest_url(&self, url: &str, domain_tag: &str) -> Result<IngestResult> {
        let _ = (url, domain_tag);
        anyhow::bail!("URL ingestion is not supported")
    }

    /// Ingest a plain text file. Splits at blank-line paragraphs.
    pub async fn ingest_text(&self, path: &Path, domain_tag: &str) -> Result<IngestResult> {
        let content = std::fs::read_to_string(path)?;
        if content.is_empty() {
            return Ok(IngestResult::default());
        }

        let source = path.to_string_lossy().to_string();
        let chunks = chunk_text_paragraphs(&content);

        self.store_chunks(&chunks, &source, domain_tag).await
    }

    /// Ingest all supported files from a directory tree.
    pub async fn ingest_directory(
        &self,
        dir: &Path,
        domain_tag: &str,
        _patterns: Option<&[String]>,
    ) -> Result<IngestResult> {
        let mut combined = IngestResult::default();

        for entry in walkdir(dir) {
            let ext = entry.extension().and_then(|e| e.to_str()).unwrap_or("");

            let result = match ext {
                "md" | "markdown" => self.ingest_markdown(&entry, domain_tag).await?,
                "txt" => self.ingest_text(&entry, domain_tag).await?,
                _ => continue, // skip unsupported file types
            };

            combined.nodes_created += result.nodes_created;
            combined.chunks_processed += result.chunks_processed;
            combined.errors.extend(result.errors);
        }

        Ok(combined)
    }

    // -----------------------------------------------------------------------
    // Internal
    // -----------------------------------------------------------------------

    /// Store chunks in CozoDB, deduplicating by content hash.
    async fn store_chunks(
        &self,
        chunks: &[ChunkData],
        source: &str,
        domain_tag: &str,
    ) -> Result<IngestResult> {
        let _guard = self
            .embedder
            .as_ref()
            .map(|_| super::schema::lock_embedding_state())
            .transpose()?;
        if let Some(embedder) = &self.embedder {
            super::schema::assert_embedding_space(
                self.storage.db(),
                &embedder.embedding_space_id(),
                embedder.dimension(),
            )?;
        }
        let content: Vec<_> = chunks.iter().map(|chunk| chunk.content.clone()).collect();
        let embeddings = self
            .embedder
            .as_ref()
            .map(|embedder| embedder.embed_chunks(&content))
            .transpose()?;
        self.storage.store(
            chunks,
            embeddings.as_deref(),
            source,
            domain_tag,
            sha256_hex,
        )
    }

    #[doc(hidden)]
    pub fn fail_next_batch_after_hash_write_for_tests(&self) {
        self.storage.fail_next_batch_after_hash_write_for_tests();
    }

    #[doc(hidden)]
    pub fn transaction_count_for_tests(&self) -> usize {
        self.storage.transaction_count_for_tests()
    }
}

pub(super) fn read_node_content(db: &DbInstance) -> Result<Vec<(String, String)>> {
    let result = db
        .run_script(
            "?[node_id, content] := *kb_nodes{node_id, content}",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        )
        .map_err(|error| anyhow::anyhow!("read KB nodes for embedding failed: {error}"))?;
    Ok(result
        .rows
        .iter()
        .map(|row| {
            (
                row[0].get_str().unwrap_or_default().to_string(),
                row[1].get_str().unwrap_or_default().to_string(),
            )
        })
        .collect())
}

pub(super) fn backfill_missing_embeddings(
    db: &DbInstance,
    embedder: &dyn LocalEmbeddingProvider,
) -> Result<()> {
    let result = db
        .run_script(
            "?[node_id, content] := *kb_nodes{node_id, content}, \
             not *kb_embeddings{node_id}",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        )
        .map_err(|error| anyhow::anyhow!("read unindexed KB nodes failed: {error}"))?;
    for batch in result
        .rows
        .chunks(super::ingest_storage::KB_INGEST_BATCH_SIZE)
    {
        let content: Vec<_> = batch
            .iter()
            .map(|row| row[1].get_str().unwrap_or_default().to_string())
            .collect();
        let embeddings = embedder.embed_chunks(&content)?;
        store_backfill_batch(db, batch, &embeddings)?;
    }
    Ok(())
}

fn store_backfill_batch(
    db: &DbInstance,
    nodes: &[Vec<cozo::DataValue>],
    embeddings: &[Vec<f32>],
) -> Result<()> {
    use cozo::{DataValue, Vector};
    use ndarray::Array1;
    if nodes.len() != embeddings.len() {
        anyhow::bail!(
            "KB embedder returned {} vectors for {} existing nodes",
            embeddings.len(),
            nodes.len()
        );
    }
    let rows = nodes
        .iter()
        .zip(embeddings)
        .map(|(node, embedding)| {
            DataValue::List(vec![
                node[0].clone(),
                DataValue::Vec(Vector::F32(Array1::from_vec(embedding.clone()))),
            ])
        })
        .collect();
    let mut params = std::collections::BTreeMap::new();
    params.insert("rows".to_string(), DataValue::List(rows));
    db.run_script(
        "?[node_id, embedding] <- $rows\n         :put kb_embeddings { node_id => embedding }",
        params,
        cozo::ScriptMutability::Mutable,
    )
    .map_err(|error| anyhow::anyhow!("backfill KB embeddings failed: {error}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Chunking functions
// ---------------------------------------------------------------------------

/// Split markdown content at `#` headings.
///
/// Each chunk = heading text + all content until the next heading of same
/// or higher level. Content before the first heading becomes an "intro" chunk.
fn chunk_markdown(content: &str) -> Vec<ChunkData> {
    let mut chunks = Vec::new();
    let mut current_title = String::new();
    let mut current_content = String::new();

    for line in content.lines() {
        if line.starts_with('#') {
            // Flush previous chunk
            if !current_content.is_empty() || !current_title.is_empty() {
                let title = if current_title.is_empty() {
                    "Introduction".to_string()
                } else {
                    current_title.clone()
                };
                let text = format!("{}\n{}", title, current_content.trim());
                chunks.push(ChunkData {
                    title,
                    content: text.trim().to_string(),
                });
            }

            // Extract heading text (strip # prefix and whitespace)
            current_title = line.trim_start_matches('#').trim().to_string();
            current_content.clear();
        } else {
            current_content.push_str(line);
            current_content.push('\n');
        }
    }

    // Flush last chunk
    if !current_content.is_empty() || !current_title.is_empty() {
        let title = if current_title.is_empty() {
            "Introduction".to_string()
        } else {
            current_title.clone()
        };
        let text = format!("{}\n{}", title, current_content.trim());
        chunks.push(ChunkData {
            title,
            content: text.trim().to_string(),
        });
    }

    chunks
}

/// Split plain text at double newlines (blank lines).
///
/// Merges small paragraphs to ensure minimum ~200 chars per chunk.
fn chunk_text_paragraphs(content: &str) -> Vec<ChunkData> {
    let paragraphs: Vec<&str> = content
        .split("\n\n")
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();

    let mut chunks = Vec::new();
    let mut buffer = String::new();
    let mut para_idx = 0;

    for para in &paragraphs {
        if buffer.is_empty() {
            buffer.push_str(para);
        } else {
            buffer.push_str("\n\n");
            buffer.push_str(para);
        }

        // Flush when buffer is large enough or this is the last paragraph
        if buffer.len() >= 200 || para == paragraphs.last().unwrap() {
            let title = format!("Paragraph {}", para_idx + 1);
            chunks.push(ChunkData {
                title,
                content: buffer.clone(),
            });
            buffer.clear();
            para_idx += 1;
        }
    }

    // Flush any remaining buffer
    if !buffer.is_empty() {
        let title = format!("Paragraph {}", para_idx + 1);
        chunks.push(ChunkData {
            title,
            content: buffer,
        });
    }

    chunks
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Walk a directory tree, returning all file paths (non-recursive on hidden dirs).
fn walkdir(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    walk_recursive(dir, &mut files);
    files
}

fn walk_recursive(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip hidden directories
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            walk_recursive(&path, files);
        } else if path.is_file() {
            files.push(path);
        }
    }
}

/// Compute SHA-256 hash of content as hex string.
fn sha256_hex(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}
