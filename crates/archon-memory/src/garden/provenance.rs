//! Provenance compatibility: which memories may be treated as one claim.
//!
//! Consolidation's generative half looks for several memories that corroborate
//! each other and proposes promoting them to one semantic memory. The danger in
//! that is not the merging — it is the *grouping*. Cluster across incompatible
//! provenance and the result asserts something no source actually says: a
//! preference the user stated for one project becomes a claim about all of them,
//! an auto-extracted guess is laundered into corroboration for a stated fact, or
//! a year-old observation is presented as evidence for this week's state.
//!
//! So compatibility is an explicit predicate with its own tests, deliberately
//! not an incidental property of a grouping key. A key answers "did these hash
//! the same"; a predicate can be asked "why were these two allowed together" and
//! can be wrong in a way a test can catch.
//!
//! # The predicate is not transitive, and that is the point
//!
//! Four of the five conditions are equivalence-like and would collapse into a
//! group-by. The fifth — that two memories were recorded within a bounded span
//! of each other — is not. A chain of memories each a fortnight apart would pass
//! a group-by on any window and produce a "cluster" spanning a year.
//!
//! [`compatible_clusters`] therefore admits a group only if EVERY PAIR in it is
//! compatible, not merely adjacent ones. A cluster is a clique, so its total
//! span is bounded by the pairwise span rather than by luck of ordering.
//!
//! # Nothing here writes
//!
//! Every function takes already-read `&[Memory]` and returns indices. There is
//! no `&dyn MemoryTrait` in this module, so proposal generation provably cannot
//! mutate the store: the handle does not exist in scope.

use chrono::Duration;

use crate::types::{Memory, MemoryType, is_withheld};

/// Tag marking a memory this crate produced by consolidating others.
///
/// Consolidation output is excluded from further consolidation. Without that, a
/// derived memory joins the next pass's cluster as if it were an independent
/// observation, and its sources are counted twice — corroboration manufactured
/// out of the act of recording corroboration.
pub const DERIVED_TAG: &str = "garden:derived";

/// Tag prefix for the run that produced a derived memory.
pub const DERIVED_FROM_RUN_TAG_PREFIX: &str = "garden:derived-run:";

/// Source types the garden writes for its own bookkeeping.
///
/// Never clusterable. `garden:last_run` is a timestamp stored as a `Fact`, and a
/// cluster of timestamps is not a semantic memory.
const GARDEN_BOOKKEEPING_SOURCE: &str = "garden";

/// The identity two memories must share before they can corroborate each other.
///
/// Borrowed rather than owned so building one costs nothing per comparison in
/// the pairwise scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryProvenance<'a> {
    /// What kind of claim this is.
    pub memory_type: MemoryType,
    /// Which project scope it was recorded in.
    pub project_path: &'a str,
    /// Which writer produced it.
    pub source_type: &'a str,
}

impl<'a> MemoryProvenance<'a> {
    pub fn of(memory: &'a Memory) -> Self {
        Self {
            memory_type: memory.memory_type,
            project_path: memory.project_path.as_str(),
            source_type: memory.source_type.as_str(),
        }
    }
}

/// Why a single memory may not take part in consolidation at all.
///
/// Returned rather than folded into a bool so a test — and a reader of a
/// rejected cluster — can see which rule excluded a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ineligible {
    /// Already hidden from ordinary reads: superseded, or retired by an
    /// approved proposal. Consolidating it would resurrect its content.
    Withheld,
    /// Produced by a previous consolidation. Re-clustering it double-counts the
    /// sources it already stands for.
    AlreadyDerived,
    /// The garden's own bookkeeping.
    Bookkeeping,
    /// A prompt rule. Rules are not observations, and a "semantic memory"
    /// synthesised from rule text would be a rule mutation wearing a different
    /// hat.
    PromptRule,
}

