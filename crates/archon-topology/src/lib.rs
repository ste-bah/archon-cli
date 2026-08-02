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
//!
//! Milestone 4 adds three advisory lints beside them —
//! [`TaskGraph::diamond_conformance`], [`TaskGraph::classify_edges`], and
//! [`TaskGraph::stop_rule_fusion`]. They are analyses, not gates: each returns
//! findings with a remedy string and none of them can fail a run.
//!
//! Milestone 3 adds [`live`]: the per-session executed prefix and the three
//! guardrail invariants evaluated against it. It is still database-free — the
//! whole point is that admission runs on the synchronous critical path of every
//! non-`Safe` tool call, where a Cozo read would take a lock and a Cozo write
//! would take the process-wide write lock. As with the trace, that is enforced
//! by this crate being unable to reach a database rather than by a convention.
//!
//! Milestone 2 adds the ambient trace ([`trace`]), post-hoc graph
//! reconstruction ([`reconstruct`]), and the canonical [`task_hash`]. It does
//! **not** add storage: there is still no `cozo`, no `archon-core`, and no
//! learning crate here. The trace is jsonl and the fold that reads it into Cozo
//! lives above this crate in the binary's `src/command/topology_fold.rs`. That
//! is the whole reason the dependency set is policed — "no database write on a
//! hot path" is enforced by this crate being unable to reach a database, not by
//! a convention.
//!
//! Admission is milestone 3 and is not here.
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
#[cfg(feature = "live")]
pub mod live;
#[cfg(feature = "workflow")]
pub mod lower_workflow;
pub mod permission;
#[cfg(feature = "trace")]
pub mod reconstruct;
pub mod task_hash;
#[cfg(feature = "trace")]
pub mod trace;

pub use analysis::{
    ClassifiedEdge, CoupledPair, CriticalPath, DiamondFinding, DiamondReport, EdgeSupport,
    FusibleChain, FusionKind, FusionReport, LikelyCause, ParallelismProfile, VerifierDiversity,
    WriteConflict,
};
pub use error::TopologyError;
pub use ir::{
    DataRef, FanoutSpec, GateKind, GraphBudget, GraphOrigin, NodeRole, PermissionClass, TaskGraph,
    TaskNode, WriteTarget,
};
#[cfg(feature = "live")]
pub use live::{
    Invariant, LiveTopology, LiveTopologyConfig, SpawnIntent, ToolIntent, Verdict, WriteIntent,
};
#[cfg(feature = "workflow")]
pub use lower_workflow::lower_workflow_spec;
pub use permission::{DECLARED_PERMISSION_LEVELS, is_declared_permission};
#[cfg(feature = "trace")]
pub use reconstruct::reconstruct_graph;
pub use task_hash::{TaskClass, classify_task, task_hash, task_hash_for_class};
#[cfg(feature = "trace")]
pub use trace::{TopologyPaths, TraceKind, TraceReadout, TraceRecord, TraceWriter, read_trace};
