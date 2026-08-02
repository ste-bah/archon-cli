//! The executed prefix: what a session has actually done so far.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use archon_workflow::write_coordinator::write_plan::ResourceKey;

use crate::index::GraphIndex;
use crate::ir::{GraphBudget, TaskGraph};

/// Structure folded in from a declared graph.
///
/// Everything expensive is computed here, once, at declare time: dominators and
/// transitive reachability. Admission then answers from lookups. That is not an
/// optimisation, it is the constraint — admission is synchronous and on the
/// critical path of every non-`Safe` tool call.
#[derive(Debug, Clone, Default)]
pub struct SessionStructure {
    /// Resource ceilings the declared graph carries.
    pub budget: GraphBudget,
    /// Node id → gates that strictly dominate it, from
    /// [`TaskGraph::dominating_gates`].
    dominating_gates: BTreeMap<String, Vec<String>>,
    /// Every gate node the graph declares, passed or not.
    declared_gates: BTreeSet<String>,
    /// `related[a]` holds every node connected to `a` by a dependency path in
    /// either direction. Symmetric by construction — invariant 2 asks whether
    /// *any* ordering constrains two nodes, and either direction does.
    related: HashMap<String, HashSet<String>>,
}

impl SessionStructure {
    /// Fold a declared graph into admission-ready structure.
    ///
    /// `None` when the graph does not validate — a cycle, a duplicate id, an
    /// unknown dependency. Reporting malformed graphs is `TaskGraph::waves`'
    /// job, and adopting one here would mean admitting against nonsense
    /// reachability, so the session keeps the structure it already had.
    #[must_use]
    pub fn from_graph(graph: &TaskGraph) -> Option<Self> {
        let index = GraphIndex::build(graph).ok()?;
        let dominating_gates = graph.dominating_gates().ok()?;
        let reachable = index.descendants(graph);

        let mut related: HashMap<String, HashSet<String>> = HashMap::new();
        for (from, row) in reachable.iter().enumerate() {
            for (to, &is_reachable) in row.iter().enumerate() {
                if !is_reachable {
                    continue;
                }
                let (a, b) = (&graph.nodes[from].id, &graph.nodes[to].id);
                related.entry(a.clone()).or_default().insert(b.clone());
                related.entry(b.clone()).or_default().insert(a.clone());
            }
        }

        Some(Self {
            budget: graph.budget,
            declared_gates: graph.gate_nodes().into_iter().collect(),
            dominating_gates,
            related,
        })
    }

    /// Whether the graph declares any gate at all.
    #[must_use]
    pub fn declares_gates(&self) -> bool {
        !self.declared_gates.is_empty()
    }

    /// Gates that strictly dominate `node_id` in the declared graph.
    #[must_use]
    pub fn dominating_gates(&self, node_id: &str) -> &[String] {
        self.dominating_gates
            .get(node_id)
            .map_or(&[][..], Vec::as_slice)
    }

    /// Whether the declared graph knows `node_id` at all.
    #[must_use]
    pub fn knows(&self, node_id: &str) -> bool {
        self.dominating_gates.contains_key(node_id)
    }
}

/// One node's claim on one resource.
#[derive(Debug, Clone)]
pub(super) struct WriteClaim {
    pub(super) node_id: String,
    pub(super) key: ResourceKey,
    /// The path as declared, for the block reason. A resource key is folded and
    /// separator-unified and makes a poor thing to show a reader.
    pub(super) declared: String,
}

/// One session's executed prefix.
#[derive(Debug, Clone, Default)]
pub struct SessionState {
    structure: SessionStructure,
    /// Set when a declared graph has been folded in. Distinguishes "declared a
    /// graph with no gates" from "declared nothing", which invariant 3 needs.
    declared: bool,
    started: BTreeSet<String>,
    finished: BTreeSet<String>,
    gates_passed: BTreeSet<String>,
    /// Lifetime total. Never decremented — that is the distinction from
    /// `AgentPool`, which caps concurrency and releases on completion, leaving a
    /// team with no lifetime total at all (finding O2).
    agents_spawned: u32,
    live_agents: u32,
    claims: Vec<WriteClaim>,
    rounds: BTreeMap<String, u32>,
    /// Dependency edges observed at runtime — a spawn's parent → child — for
    /// sessions with no declared graph. Kept separate from `structure` so a
    /// later `declare_graph` does not discard them.
    observed_parent: BTreeMap<String, String>,
}

impl SessionState {
    /// Fold in a declared graph's structure, keeping the executed prefix.
    pub(super) fn adopt_structure(&mut self, structure: SessionStructure) {
        self.structure = structure;
        self.declared = true;
    }

    /// Whether a graph has been declared for this session.
    #[must_use]
    pub fn has_declared_graph(&self) -> bool {
        self.declared
    }

    pub(super) fn structure(&self) -> &SessionStructure {
        &self.structure
    }

    /// The budget in force. The IR default when no graph was declared, which
    /// mirrors `WorkflowSpec`'s own defaults.
    #[must_use]
    pub fn budget(&self) -> GraphBudget {
        self.structure.budget
    }

