//! Declared edges that carry no declared dataflow.
//!
//! A dependency edge is a claim: "I cannot start until that has finished."
//! Where both ends declare their dataflow and the two sets do not meet, the
//! claim has no support in anything either node said about itself. That is a
//! *fake edge* — it costs wall-clock by serialising work that could run
//! concurrently, and it hides which orderings are real.
//!
//! # This never removes an edge
//!
//! Reported, never applied. An edge can be real for a reason neither node
//! wrote down — a shared runtime resource, an API rate limit, a human sequencing
//! preference. The analysis can see declarations; it cannot see reasons. So the
//! output names the pair and says what would have to be declared for the edge
//! to be justified, and the decision stays with whoever authored it.
//!
//! # Silence is the default
//!
//! Nothing is reported unless *both* ends have declared something: the
//! dependency must declare production (`writes`) and the dependent must declare
//! consumption (`reads` or `consumes`). Empty means unknown throughout this
//! crate, and a lowering with no dataflow to give — the `Vec<Subtask>` one, for
//! instance — therefore produces no findings at all rather than declaring every
//! edge in the graph fake.

use std::collections::BTreeSet;

use crate::error::TopologyError;
use crate::index::GraphIndex;
use crate::ir::{TaskGraph, WriteTarget};

/// A declared dependency with no declared dataflow behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeEdge {
    /// The node that declared `depends_on`.
    pub dependent: String,
    /// The node it declared a dependency on.
    pub dependency: String,
    /// What the dependency declared it produces, sorted. Included because the
    /// useful next question is "then which of these did you mean to consume?".
    pub produced: Vec<WriteTarget>,
    /// What the dependent declared it consumes, sorted.
    pub consumed: Vec<WriteTarget>,
}

impl FakeEdge {
    /// What to change.
    #[must_use]
    pub fn remedy(&self) -> String {
        format!(
            "either drop '{dependency}' from {dependent}'s depends_on so the two can run \
             concurrently, or declare which of {dependency}'s outputs {dependent} consumes",
            dependency = self.dependency,
            dependent = self.dependent
        )
    }
}

impl TaskGraph {
    /// Dependency edges where the dependent consumes nothing the dependency
    /// produces.
    ///
    /// An edge `dependent → dependency` is reported when all of:
    ///
    /// - the dependency declares production ([`crate::TaskNode::writes`] is
    ///   non-empty),
    /// - the dependent declares consumption ([`crate::TaskNode::reads`] or
    ///   [`crate::TaskNode::consumes`] is non-empty),
    /// - no entry of the dependent's `reads` appears in the dependency's
    ///   `writes`, and
    /// - no entry of the dependent's `consumes` names the dependency as its
    ///   producer.
    ///
    /// Overlap is exact-string, matching
    /// [`TaskGraph::write_conflicts`](crate::WriteConflict): a glob and a path
    /// that a proper matcher would intersect are treated as disjoint here. That
    /// makes this analysis *over*-report relative to a matcher-backed one — a
    /// real dataflow expressed as `src/**` against `src/a.rs` would be called
    /// fake. Callers that need the stricter answer should declare literal
    /// targets on both sides. The trade is deliberate and is the same
    /// dependency-budget constraint recorded on `write_conflicts`.
    ///
    /// Errors only on a structurally invalid graph (duplicate id, unknown
    /// dependency, cycle), which is [`GraphIndex::build`]'s contract.
    pub fn fake_edges(&self) -> Result<Vec<FakeEdge>, TopologyError> {
        // Built for its validation, not its topology: an unknown dependency id
        // must be reported as such rather than silently skipped below.
        let _index = GraphIndex::build(self)?;

        let mut fake = Vec::new();
        for dependent in &self.nodes {
            if !dependent.consumption_is_known() {
                continue;
            }
            let reads: BTreeSet<&WriteTarget> = dependent.reads.iter().collect();
            let producers: BTreeSet<&str> = dependent
                .consumes
                .iter()
                .map(|reference| reference.producer.as_str())
                .collect();

            for dependency_id in &dependent.depends_on {
                if producers.contains(dependency_id.as_str()) {
                    continue;
                }
                let Some(dependency) = self.node(dependency_id) else {
                    continue;
                };
                if !dependency.writes_are_known() {
                    continue;
                }
                if dependency
                    .writes
                    .iter()
                    .any(|target| reads.contains(target))
                {
                    continue;
                }

                fake.push(FakeEdge {
                    dependent: dependent.id.clone(),
                    dependency: dependency.id.clone(),
                    produced: sorted(&dependency.writes),
                    consumed: sorted(&dependent.reads),
                });
            }
        }
        Ok(fake)
    }
}

fn sorted(targets: &[WriteTarget]) -> Vec<WriteTarget> {
    targets
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