/// Whether one memory may take part in consolidation at all.
///
/// Unary conditions, checked before any pair is considered, so an ineligible row
/// cannot enter a cluster through a partner that happens to match it.
pub fn ineligible_reason(memory: &Memory) -> Option<Ineligible> {
    if is_withheld(&memory.tags) {
        return Some(Ineligible::Withheld);
    }
    if memory.tags.iter().any(|tag| tag == DERIVED_TAG) {
        return Some(Ineligible::AlreadyDerived);
    }
    if memory.source_type == GARDEN_BOOKKEEPING_SOURCE {
        return Some(Ineligible::Bookkeeping);
    }
    if memory.memory_type == MemoryType::Rule {
        return Some(Ineligible::PromptRule);
    }
    None
}

/// Whether two memories may be treated as evidence for one claim.
///
/// Both must be individually eligible, share a provenance identity, and have
/// been recorded within `max_span` of each other.
///
/// The span condition is what stops a cluster drifting: two memories about "the
/// current deployment target" recorded a year apart are not corroboration, they
/// are a record of a change. Merging them asserts the older one is still true.
pub fn provenance_compatible(left: &Memory, right: &Memory, max_span: Duration) -> bool {
    if left.id == right.id {
        return false;
    }
    if ineligible_reason(left).is_some() || ineligible_reason(right).is_some() {
        return false;
    }
    if MemoryProvenance::of(left) != MemoryProvenance::of(right) {
        return false;
    }
    let span = if left.created_at >= right.created_at {
        left.created_at - right.created_at
    } else {
        right.created_at - left.created_at
    };
    span <= max_span
}

/// Group `memories` into clusters in which every pair is compatible.
///
/// Returns index sets into `memories`, largest cluster first, then by lowest
/// starting index so the result is deterministic for a fixed input.
///
/// # Why greedy cliques rather than exact ones
///
/// Maximum-clique is intractable in general and this runs unattended over a
/// store that may hold thousands of rows. The greedy pass takes each memory in
/// turn as a seed and admits later candidates only if they are compatible with
/// EVERY member already admitted, which is what bounds the cluster's span.
///
/// Greedy can miss a larger clique. That direction of error is the safe one: it
/// proposes fewer consolidations than a perfect algorithm would, and every
/// cluster it does propose satisfies the predicate in full. The reverse — an
/// algorithm that finds bigger groups by relaxing pairwise checking — is exactly
/// the failure this module exists to prevent.
pub fn compatible_clusters(
    memories: &[Memory],
    min_cluster_size: usize,
    max_span: Duration,
) -> Vec<Vec<usize>> {
    if min_cluster_size < 2 {
        // A "cluster" of one is a memory, and consolidating it would create a
        // duplicate asserting the same thing with a derived label.
        return Vec::new();
    }
    let eligible: Vec<usize> = (0..memories.len())
        .filter(|index| ineligible_reason(&memories[*index]).is_none())
        .collect();

    let mut claimed = vec![false; memories.len()];
    let mut clusters: Vec<Vec<usize>> = Vec::new();

    for &seed in &eligible {
        if claimed[seed] {
            continue;
        }
        let mut cluster = vec![seed];
        for &candidate in &eligible {
            if candidate <= seed || claimed[candidate] {
                continue;
            }
            // Every member, not just the seed. This is the clique condition,
            // and deleting it would silently turn the span bound into a
            // chain-length bound.
            let compatible_with_all = cluster.iter().all(|&member| {
                provenance_compatible(&memories[member], &memories[candidate], max_span)
            });
            if compatible_with_all {
                cluster.push(candidate);
            }
        }
        if cluster.len() >= min_cluster_size {
            for &member in &cluster {
                claimed[member] = true;
            }
            clusters.push(cluster);
        }
    }

    clusters.sort_by(|left, right| {
        right
            .len()
            .cmp(&left.len())
            .then_with(|| left.first().cmp(&right.first()))
    });
    clusters
}

#[cfg(test)]
#[path = "provenance_tests.rs"]
mod provenance_tests;
