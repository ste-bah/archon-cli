//! One line of the trace: what a record is and how it is built.
//!
//! Owns [`TraceKind`] and [`TraceRecord`] together with its builder methods.
//! The type is additive by construction — every field beyond the four the
//! design names is `Option`/defaulted — and unknown kinds decode to
//! [`TraceKind::Unknown`] rather than failing the record, so a newer writer's
//! output survives an older reader.

use serde::{Deserialize, Serialize};

use crate::ir::{PermissionClass, WriteTarget};

/// Ceiling on any free-text field inside a record.
pub(super) const MAX_DETAIL_CHARS: usize = 512;

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
    /// A node read a named file. The mirror of [`TraceKind::FileWritten`], and
    /// the record milestone 4's stop-rule fusion lint needs: coupling between
    /// two concurrent nodes is only visible when both halves of the dataflow
    /// are recorded. Added after the trace format shipped, which costs nothing
    /// — the enum is `#[serde(other)]`-tolerant, so an older reader decodes
    /// these to [`TraceKind::Unknown`] and skips them.
    FileRead,
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
    /// Targets the record's node read. Additive, and absent from every record
    /// written before this field existed — which decodes as "nothing observed",
    /// i.e. *unknown*, exactly as [`crate::TaskNode::reads`] requires.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reads: Vec<WriteTarget>,
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
            reads: Vec::new(),
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
    pub fn with_reads(mut self, reads: Vec<WriteTarget>) -> Self {
        self.reads = reads;
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

pub(super) fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    value.chars().take(limit).collect()
}
