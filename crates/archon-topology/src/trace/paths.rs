//! The on-disk layout, and the only thing allowed to name a file in it.
//!
//! Owns the directory and file-name constants, [`TopologyPaths`] which resolves
//! them for a project, and [`sanitize_graph_id`], which is the boundary that
//! keeps a graph id from escaping the topology root.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::ir::TaskGraph;

use super::writer::TraceWriter;

/// Directory name under `.archon` holding all per-graph trace directories.
pub const TOPOLOGY_DIR: &str = "topology";
/// File name of the persisted graph within a graph directory.
pub const GRAPH_FILE: &str = "graph.json";
/// File name of the append-only event log within a graph directory.
pub const TRACE_FILE: &str = "trace.jsonl";
/// Marker file written last by a successful fold.
pub const INGESTED_MARKER: &str = "ingested";

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
