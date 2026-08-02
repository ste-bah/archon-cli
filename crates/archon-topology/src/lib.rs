//! Topology IR and pure graph analyses.
//!
//! Archon represents "a graph of work" on three surfaces — `/workflow` specs,
//! team subtask lists, and ordinary coding turns — and only the first was ever
//! governed. This crate is the one intermediate representation all three lower
//! into, so analysis is written once instead of three times.
//!
//! # Scope
//!
//! Milestone 1: the IR, the `WorkflowSpec` lowering, and five pure analyses.
//! No storage, no trace, no admission — those are milestones 2 and 3, and the
//! crate's dependency set is deliberately too small to reach them. There is no
//! `cozo`, no `archon-core`, and no learning crate here, so the analyses stay
//! usable in contexts with no database at all.
//!
//! The `Vec<Subtask>` lowering lives in `archon-core`
//! (`orchestrator::topology`) rather than here, because `Subtask` is
//! `archon-core`'s type: putting it here would invert the dependency edge.
//!
//! # The unknown-dataflow rule
//!
//! [`TaskNode::consumes`] being empty means *unknown*, not *nothing*. The
//! subtask lowering has no dataflow to give and the session lowering will not
//! either, so empty is the common case, not the exception. Any analysis
//! reasoning from dataflow must treat empty as "cannot conclude" and stay
//! silent. The same applies to [`TaskNode::writes`], which is why
//! [`TaskGraph::write_conflicts`] skips nodes with no declared targets rather
//! than treating them as conflict-free.

pub mod analysis;
pub mod error;
mod index;
pub mod ir;
#[cfg(feature = "workflow")]
pub mod lower_workflow;

pub use analysis::{CriticalPath, ParallelismProfile, WriteConflict};
pub use error::TopologyError;
pub use ir::{
    DataRef, FanoutSpec, GateKind, GraphBudget, GraphOrigin, NodeRole, PermissionClass, TaskGraph,
    TaskNode, WriteTarget,
};
#[cfg(feature = "workflow")]
pub use lower_workflow::lower_workflow_spec;
