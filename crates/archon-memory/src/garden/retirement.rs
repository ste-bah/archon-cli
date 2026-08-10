//! Retirement: what an unattended pass does INSTEAD of deleting.
//!
//! Two of consolidation's phases destroy rows outright — staleness pruning and
//! overflow pruning both call `delete_memory`. Everything else it does is
//! reversible: importance decay is a signed delta with an immutable provenance
//! id, and every merge marks the loser `superseded` rather than removing it, so
//! a wrong merge is undone by deleting a tag.
//!
//! Those two are different in kind. A deleted memory cannot be reviewed after
//! the fact, cannot be restored, and leaves nothing behind to say it existed.
//! Running them from a background job at 3am means the user finds out only by
//! noticing something they used to remember is gone — and cannot tell whether
//! it was pruned, never stored, or lost to a bug.
//!
//! So a scheduled pass does not delete. It emits a [`RetirementCandidate`] per
//! row it *would* have pruned, and returns them in the report. Somebody with a
//! governed store persists them for review; the memory itself is untouched
//! either way.
//!
//! # Returned, never written to the graph
//!
//! This deliberately follows [`super::ReviewPair`], for the reason that pattern
//! exists. An earlier version of the review band recorded its undecided pairs as
//! `RelatedTo` edges; the fragment-merge phase then read those edges as merge
//! instructions and hard-deleted one memory from each pair. Thirteen were
//! destroyed. The lesson was not "merge more carefully" — it was that a phase
//! withholding a decision must not write anything a later phase can read as a
//! decision.
//!
//! A retirement candidate is exactly such a withheld decision, so it is a value
//! handed back to the caller. Nothing about it exists in the memory graph: no
//! row, no tag, no edge. A phase that runs afterwards cannot see it, and
//! therefore cannot act on it.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::{Memory, MemoryType};

/// Longest excerpt carried on a candidate.
///
/// Enough to recognise which memory is being proposed for removal without
/// copying its whole content into a second store. A reviewer who needs the full
/// text can read the row, which by construction still exists.
const EXCERPT_MAX_CHARS: usize = 300;

/// What a pass does when a phase decides a row should go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrunePolicy {
    /// Delete it. Only for a pass a person started and can see the result of.
    ///
    /// This is what `/garden` and session-start consolidation have always done,
    /// and they keep doing it: the person typed the command, the report says how
    /// many rows went, and they are there to read it.
    Delete,
    /// Never delete. Emit a [`RetirementCandidate`] for review instead.
    ///
    /// The only policy an unattended pass may use.
    Propose,
}

impl PrunePolicy {
    /// Whether a phase under this policy is permitted to destroy a row.
    pub fn may_delete(self) -> bool {
        matches!(self, Self::Delete)
    }
}

/// Why a memory was proposed for retirement.
///
/// Carried as data rather than a formatted sentence so a reviewer can filter and
/// count by cause, and so the numbers that justified the proposal survive into
/// the record instead of being reconstructed from prose.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RetirementReason {
    /// Untouched past the staleness window while below the importance floor.
    Stale {
        days_since_access: i64,
        staleness_days: u32,
        importance_floor: f64,
    },
    /// The store is over its cap and this row is among the least important.
    ///
    /// Weaker evidence than staleness by itself: nothing is wrong with the
    /// memory, it simply sorted last. Recorded distinctly so a reviewer can hold
    /// it to a different standard.
    Overflow {
        max_memories: usize,
        total_memories: usize,
    },
}

impl RetirementReason {
    /// A stable short name, for grouping and for storage keys.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Stale { .. } => "stale",
            Self::Overflow { .. } => "overflow",
        }
    }
}

/// A memory a consolidation pass declined to delete, offered for review.
///
/// Everything a reviewer needs to decide without loading the store, and a
/// `memory_id` to act on if they choose to. The memory it names is still live:
/// nothing here has happened to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetirementCandidate {
    pub memory_id: String,
    pub title: String,
    /// The first few hundred characters of the content, for recognition.
    pub excerpt: String,
    pub memory_type: MemoryType,
    pub importance: f64,
    pub created_at: DateTime<Utc>,
    pub last_accessed: Option<DateTime<Utc>>,
    pub access_count: u64,
    pub reason: RetirementReason,
}

impl RetirementCandidate {
    /// Build a candidate from the row a phase declined to prune.
    pub fn from_memory(memory: &Memory, reason: RetirementReason) -> Self {
        Self {
            memory_id: memory.id.clone(),
            title: memory.title.clone(),
            excerpt: excerpt(&memory.content),
            memory_type: memory.memory_type,
            importance: memory.importance,
            created_at: memory.created_at,
            last_accessed: memory.last_accessed,
            access_count: memory.access_count,
            reason,
        }
    }
}

/// Flatten and clip content for display, on a character boundary.
fn excerpt(content: &str) -> String {
    let flat: String = content.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(EXCERPT_MAX_CHARS).collect()
}

#[cfg(test)]
#[path = "retirement_tests.rs"]
mod retirement_tests;
