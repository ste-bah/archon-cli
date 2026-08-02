//! Diamond conformance: does a fan-out reach its reducer through independent
//! verification, and are the verifiers actually different from one another?
//!
//! Advisory only. Nothing here blocks a run — the milestone 3 invariants are
//! the enforcement surface and this is not one of them.
//!
//! # Why "nearest" reducer rather than "any reachable" reducer
//!
//! On a real graph almost every reducer is transitively reachable from almost
//! every fan-out, so "is there a reducer downstream with no verifier between"
//! reports the same fan-out once per downstream stage and says nothing. The
//! question that means something is about the *reduce that closes this
//! fan-out*: the first reducer encountered walking forward, before any other
//! reduction has already folded the branches away. That is the reduce frontier
//! computed by [`reduce_frontier`].

use std::collections::{BTreeSet, HashSet, VecDeque};

use crate::error::TopologyError;
use crate::index::GraphIndex;
use crate::ir::{NodeRole, TaskGraph};

/// One advisory finding about a fan-out/verify/reduce diamond.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiamondFinding {
    /// A fan-out is folded by a reducer with no verification stage on any path
    /// between them. The branches are merged on their own say-so.
    UnverifiedFanout {
        /// Node carrying the [`crate::FanoutSpec`].
        fanout: String,
        /// The reducer that folds it.
        reducer: String,
    },
    /// A reducer whose entire verification is one stage. One reviewer is not a
    /// panel: there is no second opinion to disagree with the first.
    SoleVerifier { reducer: String, verifier: String },
    /// A reducer with several verifiers that all name the same agent. Repeating
    /// one reviewer is not adversarial review, and the correlated failure —
    /// the thing the agent cannot see — survives every one of them.
    HomogeneousVerifiers {
        reducer: String,
        /// Sorted, for determinism.
        verifiers: Vec<String>,
        /// The single agent all of them name.
        agent: String,
    },
}

impl DiamondFinding {
    /// The node the finding is about, for grouping output by node.
    #[must_use]
    pub fn subject(&self) -> &str {
        match self {
            DiamondFinding::UnverifiedFanout { fanout, .. } => fanout,
            DiamondFinding::SoleVerifier { reducer, .. }
            | DiamondFinding::HomogeneousVerifiers { reducer, .. } => reducer,
        }
    }

