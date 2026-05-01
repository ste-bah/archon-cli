//! KB document ingest — markdown, text, directory scanning.
//!
//! Implements REQ-KB-001. Heading-aware chunking, SHA-256 deduplication,
//! batch storage in CozoDB.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};
use sha2::{Digest, Sha256};

use super::schema::KbNodeType;
use super::{IngestResult, IngestSource};

// ---------------------------------------------------------------------------
// Ingester
// ---------------------------------------------------------------------------

/// Document ingester for the knowledge base.
pub struct Ingester {
    db: DbInstance,
}

impl Ingester {
    /// Create a new ingester backed by the given CozoDB instance.
    ///
    /// Assumes `ensure_kb_schema()` has already been called.
    pub fn new(db: DbInstance) -> Result<Self> {
        Ok(Self { db })
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

    /// Ingest a PDF file. Splits at page boundaries.
    pub async fn ingest_pdf(&self, path: &Path, domain_tag: &str) -> Result<IngestResult> {
        // PDF reading requires external tooling; for now extract as text if possible.
        // Falls back to treating the file as text (will produce garbled output for
        // binary PDFs, but won't crash).
        let content = std::fs::read_to_string(path).unwrap_or_default();
        if content.is_empty() {
            return Ok(IngestResult::default());
        }

        let source = path.to_string_lossy().to_string();
        let chunks = chunk_text_paragraphs(&content);

        self.store_chunks(&chunks, &source, domain_tag).await
    }

    /// Ingest from a URL: fetch HTML, convert to markdown, then process.
    pub async fn ingest_url(&self, url: &str, domain_tag: &str) -> Result<IngestResult> {
        // URL fetching is deferred to integration with archon-cli's WebFetch.
        // For now, return empty result without error.
        let _ = (url, domain_tag);
        Ok(IngestResult::default())
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
        let mut result = IngestResult {
            chunks_processed: chunks.len(),
            ..Default::default()
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();

        for (idx, chunk) in chunks.iter().enumerate() {
            let content_hash = sha256_hex(&chunk.content);

            // Check for duplicate by content_hash
            if self.hash_exists(&content_hash)? {
                continue; // skip exact duplicate
            }

            let node_id = uuid::Uuid::new_v4().to_string();
            let node_type = node_type_str(&KbNodeType::Raw);

            let mut params = BTreeMap::new();
            params.insert("nid".to_string(), DataValue::from(node_id.as_str()));
            params.insert("ntype".to_string(), DataValue::from(node_type));
            params.insert("source".to_string(), DataValue::from(source));
            params.insert("dtag".to_string(), DataValue::from(domain_tag));
            params.insert("title".to_string(), DataValue::from(chunk.title.as_str()));
            params.insert(
                "content".to_string(),
                DataValue::from(chunk.content.as_str()),
            );
            params.insert("chash".to_string(), DataValue::from(content_hash.as_str()));
            params.insert("cidx".to_string(), DataValue::from(idx as i64));
            params.insert("cat".to_string(), DataValue::from(now));
            params.insert("uat".to_string(), DataValue::from(now));

            self.db
                .run_script(
                    "?[node_id, node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at] \
                     <- [[$nid, $ntype, $source, $dtag, $title, $content, $chash, $cidx, $cat, $uat]]
                     :put kb_nodes { node_id => node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at }",
                    params,
                    ScriptMutability::Mutable,
                )
                .map_err(|e| anyhow::anyhow!("insert kb_node failed: {}", e))?;

            result.nodes_created += 1;
        }

        Ok(result)
    }

    /// Check if a content hash already exists in kb_nodes.
    fn hash_exists(&self, content_hash: &str) -> Result<bool> {
        let mut params = BTreeMap::new();
        params.insert("ch".to_string(), DataValue::from(content_hash));

        let result = self
            .db
            .run_script(
                "?[node_id] := *kb_nodes{node_id, content_hash}, content_hash = $ch",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(|e| anyhow::anyhow!("hash check failed: {}", e))?;

        Ok(!result.rows.is_empty())
    }
}

// ---------------------------------------------------------------------------
// Chunking functions
// ---------------------------------------------------------------------------

/// A chunk of document content ready for storage.
struct ChunkData {
    title: String,
    content: String,
}

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

/// Convert KbNodeType to string for CozoDB storage.
fn node_type_str(t: &KbNodeType) -> &'static str {
    match t {
        KbNodeType::Raw => "raw",
        KbNodeType::Compiled => "compiled",
        KbNodeType::Concept => "concept",
        KbNodeType::Answer => "answer",
        KbNodeType::Index => "index",
    }
}
