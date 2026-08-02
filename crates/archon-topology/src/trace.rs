//! The ambient trace: an append-only jsonl event log per graph.
//!
//! # Why files and not the database
//!
//! The Cozo stores are SQLite-backed behind a process-wide write lock keyed by
//! canonicalized database path (`archon-cozo`). A write from a fan-out worker
//! therefore serializes every other writer in the process, and the guarded
//! retry budget parks a thread for up to ~19 seconds under contention. Tracing
//! is by definition on the hot path of every tool call, so it cannot touch the
//! database at all. It appends to a file instead, and a single batched fold
//! (milestone 2, `src/command/topology_fold.rs`) is the only writer.
//!
//! This crate has no `cozo` dependency, which makes that a compile-time
//! property rather than a convention someone has to remember.
//!
//! # Layout
//!
//! ```text
//! .archon/topology/
//!   <graph-id>/
//!     graph.json      the lowered TaskGraph, or a reconstructed skeleton
//!     trace.jsonl     append-only event log
//!     ingested        marker, written last by a successful fold
//! ```
//!
//! # Concurrency contract
//!
//! [`TraceWriter::append`] serializes the record and its terminating newline
//! into one buffer and issues a single `write` against a handle opened in
//! append mode. Concurrent appenders therefore interleave whole lines and never
//! split one. Note that `archon-workflow`'s `WorkflowStore::append_event_line`
//! — cited by the design as the precedent — does *not* do this; it makes two
//! `write_all` calls, body then newline, and can interleave between them. That
//! is a latent defect there, not a pattern to copy.
//!
//! A reader running concurrently with appends must therefore treat a trailing
//! fragment as absent rather than as corruption; [`read_trace`] does.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ir::{PermissionClass, TaskGraph, WriteTarget};

/// Directory name under `.archon` holding all per-graph trace directories.
pub const TOPOLOGY_DIR: &str = "topology";
/// File name of the persisted graph within a graph directory.
pub const GRAPH_FILE: &str = "graph.json";
/// File name of the append-only event log within a graph directory.
pub const TRACE_FILE: &str = "trace.jsonl";
/// Marker file written last by a successful fold.
pub const INGESTED_MARKER: &str = "ingested";

/// Ceiling on a single serialized record, in bytes.
///
/// Single-`write` append atomicity is a property of small writes; a multi-
/// megabyte tool input would defeat it and would bloat the trace besides.
/// Records are truncated to fit rather than dropped — a lossy record still
/// carries its node attribution, which is what the fold needs.
pub const MAX_RECORD_BYTES: usize = 16 * 1024;

/// Ceiling on any free-text field inside a record.
const MAX_DETAIL_CHARS: usize = 512;

/// What happened.
///
/// Unknown kinds deserialize to [`TraceKind::Unknown`] rather than failing the
/// whole record, so a newer writer's records survive an older reader. The fold
/// skips them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceKind {
    /// The graph was declared up front. Carries no node.
    GraphDeclared,
    NodeStarted,
    NodeFinished,
    AgentSpawned,
    ToolAttempt,
    FileWritten,
    GatePassed,
    Verification,
    Retry,
    /// A kind this build does not know. Never written, only read.
    #[serde(other)]
    Unknown,
}

