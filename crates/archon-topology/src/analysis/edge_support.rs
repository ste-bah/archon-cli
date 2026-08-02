//! What supports a declared dependency edge — three answers, not two.
//!
//! A dependency edge is a claim: "I cannot start until that has finished."
//! The earlier version of this analysis asked one question — does the dependent
//! consume anything the dependency produces? — and treated every "no" as a
//! defect. On a real seventeen-task corpus that was too blunt in both
//! directions, so the question is now asked in three parts:
//!
//! - [`EdgeSupport::Dataflow`] — the dependent consumes something the
//!   dependency produces. Silent.
//! - [`EdgeSupport::OrderingOnly`] — the dependency is contracted to produce no
//!   artifact at all and declares only repository files, while the dependent
//!   consumes artifacts. The two ends speak disjoint vocabularies, so the
//!   absence of overlap is not evidence of anything: code has to exist before a
//!   command that calls it can run, and no data flows across that edge because
//!   none should. Informational; never a defect.
//! - [`EdgeSupport::Unsupported`] — the dependency **does** produce artifacts
//!   and the dependent consumes none of them. This is the finding.
//!
//! # This never removes an edge
//!
//! Reported, never applied. An edge can be real for a reason neither node wrote
//! down — a shared runtime resource, an API rate limit, a human sequencing
//! preference. The analysis can see declarations; it cannot see reasons.
//!
//! # An `Unsupported` finding names both causes, and does not blame the
//! dependent by default
//!
//! The first version of the remedy text said "drop the edge, or declare what
//! you consume", which reads as though the *dependent* is at fault. On the real
//! corpus the cause ran the other way: several ingest tasks wrote a shared
//! registry without declaring that they did, so six edges looked unsupported
//! when the producers were under-declared. Dropping any of those edges would
//! have been the wrong repair. So every `Unsupported` finding names both
//! candidate causes, and where the graph carries evidence for one it says which
//! — see [`LikelyCause`].
//!
//! # Two signals considered and not used
//!
//! **Implementation phases.** A decomposed PRD's phase table is a structural
//! gate, and an edge crossing a phase boundary is ordered by that gate whatever
//! its dataflow. It is not used here because the phase→task mapping is not a
//! PRD declaration: the corpus's own guidance file records it as *"NOT declared
//! by the PRD. §14 names no task ids… a hand mapping"*. Classifying on it would
//! claim an authority the source disclaims, and would put a corpus-specific
//! file inside a crate that must stay general.
//!
//! **`workstream:`.** A per-task declaration, general, and a real gate — but it
//! does not reach this IR, and it would not have decided the case that motivated
//! the change: the CLI-surface task and the command that depends on it are in
//! the *same* workstream. A signal that cannot separate the one edge it was
//! proposed for is not the rule to build on.
//!
//! What is used instead is the declaration each end already makes about what it
//! produces and consumes — see [`TaskGraph::classify_edges`] — which is local,
//! authored, and available on every surface that lowers into this IR.
//!
//! # Silence is still the default
//!
//! Nothing is classified unless *both* ends have declared something: the
//! dependency must declare production ([`crate::TaskNode::writes`]) and the
//! dependent must declare consumption (`reads` or `consumes`). Empty means
//! unknown throughout this crate, so a lowering with no dataflow to give — the
//! `Vec<Subtask>` one, for instance — yields no classified edges at all rather
//! than an opinion about every edge in the graph.

use std::collections::BTreeSet;

use crate::error::TopologyError;
use crate::index::GraphIndex;
use crate::ir::{TaskGraph, TaskNode, WriteTarget};

/// Which end of an [`EdgeSupport::Unsupported`] edge the graph points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LikelyCause {
    /// The dependency is contracted to produce artifacts that **nothing** in
    /// the graph declares it consumes, while `dependents` tasks declare they
    /// must wait for it. An ordering that real, carried by an output nobody
    /// named, is what an undeclared write looks like — and under-declaration on
    /// the producing side explains every one of those edges at once, where
    /// dropping them explains none.
    UnderDeclaredProducer {
        /// How many tasks declare a dependency on the producer.
        dependents: usize,
        /// How many artifacts it is contracted to produce.
        artifacts: usize,
    },
    /// The graph does not distinguish the two causes. Both are named; neither
    /// is ranked.
    Undetermined,
}

/// What supports one declared `depends_on` edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeSupport {
    /// The dependent consumes something the dependency produces.
    Dataflow,
    /// Code ordering: the dependency produces no artifact, only source, and the
    /// dependent consumes artifacts. Nothing flows and nothing should.
    OrderingOnly,
    /// The dependency produces artifacts the dependent consumes none of.
    Unsupported(LikelyCause),
}

