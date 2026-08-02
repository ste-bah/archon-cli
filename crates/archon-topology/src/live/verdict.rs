//! What admission is asked, and what it answers.
//!
//! The intent types are crate-local rather than `archon-tools`' request type.
//! The design sketch wrote `on_tool(&self, req: &ToolRunAdmissionRequest)`,
//! which would pull tokio and the whole tool registry into a crate whose
//! dependency budget is petgraph + serde + archon-workflow. The binary owns the
//! translation, and it is a dozen lines.

use crate::ir::PermissionClass;

/// Which invariant a block came from.
///
/// Carried separately from the reason string so a caller can count blocks per
/// invariant, or suppress one, without parsing prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invariant {
    /// Invariant 1 — lifetime agent budget.
    AgentCap,
    /// Invariant 2 — single writer per artifact.
    SingleWriter,
    /// Invariant 3 — irreversible action with no passed gate dominating it.
    UngatedIrreversible,
}

impl Invariant {
    /// Stable identifier, for logs and metrics.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Invariant::AgentCap => "agent_cap",
            Invariant::SingleWriter => "single_writer",
            Invariant::UngatedIrreversible => "ungated_irreversible",
        }
    }
}

/// Admission's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allowed,
    Blocked {
        invariant: Invariant,
        /// Why, in a form the model can act on.
        ///
        /// **Reasons name the conflicting node and the invariant**, because the
        /// model reads them and needs to route *around* the block rather than
        /// retry into it. "Blocked by policy" would produce a retry loop.
        reason: String,
    },
}

impl Verdict {
    /// Build a block.
    pub(super) fn blocked(invariant: Invariant, reason: impl Into<String>) -> Self {
        Verdict::Blocked {
            invariant,
            reason: reason.into(),
        }
    }

    #[must_use]
    pub fn is_blocked(&self) -> bool {
        matches!(self, Verdict::Blocked { .. })
    }

    #[must_use]
    pub fn is_allowed(&self) -> bool {
        matches!(self, Verdict::Allowed)
    }

    /// The reason text, if this is a block.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Verdict::Allowed => None,
            Verdict::Blocked { reason, .. } => Some(reason),
        }
    }

    /// The invariant that produced this block, if any.
    #[must_use]
    pub fn invariant(&self) -> Option<Invariant> {
        match self {
            Verdict::Allowed => None,
            Verdict::Blocked { invariant, .. } => Some(*invariant),
        }
    }
}

/// A subagent about to be launched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnIntent {
    /// Node the child will be recorded as.
    pub node_id: String,
    /// Node launching it, if known. Used only to record the dependency edge, so
    /// that a later write conflict between parent and child is correctly
    /// recognised as *related* and admitted.
    pub parent_id: Option<String>,
    /// Agent type, for the block reason.
    pub agent: String,
}

/// A write about to happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteIntent {
    /// Node performing the write.
    pub node_id: String,
    /// Raw declared paths or globs. Normalisation and overlap are
    /// `archon-workflow`'s write coordinator's business, not this crate's.
    pub paths: Vec<String>,
}

/// A tool call about to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolIntent {
    /// Node making the call. For an undeclared turn this is the turn root.
    pub node_id: String,
    /// Tool name, for the block reason.
    pub tool: String,
    /// The tool's declared permission, already mapped onto the IR's class. The
    /// mapping is `archon_core::orchestrator::topology::permission_class_for_level`
    /// and the declared spec-side format is [`crate::permission`]; both resolve
    /// to the same three values.
    pub permission: PermissionClass,
    /// Paths the call declares it writes, if any. Empty means *unknown*, not
    /// *none* — see the unknown-dataflow rule — so an empty list runs no
    /// single-writer check rather than asserting the call writes nothing.
    pub writes: Vec<String>,
    /// Set when this call launches a subagent.
    pub spawn: Option<SpawnIntent>,
}

impl ToolIntent {
    /// A minimal intent: a node calling a tool at a permission class.
    #[must_use]
    pub fn new(
        node_id: impl Into<String>,
        tool: impl Into<String>,
        permission: PermissionClass,
    ) -> Self {
        Self {
            node_id: node_id.into(),
            tool: tool.into(),
            permission,
            writes: Vec::new(),
            spawn: None,
        }
    }

    /// Declare the paths this call writes.
    #[must_use]
    pub fn with_writes(mut self, paths: Vec<String>) -> Self {
        self.writes = paths;
        self
    }

    /// Declare that this call launches a subagent.
    #[must_use]
    pub fn with_spawn(mut self, spawn: SpawnIntent) -> Self {
        self.spawn = Some(spawn);
        self
    }
}