/// One line of the trace.
///
/// Additive by construction: every field beyond the four the design names is
/// `Option`/defaulted, so a record written by an older build still parses and a
/// new field needs no migration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceRecord {
    /// Timestamp, RFC3339. Supplied by the caller — this crate has no clock
    /// dependency and is not going to acquire one for a string.
    pub ts: String,
    pub graph_id: String,
    /// Node this record attributes to. Empty for graph-level records.
    #[serde(default)]
    pub node_id: String,
    pub kind: TraceKind,
    /// Node that spawned `node_id`. The reconstruction turns these into edges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<PermissionClass>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub blocked: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub error: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writes: Vec<WriteTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    /// Free text, truncated. Never carries tool input verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl TraceRecord {
    /// A record with the mandatory fields set and everything else absent.
    #[must_use]
    pub fn new(ts: impl Into<String>, graph_id: impl Into<String>, kind: TraceKind) -> Self {
        Self {
            ts: ts.into(),
            graph_id: graph_id.into(),
            node_id: String::new(),
            kind,
            parent_node_id: None,
            agent: None,
            tool: None,
            permission: None,
            blocked: false,
            error: false,
            writes: Vec::new(),
            duration_ms: None,
            attempt: None,
            detail: None,
        }
    }

    #[must_use]
    pub fn with_node(mut self, node_id: impl Into<String>) -> Self {
        self.node_id = node_id.into();
        self
    }

    #[must_use]
    pub fn with_parent(mut self, parent_node_id: impl Into<String>) -> Self {
        self.parent_node_id = Some(parent_node_id.into());
        self
    }

    #[must_use]
    pub fn with_agent(mut self, agent: impl Into<String>) -> Self {
        self.agent = Some(agent.into());
        self
    }

    #[must_use]
    pub fn with_tool(mut self, tool: impl Into<String>) -> Self {
        self.tool = Some(tool.into());
        self
    }

    #[must_use]
    pub fn with_permission(mut self, permission: PermissionClass) -> Self {
        self.permission = Some(permission);
        self
    }

    #[must_use]
    pub fn with_outcome(mut self, blocked: bool, error: bool) -> Self {
        self.blocked = blocked;
        self.error = error;
        self
    }

    #[must_use]
    pub fn with_writes(mut self, writes: Vec<WriteTarget>) -> Self {
        self.writes = writes;
        self
    }

    #[must_use]
    pub fn with_duration_ms(mut self, duration_ms: u64) -> Self {
        self.duration_ms = Some(duration_ms);
        self
    }

    #[must_use]
    pub fn with_attempt(mut self, attempt: u32) -> Self {
        self.attempt = Some(attempt);
        self
    }

    /// Attach free text, truncated to [`MAX_DETAIL_CHARS`] on a character
    /// boundary.
    #[must_use]
    pub fn with_detail(mut self, detail: impl AsRef<str>) -> Self {
        self.detail = Some(truncate_chars(detail.as_ref(), MAX_DETAIL_CHARS));
        self
    }
}

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    value.chars().take(limit).collect()
}

/// Resolves the on-disk layout for a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopologyPaths {
    root: PathBuf,
}

impl TopologyPaths {
    /// `<project_root>/.archon/topology`.
    #[must_use]
    pub fn for_project(project_root: &Path) -> Self {
        Self {
            root: project_root.join(".archon").join(TOPOLOGY_DIR),
        }
    }

    /// Use `root` directly as the topology directory. For tests and for callers
    /// that already resolved `.archon` themselves.
    #[must_use]
    pub fn at_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn graph_dir(&self, graph_id: &str) -> PathBuf {
        self.root.join(sanitize_graph_id(graph_id))
    }

    #[must_use]
    pub fn graph_json(&self, graph_id: &str) -> PathBuf {
        self.graph_dir(graph_id).join(GRAPH_FILE)
    }

    #[must_use]
    pub fn trace_jsonl(&self, graph_id: &str) -> PathBuf {
        self.graph_dir(graph_id).join(TRACE_FILE)
    }

    #[must_use]
    pub fn ingested_marker(&self, graph_id: &str) -> PathBuf {
        self.graph_dir(graph_id).join(INGESTED_MARKER)
    }

    /// True when a fold has already completed for this graph.
    #[must_use]
    pub fn is_ingested(&self, graph_id: &str) -> bool {
        self.ingested_marker(graph_id).is_file()
    }