    /// What to change, phrased as an instruction rather than a complaint.
    #[must_use]
    pub fn remedy(&self) -> String {
        match self {
            DiamondFinding::UnverifiedFanout { fanout, reducer } => format!(
                "insert a Verify stage between '{fanout}' and '{reducer}', or make '{reducer}' \
                 depend on one that already exists"
            ),
            DiamondFinding::SoleVerifier { reducer, verifier } => format!(
                "add a second, independently-agented Verify stage feeding '{reducer}'; \
                 '{verifier}' is currently its only verification"
            ),
            DiamondFinding::HomogeneousVerifiers {
                reducer,
                verifiers,
                agent,
            } => format!(
                "give at least one of {} a different agent than '{agent}' before it reaches \
                 '{reducer}'",
                verifiers
                    .iter()
                    .map(|id| format!("'{id}'"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

/// The diversity score for one reducer's verification.
///
/// Reported whether or not it produced a finding: the number is the useful
/// output, and a caller comparing two candidate shapes wants it for both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifierDiversity {
    pub reducer: String,
    /// Verify-role nodes that reach this reducer without passing through
    /// another reducer first. Sorted.
    pub verifiers: Vec<String>,
    /// Distinct non-empty `agent` values among them. Zero when no verifier
    /// names an agent, which is *unknown* rather than "all the same".
    pub distinct_agents: usize,
}

/// Everything [`TaskGraph::diamond_conformance`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiamondReport {
    pub findings: Vec<DiamondFinding>,
    /// One entry per reducer that has at least one verifier, sorted by reducer.
    pub diversity: Vec<VerifierDiversity>,
}

impl DiamondReport {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }
}

impl TaskGraph {
    /// Check that every fan-out is folded through independent verification, and
    /// score how independent that verification actually is.
    ///
    /// Three checks, all advisory:
    ///
    /// 1. **Unverified fold.** For each fan-out, the reducers on its reduce
    ///    frontier that no Verify node reaches. Reported per (fan-out, reducer)
    ///    pair so the message can name both ends.
    /// 2. **Sole verifier.** A reducer whose frontier verification is a single
    ///    stage. This needs no agent information: one stage is one reviewer
    ///    whatever it is called.
    /// 3. **Homogeneous verifiers.** Two or more verifiers that all name the
    ///    same agent. Requires every one of them to declare an agent — an
    ///    absent agent is *unknown*, and concluding "all the same" from a graph
    ///    that never said would be exactly the false positive the crate's
    ///    unknown rule exists to prevent.
    ///
    /// The IR carries no prompt, so "same agent and prompt" is scored on agent
    /// alone. That under-reports (two stages with the same agent and genuinely
    /// different prompts are not flagged) and never over-reports, which is the
    /// direction this crate errs in everywhere else.
    ///
    /// A graph with no fan-out and no reducer yields an empty report rather
    /// than an error: it did not opt into the shape.
    pub fn diamond_conformance(&self) -> Result<DiamondReport, TopologyError> {
        let index = GraphIndex::build(self)?;
        let mut report = DiamondReport::default();

        // Frontier reducers per fan-out, and the verifiers that reach each.
        let mut scored: BTreeSet<usize> = BTreeSet::new();
        for (position, node) in self.nodes.iter().enumerate() {
            if node.fanout.is_none() {
                continue;
            }
            for reducer in reduce_frontier(self, &index, position) {
                let verifiers = frontier_verifiers(self, &index, reducer);
                if verifiers.is_empty() {
                    report.findings.push(DiamondFinding::UnverifiedFanout {
                        fanout: node.id.clone(),
                        reducer: self.nodes[reducer].id.clone(),
                    });
                }
                scored.insert(reducer);
            }
        }

        // Diversity is scored for every reducer that has verification at all,
        // not only ones downstream of a fan-out: a reduce over hand-listed
        // branches is the same shape without the `FanoutSpec`.
        for (position, node) in self.nodes.iter().enumerate() {
            if node.role == NodeRole::Reduce {
                scored.insert(position);
            }
        }

        for reducer in scored {
            let verifiers = frontier_verifiers(self, &index, reducer);
            if verifiers.is_empty() {
                continue;
            }
            let ids: Vec<String> = verifiers
                .iter()
                .map(|&position| self.nodes[position].id.clone())
                .collect();
            let agents: Vec<Option<&str>> = verifiers
                .iter()
                .map(|&position| self.nodes[position].agent.as_deref())
                .collect();
            let distinct: BTreeSet<&str> = agents.iter().filter_map(|agent| *agent).collect();

            report.diversity.push(VerifierDiversity {
                reducer: self.nodes[reducer].id.clone(),
                verifiers: ids.clone(),
                distinct_agents: distinct.len(),
            });

            match ids.as_slice() {
                [only] => report.findings.push(DiamondFinding::SoleVerifier {
                    reducer: self.nodes[reducer].id.clone(),
                    verifier: only.clone(),
                }),
                _ => {
                    // Every verifier must have declared an agent. One absent
                    // agent means the graph never said whether they differ.
                    if agents.iter().all(Option::is_some) && distinct.len() == 1 {
                        let agent = (*distinct.iter().next().unwrap_or(&"")).to_string();
                        report.findings.push(DiamondFinding::HomogeneousVerifiers {
                            reducer: self.nodes[reducer].id.clone(),
                            verifiers: ids,
                            agent,
                        });
                    }
                }
            }
        }

        Ok(report)
    }
}

/// Reducers reachable from `start` by a path that crosses no other reducer.
///
/// Breadth-first over successors, not expanding past a reducer: the first
/// reduction encountered on a branch is the one that folds it, and anything
/// beyond that is folding something else.
fn reduce_frontier(graph: &TaskGraph, index: &GraphIndex, start: usize) -> Vec<usize> {
    let mut frontier = Vec::new();
    let mut seen: HashSet<usize> = HashSet::from([start]);
    let mut queue: VecDeque<usize> = VecDeque::from([start]);

    while let Some(position) = queue.pop_front() {
        for successor in successors(index, position) {
            if !seen.insert(successor) {
                continue;
            }
            if graph.nodes[successor].role == NodeRole::Reduce {
                frontier.push(successor);
                continue;
            }
            queue.push_back(successor);
        }
    }
    frontier.sort_unstable();
    frontier
}

/// Verify-role nodes that reach `reducer` without an intervening reducer.
///
/// The mirror of [`reduce_frontier`], walked backwards. Sorted by position so
/// the report is deterministic.
///
/// The walk passes *through* a verifier rather than stopping at it. Two review
/// stages in series — the shipped scaffold runs `verification-wave` and then
/// `adversarial-review` before its reduce — are two independent checks of the
/// same work, and counting only the last one would score that shape as having a
/// sole verifier, which is exactly the shape it was changed away from. The walk
/// stops only at another reducer, because everything behind that has already
/// been folded and belongs to *that* reduce.
fn frontier_verifiers(graph: &TaskGraph, index: &GraphIndex, reducer: usize) -> Vec<usize> {
    let mut verifiers = Vec::new();
    let mut seen: HashSet<usize> = HashSet::from([reducer]);
    let mut queue: VecDeque<usize> = VecDeque::from([reducer]);

    while let Some(position) = queue.pop_front() {
        for predecessor in predecessors(index, position) {
            if !seen.insert(predecessor) {
                continue;
            }
            if graph.nodes[predecessor].role == NodeRole::Reduce {
                continue;
            }
            if graph.nodes[predecessor].role == NodeRole::Verify {
                verifiers.push(predecessor);
            }
            queue.push_back(predecessor);
        }
    }
    verifiers.sort_unstable();
    verifiers
}

fn successors(index: &GraphIndex, position: usize) -> Vec<usize> {
    index
        .graph
        .neighbors_directed(index.node_index[position], petgraph::Direction::Outgoing)
        .map(|node| index.graph[node])
        .collect()
}

fn predecessors(index: &GraphIndex, position: usize) -> Vec<usize> {
    index
        .graph
        .neighbors_directed(index.node_index[position], petgraph::Direction::Incoming)
        .map(|node| index.graph[node])
        .collect()
}
