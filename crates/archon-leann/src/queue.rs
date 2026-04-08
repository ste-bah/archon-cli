//! Indexing process queue — batch index files from a JSON queue.
//!
//! Implements REQ-LEANN-006.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::indexer::Indexer;
use crate::metadata::QueueResult;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A single entry in the queue JSON file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueEntry {
    pub file_path: String,
    pub added_at: String,
    #[serde(default)]
    pub error_count: u32,
    #[serde(default)]
    pub last_error: Option<String>,
}

/// Status of the queue without processing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueStatus {
    pub pending: usize,
    pub dead_letter: usize,
    pub files: Vec<String>,
}

/// Maximum consecutive failures before a file is moved to dead-letter status.
const MAX_RETRIES: u32 = 3;

// ---------------------------------------------------------------------------
// QueueProcessor
// ---------------------------------------------------------------------------

/// Processes the indexing queue.
pub struct QueueProcessor<'a> {
    indexer: &'a Indexer,
}

impl<'a> QueueProcessor<'a> {
    pub fn new(indexer: &'a Indexer) -> Self {
        Self { indexer }
    }

    /// Read the JSON queue, attempt to index each pending file, track
    /// success/failure, write back the remaining entries (failed + dead-lettered),
    /// and return a [`QueueResult`].
    pub async fn process_queue(&self, queue_path: &Path) -> Result<QueueResult> {
        // 1. Non-existent queue file → no work.
        if !queue_path.exists() {
            return Ok(QueueResult::default());
        }

        // 2. Read and parse JSON (handle empty file as empty array).
        let content = std::fs::read_to_string(queue_path)
            .with_context(|| format!("failed to read queue file: {}", queue_path.display()))?;

        let mut entries: Vec<QueueEntry> = if content.trim().is_empty() {
            Vec::new()
        } else {
            serde_json::from_str(&content)
                .with_context(|| format!("failed to parse queue JSON: {}", queue_path.display()))?
        };

        if entries.is_empty() {
            return Ok(QueueResult::default());
        }

        let mut processed = 0usize;
        let mut failed = 0usize;

        // 3. Process each entry that is below the dead-letter threshold.
        for entry in entries.iter_mut() {
            if entry.error_count >= MAX_RETRIES {
                // Already dead-lettered; skip without processing.
                continue;
            }

            let path = Path::new(&entry.file_path);
            match self.indexer.index_file(path).await {
                Ok(()) => {
                    processed += 1;
                    // Mark as successfully processed so we can filter it out below.
                    // We use a sentinel: set error_count to u32::MAX so we can
                    // distinguish "succeeded" from "failed / dead-letter".
                    entry.error_count = u32::MAX;
                }
                Err(err) => {
                    failed += 1;
                    entry.error_count += 1;
                    entry.last_error = Some(err.to_string());
                }
            }
        }

        // 4. Write back remaining entries:
        //    - Exclude successfully processed files (error_count == u32::MAX).
        //    - Keep failed (error_count 1..MAX_RETRIES) and dead-lettered (>= MAX_RETRIES).
        let remaining_entries: Vec<&QueueEntry> = entries
            .iter()
            .filter(|e| e.error_count != u32::MAX)
            .collect();

        let remaining = remaining_entries.len();

        write_queue_atomic(queue_path, &remaining_entries)?;

        Ok(QueueResult {
            processed,
            failed,
            remaining,
        })
    }

    /// Append file paths to the queue file.
    ///
    /// Creates the queue file if it does not exist.  Uses an atomic
    /// write (temp file + rename) to prevent corruption from concurrent writes.
    pub fn add_to_queue(queue_path: &Path, file_paths: &[PathBuf]) -> Result<()> {
        if file_paths.is_empty() {
            return Ok(());
        }

        // 1. Read existing entries (empty vec if file doesn't exist or is empty).
        let mut entries: Vec<QueueEntry> = if queue_path.exists() {
            let content = std::fs::read_to_string(queue_path)
                .with_context(|| format!("failed to read queue: {}", queue_path.display()))?;
            if content.trim().is_empty() {
                Vec::new()
            } else {
                serde_json::from_str(&content)
                    .with_context(|| format!("failed to parse queue: {}", queue_path.display()))?
            }
        } else {
            Vec::new()
        };

        // 2. Append new entries.
        let now = now_iso8601();
        for path in file_paths {
            entries.push(QueueEntry {
                file_path: path.to_string_lossy().into_owned(),
                added_at: now.clone(),
                error_count: 0,
                last_error: None,
            });
        }

        // 3. Atomic write.
        let refs: Vec<&QueueEntry> = entries.iter().collect();
        write_queue_atomic(queue_path, &refs)?;

        Ok(())
    }

    /// Return the count and file list of the queue without processing it.
    ///
    /// Returns immediately with zeros if the queue file does not exist.
    pub fn queue_status(queue_path: &Path) -> Result<QueueStatus> {
        if !queue_path.exists() {
            return Ok(QueueStatus {
                pending: 0,
                dead_letter: 0,
                files: Vec::new(),
            });
        }

        let content = std::fs::read_to_string(queue_path)
            .with_context(|| format!("failed to read queue: {}", queue_path.display()))?;

        let entries: Vec<QueueEntry> = if content.trim().is_empty() {
            Vec::new()
        } else {
            serde_json::from_str(&content)
                .with_context(|| format!("failed to parse queue: {}", queue_path.display()))?
        };

        let mut pending = 0usize;
        let mut dead_letter = 0usize;
        let mut files = Vec::new();

        for entry in &entries {
            if entry.error_count >= MAX_RETRIES {
                dead_letter += 1;
            } else {
                pending += 1;
                files.push(entry.file_path.clone());
            }
        }

        Ok(QueueStatus {
            pending,
            dead_letter,
            files,
        })
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Atomically write queue entries to `path` (write to a `.tmp` sibling, then rename).
fn write_queue_atomic(path: &Path, entries: &[&QueueEntry]) -> Result<()> {
    let json =
        serde_json::to_string_pretty(entries).context("failed to serialize queue entries")?;

    // Determine a temp path in the same directory so rename is atomic.
    let tmp_path = path.with_extension("json.tmp");

    // Ensure parent directory exists.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create queue directory: {}", parent.display()))?;
    }

    std::fs::write(&tmp_path, &json)
        .with_context(|| format!("failed to write tmp queue: {}", tmp_path.display()))?;

    std::fs::rename(&tmp_path, path)
        .with_context(|| format!("failed to rename tmp queue to {}", path.display()))?;

    Ok(())
}

/// Return the current UTC time as an ISO 8601 string.
fn now_iso8601() -> String {
    chrono::Utc::now().to_rfc3339()
}
