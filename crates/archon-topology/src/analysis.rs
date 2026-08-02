//! Pure analyses over [`crate::TaskGraph`]. No I/O, no database, no clock.
//!
//! Milestone 1 only exposes these; nothing consumes them yet beyond
//! [`TaskGraph::waves`], which replaced the orchestrator's private DAG.
//!
//! Each analysis lives in its own module and contributes its own `impl
//! TaskGraph` block. Ordinary `mod` declarations throughout — the command
//! layer's 97 `include!` sites and 139 `#[path = …]` declarations are the
//! pattern this crate exists partly to avoid inheriting.

pub mod critical_path;
pub mod gates;
pub mod parallelism;
pub mod waves;
pub mod write_conflicts;

pub use critical_path::CriticalPath;
pub use parallelism::ParallelismProfile;
pub use write_conflicts::WriteConflict;