impl EdgeSupport {
    /// Whether this classification is a finding the author has to act on.
    ///
    /// `OrderingOnly` is deliberately not a defect: reporting it as one is what
    /// made the earlier version of this lint untrustworthy on a real corpus.
    #[must_use]
    pub fn is_defect(self) -> bool {
        matches!(self, EdgeSupport::Unsupported(_))
    }

    /// Stable identifier, for logs and metrics.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            EdgeSupport::Dataflow => "dataflow",
            EdgeSupport::OrderingOnly => "ordering_only",
            EdgeSupport::Unsupported(_) => "unsupported",
        }
    }
}

/// One declared dependency edge, classified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedEdge {
    /// The node that declared `depends_on`.
    pub dependent: String,
    /// The node it declared a dependency on.
    pub dependency: String,
    /// What the classification concluded.
    pub support: EdgeSupport,
    /// What the dependency declared it produces, sorted.
    pub produced: Vec<WriteTarget>,
    /// What the dependent declared it consumes, sorted.
    pub consumed: Vec<WriteTarget>,
}

impl ClassifiedEdge {
    /// Whether this edge is a finding rather than an observation.
    #[must_use]
    pub fn is_defect(&self) -> bool {
        self.support.is_defect()
    }

    /// One line naming what was concluded.
    #[must_use]
    pub fn headline(&self) -> String {
        match self.support {
            EdgeSupport::Dataflow => "declared dataflow".to_string(),
            EdgeSupport::OrderingOnly => "code ordering, no dataflow".to_string(),
            EdgeSupport::Unsupported(_) => {
                "produces artifacts the dependent does not consume".to_string()
            }
        }
    }

    /// What to change — or, for an ordering-only edge, that there is nothing to
    /// change.
    #[must_use]
    pub fn remedy(&self) -> String {
        let (dependent, dependency) = (&self.dependent, &self.dependency);
        match self.support {
            EdgeSupport::Dataflow => {
                format!("{dependent} consumes what {dependency} produces; nothing to change")
            }
            EdgeSupport::OrderingOnly => format!(
                "{dependency} is contracted to produce no artifact and declares only source \
                 files, while {dependent} consumes artifacts: this edge orders code, not data. \
                 Nothing flows across it and nothing should — leave it alone."
            ),
            EdgeSupport::Unsupported(cause) => self.unsupported_remedy(cause),
        }
    }

    fn unsupported_remedy(&self, cause: LikelyCause) -> String {
        let (dependent, dependency) = (&self.dependent, &self.dependency);
        let both = format!(
            "{dependent} declares it must wait for {dependency}, which is contracted to produce \
             artifacts {dependent} names none of. Exactly two things cause that: the edge is \
             unnecessary, or {dependency} under-declares — it writes something {dependent} reads \
             and no contract says so."
        );
        match cause {
            LikelyCause::UnderDeclaredProducer {
                dependents,
                artifacts,
            } => format!(
                "{both} The graph favours the second: nothing anywhere consumes any of the \
                 {artifacts} artifact(s) {dependency} is contracted to produce, yet {dependents} \
                 task(s) declare they must wait for it. Declare what {dependency} actually writes \
                 before considering dropping the edge; dropping it while the write is real would \
                 let the two race."
            ),
            LikelyCause::Undetermined => format!(
                "{both} Settle it from what {dependency} writes: if it writes something \
                 {dependent} reads, add that to {dependency}'s declared outputs; if it does not, \
                 drop '{dependency}' from {dependent}'s depends_on so the two can run \
                 concurrently."
            ),
        }
    }
}

impl TaskGraph {
    /// Every declared dependency edge on which the declarations support a
    /// conclusion, with that conclusion.
    ///
    /// An edge appears only when the dependency declares production
    /// ([`TaskNode::writes_are_known`]) **and** the dependent declares
    /// consumption ([`TaskNode::consumption_is_known`]). Anything else is
    /// unknown, not clean, and is omitted rather than guessed at.
    ///
    /// Overlap is exact-string on the whole [`WriteTarget`], matching
    /// [`TaskGraph::write_conflicts`](crate::WriteConflict): a glob and a path
    /// that a proper matcher would intersect are treated as disjoint here. That
    /// makes this analysis *over*-report relative to a matcher-backed one — a
    /// real dataflow expressed as `src/**` against `src/a.rs` would not be
    /// recognised. The trade is the same dependency-budget constraint recorded
    /// on `write_conflicts`.
    ///
    /// Errors only on a structurally invalid graph (duplicate id, unknown
    /// dependency, cycle), which is [`GraphIndex::build`]'s contract.
    pub fn classify_edges(&self) -> Result<Vec<ClassifiedEdge>, TopologyError> {
        // Built for its validation, not its topology: an unknown dependency id
        // must be reported as such rather than silently skipped below.
        let _index = GraphIndex::build(self)?;

        let mut classified = Vec::new();
        for dependent in &self.nodes {
            if !dependent.consumption_is_known() {
                continue;
            }
            for dependency_id in &dependent.depends_on {
                let Some(dependency) = self.node(dependency_id) else {
                    continue;
                };
                if !dependency.writes_are_known() {
                    continue;
                }
                classified.push(ClassifiedEdge {
                    dependent: dependent.id.clone(),
                    dependency: dependency.id.clone(),
                    support: self.support_for(dependent, dependency),
                    produced: sorted(&dependency.writes),
                    consumed: sorted(&dependent.reads),
                });
            }
        }
        Ok(classified)
    }

