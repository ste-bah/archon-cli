//! A non-blocking note from the patch gate: a changed file whose functions
//! the complexity cap could not read reliably.
//!
//! The cap never refuses a patch on such a reading — a scanner gap is the
//! harness's defect, not the agent's — so the note is the only trace that a
//! check was weakened or skipped. It travels back with a validated patch and
//! is recorded where the branch's outcome is persisted, so an operator can
//! see which files went unmeasured and why.

use serde::{Deserialize, Serialize};

/// The rule name the note is recorded under.
pub const COMPLEXITY_SCAN_UNRELIABLE: &str = "complexity_scan_unreliable";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnreliableScan {
    /// Always [`COMPLEXITY_SCAN_UNRELIABLE`]; carried so a persisted note
    /// names its rule.
    pub rule: String,
    /// The changed file, relative to the repository root.
    pub path: String,
    /// 1-based line the unreliable reading starts at.
    pub line: usize,
    /// The grammar (`rust`, `tsx`, ...) or, for the hand scanner, the file
    /// extension.
    pub language: String,
    /// What went wrong, and in which text (baseline or post-patch).
    pub reason: String,
}
