//! The append side, and the single-`write` guarantee it rests on.
//!
//! Owns [`TraceWriter`] and the encoder that keeps a line small enough for that
//! guarantee to hold. The writer holds a path rather than a handle and reopens
//! in append mode per record, which is what lets N workers append with no lock.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::paths::TRACE_FILE;
use super::record::{TraceRecord, truncate_chars};

/// Ceiling on a single serialized record, in bytes.
///
/// Single-`write` append atomicity is a property of small writes; a multi-
/// megabyte tool input would defeat it and would bloat the trace besides.
/// Records are truncated to fit rather than dropped — a lossy record still
/// carries its node attribution, which is what the fold needs.
pub const MAX_RECORD_BYTES: usize = 16 * 1024;

/// Append-only writer for one graph's `trace.jsonl`.
///
/// Cheap to clone and safe to share: it holds a path, not a handle, and opens
/// in append mode per record. Holding a long-lived handle would be marginally
/// faster and would break the "N workers, no lock" property on Windows, where a
/// handle's file pointer is not shared the way an inherited fd is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceWriter {
    path: PathBuf,
}

impl TraceWriter {
    /// Create the graph directory and return a writer for `trace.jsonl` inside.
    pub fn create(graph_dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(graph_dir)?;
        Ok(Self {
            path: graph_dir.join(TRACE_FILE),
        })
    }

    /// Point at an explicit file. No directory is created.
    #[must_use]
    pub fn at_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one record.
    ///
    /// The line and its terminator are built in memory and handed to a single
    /// `write_all` against an append-mode handle, so the operating system sees
    /// one `write` carrying a complete line. That is what lets N workers append
    /// concurrently with no lock and no interleaving.
    ///
    /// Over-long records are truncated to [`MAX_RECORD_BYTES`] by dropping
    /// `detail` and then the target lists, in that order, rather than being
    /// dropped outright — the node attribution is the part the fold cannot do
    /// without.
    pub fn append(&self, record: &TraceRecord) -> io::Result<()> {
        let line = encode_record(record)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(line.as_bytes())
    }
}

/// Serialize a record to one newline-terminated line, shedding optional fields
/// until it fits.
pub(super) fn encode_record(record: &TraceRecord) -> io::Result<String> {
    let mut candidate = record.clone();
    for shed in 0..3 {
        match shed {
            0 => {}
            1 => candidate.detail = None,
            // Both target lists go together. Shedding one and keeping the other
            // would leave a record asserting "this node wrote X and read
            // nothing", which the dataflow lints would believe.
            _ => {
                candidate.writes.clear();
                candidate.reads.clear();
            }
        }
        let mut line = serde_json::to_string(&candidate).map_err(io::Error::other)?;
        // `<` rather than `<=`: the newline has yet to be pushed and counts
        // toward the budget.
        if line.len() < MAX_RECORD_BYTES {
            line.push('\n');
            return Ok(line);
        }
    }
    // Everything optional is gone and it still does not fit: the mandatory
    // fields alone are oversized. Emit a minimal record so the node is not lost.
    let mut minimal = TraceRecord::new(
        truncate_chars(&record.ts, 64),
        truncate_chars(&record.graph_id, 256),
        record.kind,
    );
    minimal.node_id = truncate_chars(&record.node_id, 256);
    let mut line = serde_json::to_string(&minimal).map_err(io::Error::other)?;
    line.push('\n');
    Ok(line)
}