    /// The subset of [`TaskGraph::classify_edges`] that is a finding.
    pub fn unsupported_edges(&self) -> Result<Vec<ClassifiedEdge>, TopologyError> {
        Ok(self
            .classify_edges()?
            .into_iter()
            .filter(ClassifiedEdge::is_defect)
            .collect())
    }

    fn support_for(&self, dependent: &TaskNode, dependency: &TaskNode) -> EdgeSupport {
        if consumes_from(dependent, &dependency.id) || reads_any_of(dependent, &dependency.writes) {
            return EdgeSupport::Dataflow;
        }
        if is_ordering_only(dependent, dependency) {
            return EdgeSupport::OrderingOnly;
        }
        EdgeSupport::Unsupported(self.likely_cause(dependency))
    }

    /// Whether the graph carries evidence that the *producer* is the problem.
    ///
    /// The evidence is graph-wide rather than edge-local on purpose: one
    /// dependent naming none of a producer's outputs says nothing, but a
    /// producer whose contracted outputs are named by nobody at all — while
    /// tasks queue behind it — is a producer whose real output is undeclared.
    fn likely_cause(&self, dependency: &TaskNode) -> LikelyCause {
        let artifacts = dependency
            .writes
            .iter()
            .filter(|target| matches!(target, WriteTarget::Artifact(_)))
            .count();
        if artifacts == 0 {
            return LikelyCause::Undetermined;
        }
        let named_by_anyone = self.nodes.iter().any(|node| {
            node.id != dependency.id
                && (consumes_from(node, &dependency.id) || reads_any_of(node, &dependency.writes))
        });
        if named_by_anyone {
            return LikelyCause::Undetermined;
        }
        let dependents = self
            .nodes
            .iter()
            .filter(|node| node.depends_on.contains(&dependency.id))
            .count();
        LikelyCause::UnderDeclaredProducer {
            dependents,
            artifacts,
        }
    }
}

/// Whether `node`'s producer-keyed dataflow names `producer_id`.
fn consumes_from(node: &TaskNode, producer_id: &str) -> bool {
    node.consumes
        .iter()
        .any(|reference| reference.producer == producer_id)
}

/// Whether `node` reads any of `targets`, compared whole (kind and value).
fn reads_any_of(node: &TaskNode, targets: &[WriteTarget]) -> bool {
    let reads: BTreeSet<&WriteTarget> = node.reads.iter().collect();
    targets.iter().any(|target| reads.contains(target))
}

/// The ordering-only shape: the two ends cannot overlap by construction.
///
/// Three conditions, all from declarations that already exist:
///
/// 1. The dependency is contracted to produce **no** artifact — its whole
///    declared production is repository files. That is what an empty
///    `deliverable_contracts` plus a populated "files expected to change"
///    lowers to.
/// 2. The dependent's declared consumption is **entirely** artifacts.
/// 3. No file the dependency writes carries the same path string as an artifact
///    the dependent reads.
///
/// Condition 3 is the one that is not redundant. Conditions 1 and 2 establish
/// that the two ends name resources in different vocabularies, so no overlap
/// was ever possible and the absence of one is not evidence. But a task that
/// writes `.archon/…/registry.json` as a plain file while its dependent reads
/// the *artifact* of the same name is not speaking a different vocabulary — it
/// is under-declaring, which is exactly the finding this must not swallow. So a
/// same-path-different-kind match disqualifies the edge from being called
/// ordering-only and it stays reported.
fn is_ordering_only(dependent: &TaskNode, dependency: &TaskNode) -> bool {
    let produces_artifacts = dependency
        .writes
        .iter()
        .any(|target| matches!(target, WriteTarget::Artifact(_)));
    if produces_artifacts {
        return false;
    }
    if dependent.reads.is_empty()
        || !dependent
            .reads
            .iter()
            .all(|target| matches!(target, WriteTarget::Artifact(_)))
    {
        return false;
    }
    let read_values: BTreeSet<&str> = dependent.reads.iter().map(target_value).collect();
    !dependency
        .writes
        .iter()
        .any(|target| read_values.contains(target_value(target)))
}

fn target_value(target: &WriteTarget) -> &str {
    match target {
        WriteTarget::Path(value) | WriteTarget::Artifact(value) => value.as_str(),
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
