//! Pure analyses over [`crate::TaskGraph`]. No I/O, no database, no clock.
//!
//! Milestone 1 only exposes these; nothing consumes them yet beyond
//! [`TaskGraph::waves`], which replaced the orchestrator's private DAG.
//!
//! Each analysis lives in its own module and contributes its own `impl
//! TaskGraph` block. Ordinary `mod` declarations throughout — the command
//! layer's 97 `include!` sites and 139 `#[path = …]` declarations are the
//! pattern this crate exists partly to avoid inheriting.

//! Milestone 4 adds three more — [`diamond`], [`edge_support`], [`fusion`] —
//! and they are **advisory**. They report and explain; none of them can block a
//! run. Enforcement is milestone 3's `live` module and stays there.

pub mod critical_path;
pub mod diamond;
pub mod edge_support;
pub mod fusion;
pub mod gates;
pub mod parallelism;
pub mod waves;
pub mod write_conflicts;

pub use critical_path::CriticalPath;
pub use diamond::{DiamondFinding, DiamondReport, VerifierDiversity};
pub use edge_support::{ClassifiedEdge, EdgeSupport, LikelyCause};
pub use fusion::{CoupledPair, FusibleChain, FusionKind, FusionReport};
pub use parallelism::ParallelismProfile;
pub use write_conflicts::WriteConflict;
