//! Errors raised by graph construction.
//!
//! Hand-rolled rather than `thiserror`-derived: milestone 1 pins this crate's
//! unconditional dependency set to petgraph + serde, and a `Display` impl is
//! cheaper than widening it.

use std::fmt;

/// A structural defect in a [`crate::TaskGraph`].
///
/// These are the only failure modes in the crate — every analysis is otherwise
/// total, and none performs I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopologyError {
    /// Two nodes share an id, so `depends_on` cannot be resolved unambiguously.
    DuplicateNode { id: String },
    /// A node depends on an id that is not in the graph.
    UnknownDependency { node: String, dependency: String },
    /// `depends_on` contains a cycle, so no topological order exists.
    Cycle,
}

impl fmt::Display for TopologyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateNode { id } => {
                write!(f, "duplicate node id '{id}' in task graph")
            }
            Self::UnknownDependency { node, dependency } => {
                write!(f, "node '{node}' depends on unknown node '{dependency}'")
            }
            Self::Cycle => write!(f, "dependency cycle detected in task graph"),
        }
    }
}

impl std::error::Error for TopologyError {}