    /// Override the budget without declaring a graph. For a plain session,
    /// whose cap comes from configuration rather than from a spec.
    pub fn set_budget(&mut self, budget: GraphBudget) {
        self.structure.budget = budget;
    }

    /// Agents spawned over the session's lifetime.
    #[must_use]
    pub fn agents_spawned(&self) -> u32 {
        self.agents_spawned
    }

    /// Agents currently live.
    #[must_use]
    pub fn live_agents(&self) -> u32 {
        self.live_agents
    }

    /// Whether `node_id` is started and not yet finished.
    #[must_use]
    pub fn is_live(&self, node_id: &str) -> bool {
        self.started.contains(node_id) && !self.finished.contains(node_id)
    }

    /// Gates passed so far.
    #[must_use]
    pub fn gates_passed(&self) -> Vec<String> {
        self.gates_passed.iter().cloned().collect()
    }

    /// Rounds recorded for `node_id`.
    #[must_use]
    pub fn round(&self, node_id: &str) -> u32 {
        self.rounds.get(node_id).copied().unwrap_or(0)
    }

    pub(super) fn start_node(&mut self, node_id: &str) {
        if node_id.is_empty() {
            return;
        }
        self.finished.remove(node_id);
        self.started.insert(node_id.to_string());
    }

    pub(super) fn finish_node(&mut self, node_id: &str) {
        self.finished.insert(node_id.to_string());
        self.release_claims(node_id);
        self.finish_agent(node_id);
    }

    pub(super) fn pass_gate(&mut self, node_id: &str) {
        if node_id.is_empty() {
            return;
        }
        self.gates_passed.insert(node_id.to_string());
    }

    pub(super) fn record_round(&mut self, node_id: &str) {
        *self.rounds.entry(node_id.to_string()).or_insert(0) += 1;
    }

    /// Account for an admitted spawn.
    pub(super) fn record_spawn(&mut self, node_id: &str, parent_id: Option<&str>) {
        self.agents_spawned = self.agents_spawned.saturating_add(1);
        self.live_agents = self.live_agents.saturating_add(1);
        self.start_node(node_id);
        if let Some(parent) = parent_id.filter(|parent| !parent.is_empty() && *parent != node_id) {
            self.observed_parent
                .insert(node_id.to_string(), parent.to_string());
        }
    }

    /// Release one live agent slot. The lifetime total stands.
    pub(super) fn finish_agent(&mut self, node_id: &str) {
        if !node_id.is_empty() {
            self.finished.insert(node_id.to_string());
        }
        self.live_agents = self.live_agents.saturating_sub(1);
    }

    /// Claims held by nodes other than `node_id` that overlap `key`.
    pub(super) fn conflicting_claims(
        &self,
        node_id: &str,
        key: &ResourceKey,
    ) -> Option<&WriteClaim> {
        self.claims.iter().find(|claim| {
            claim.node_id != node_id
                && self.is_live(&claim.node_id)
                && !self.are_related(node_id, &claim.node_id)
                && archon_workflow::write_coordinator::write_plan::keys_conflict(&claim.key, key)
        })
    }

    /// Record a claim under the key its declarer chose.
    ///
    /// The key is built by the caller rather than here, because the same path
    /// is an exclusive claim or a coordinated-append claim depending on what
    /// the write intent declared, and that distinction is not recoverable from
    /// the path string.
    ///
    /// Re-claiming the same key by the same node is a no-op.
    pub(super) fn claim(&mut self, node_id: &str, declared: &str, key: ResourceKey) {
        if self
            .claims
            .iter()
            .any(|claim| claim.node_id == node_id && claim.key == key)
        {
            return;
        }
        self.claims.push(WriteClaim {
            node_id: node_id.to_string(),
            key,
            declared: declared.to_string(),
        });
    }

    pub(super) fn release_claims(&mut self, node_id: &str) {
        self.claims.retain(|claim| claim.node_id != node_id);
    }

    /// Whether a dependency path connects `a` and `b` in either direction.
    ///
    /// Two nodes joined by a path are ordered relative to one another, so they
    /// cannot write concurrently and there is no conflict to report. Unrelated
    /// live nodes are the case invariant 2 exists for.
    ///
    /// Declared reachability first; then the observed spawn chain, which is all
    /// an undeclared turn has. Same node is trivially related, which is what
    /// makes a node re-writing its own file free.
    #[must_use]
    pub fn are_related(&self, a: &str, b: &str) -> bool {
        if a == b {
            return true;
        }
        if self
            .structure
            .related
            .get(a)
            .is_some_and(|reachable| reachable.contains(b))
        {
            return true;
        }
        self.observed_ancestor(a, b) || self.observed_ancestor(b, a)
    }

    /// Whether `ancestor` is reachable by walking `node`'s observed parents.
    ///
    /// Bounded by the number of recorded edges, so a cycle introduced by
    /// bookkeeping cannot spin.
    fn observed_ancestor(&self, node: &str, ancestor: &str) -> bool {
        let mut current = node;
        for _ in 0..self.observed_parent.len() {
            match self.observed_parent.get(current) {
                Some(parent) if parent == ancestor => return true,
                Some(parent) => current = parent,
                None => return false,
            }
        }
        false
    }
}