    /// Write the ingested marker. **Call this last** — a crash before it
    /// replays the fold, which is idempotent, whereas a crash after it would
    /// silently lose the graph.
    pub fn mark_ingested(&self, graph_id: &str, note: &str) -> io::Result<()> {
        let dir = self.graph_dir(graph_id);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join(INGESTED_MARKER), note.as_bytes())
    }

    /// Every graph directory present, sorted. Missing root is not an error —
    /// it just means nothing has been traced yet.
    pub fn list_graph_ids(&self) -> io::Result<Vec<String>> {
        let mut ids = Vec::new();
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            if entry.file_type()?.is_dir()
                && let Some(name) = entry.file_name().to_str()
            {
                ids.push(name.to_string());
            }
        }
        ids.sort();
        Ok(ids)
    }

    /// Persist the graph, atomically. Read back by the fold.
    pub fn write_graph(&self, graph: &TaskGraph) -> io::Result<()> {
        let dir = self.graph_dir(&graph.id);
        fs::create_dir_all(&dir)?;
        let encoded = serde_json::to_vec_pretty(graph).map_err(io::Error::other)?;
        let target = dir.join(GRAPH_FILE);
        let temporary = dir.join(format!("{GRAPH_FILE}.tmp"));
        {
            let mut file = File::create(&temporary)?;
            file.write_all(&encoded)?;
            file.sync_all()?;
        }
        fs::rename(&temporary, &target)
    }

    /// Read the persisted graph. `Ok(None)` when the turn never declared one —
    /// the common case, and the reason
    /// [`crate::reconstruct::reconstruct_graph`] exists.
    pub fn read_graph(&self, graph_id: &str) -> io::Result<Option<TaskGraph>> {
        let path = self.graph_json(graph_id);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        match serde_json::from_slice(&bytes) {
            Ok(graph) => Ok(Some(graph)),
            // A half-written graph.json is recoverable: the trace alone is
            // enough to reconstruct a skeleton. Failing the fold here would
            // strand the whole graph.
            Err(_) => Ok(None),
        }
    }

    /// Open an appender for a graph, creating the directory.
    pub fn writer(&self, graph_id: &str) -> io::Result<TraceWriter> {
        TraceWriter::create(&self.graph_dir(graph_id))
    }
}

/// A graph id becomes a directory name, so it must not escape the root or carry
/// separators. Anything outside `[A-Za-z0-9._-]` becomes `_`.
#[must_use]
pub fn sanitize_graph_id(graph_id: &str) -> String {
    let sanitized: String = graph_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    // Separators are already gone, so no id can traverse. What remains to guard
    // is a name that is *entirely* dots — `.` and `..` name the current and
    // parent directory — and a name that begins with a dot, which would create
    // a hidden directory that `list_graph_ids` reports but a human never sees.
    if sanitized.chars().all(|c| c == '.') {
        return "unnamed".to_string();
    }
    if sanitized.starts_with('.') {
        return format!("_{sanitized}");
    }
    sanitized
}

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
    /// `detail` and then `writes`, in that order, rather than being dropped
    /// outright — the node attribution is the part the fold cannot do without.
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
fn encode_record(record: &TraceRecord) -> io::Result<String> {
    let mut candidate = record.clone();
    for shed in 0..3 {
        match shed {
            0 => {}
            1 => candidate.detail = None,
            _ => candidate.writes.clear(),
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

/// What [`read_trace`] recovered.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TraceReadout {
    pub records: Vec<TraceRecord>,
    /// Complete lines that failed to parse. Counted, not fatal.
    pub malformed_lines: usize,
    /// True when the file did not end in a newline, i.e. the last line was
    /// still being written or the process died mid-append. The fragment is
    /// discarded.
    pub truncated_tail: bool,
}

impl TraceReadout {
    /// True when nothing at all was recovered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

/// Read a trace, tolerating truncation and unknown record shapes.
///
/// A missing file yields an empty readout rather than an error: a graph that
/// declared itself and then did nothing is legitimate.
///
/// **Only complete lines are parsed.** A trailing fragment with no newline is
/// discarded and flagged, which is what makes a fold safe to run while workers
/// are still appending — the fragment will be complete on the next read, and
/// the fold is idempotent, so nothing is lost by skipping it now.
pub fn read_trace(path: &Path) -> io::Result<TraceReadout> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(TraceReadout::default());
        }
        // A trace containing invalid UTF-8 is a corrupt trace, not a fatal
        // condition; recover what is decodable.
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            let bytes = fs::read(path)?;
            String::from_utf8_lossy(&bytes).into_owned()
        }
        Err(error) => return Err(error),
    };

    let mut readout = TraceReadout {
        truncated_tail: !contents.is_empty() && !contents.ends_with('\n'),
        ..TraceReadout::default()
    };

    let complete = match contents.rfind('\n') {
        Some(index) => &contents[..=index],
        // No newline anywhere: every byte present is a fragment.
        None => "",
    };

    for line in complete.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<TraceRecord>(line) {
            Ok(record) => readout.records.push(record),
            Err(_) => readout.malformed_lines += 1,
        }
    }

    Ok(readout)
}

#[cfg(test)]
mod tests;
