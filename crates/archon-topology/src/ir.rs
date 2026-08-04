//! The topology intermediate representation.
//!
//! One graph type that `/workflow` specs, team subtask lists, and (from
//! milestone 2) reconstructed session turns all lower into, so analysis and
//! recording are written once instead of three times.

use serde::{Deserialize, Serialize};

/// Which surface produced this graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum GraphOrigin {
    Workflow { run_id: String },
    Team { session_id: String },
    Session { session_id: String },
}

/// What a gate gates. `Quality` is distinct from [`NodeRole::Verify`]: a
/// `QualityGate` stage runs checks and reports, whereas a gate *blocks*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateKind {
    /// A human must approve before downstream work proceeds.
    Human,
    /// A recorded resumption point that downstream work must not precede.
    Checkpoint,
}

/// The job a node performs. Analyses key off this rather than off the
/// originating surface's own stage vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeRole {
    /// Control or planning; produces a plan, not an artifact.
    Plan,
    /// Ordinary work: an agent or implementation step.
    Work,
    /// Checks the output of upstream work. Reports; does not block.
    Verify,
    /// Folds many upstream outputs into one.
    Reduce,
    /// Blocks downstream work until satisfied.
    Gate(GateKind),
    /// A direct tool invocation.
    Tool,
}

impl NodeRole {
    /// True when this node blocks downstream work. Used by the dominator
    /// computation in [`crate::analysis`].
    #[must_use]
    pub fn is_gate(self) -> bool {
        matches!(self, NodeRole::Gate(_))
    }
}

/// A reference to data produced by another node.
///
/// Today the only recoverable dataflow in the whole tree is a fan-out
/// `foreach: "${producer.items}"`, so `accessor` is `"items"` in practice.
/// The field is kept general because milestone 4 extends `${stage.accessor}`
/// interpolation beyond `foreach`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DataRef {
    /// Node id that produces the referenced value.
    pub producer: String,
    /// Accessor on the producer's output, e.g. `items`.
    pub accessor: String,
}

impl DataRef {
    #[must_use]
    pub fn new(producer: impl Into<String>, accessor: impl Into<String>) -> Self {
        Self {
            producer: producer.into(),
            accessor: accessor.into(),
        }
    }
}

/// Something a node writes to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum WriteTarget {
    /// A repository-relative file path or glob.
    Path(String),
    /// A named artifact in the run store.
    Artifact(String),
}

/// How dangerous a node's effects are.
///
/// `Safe` is the floor rather than an assertion of safety: a lowering that has
/// no permission information reports `Safe`, because milestone 3 must never
/// fail closed on missing bookkeeping.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum PermissionClass {
    #[default]
    Safe,
    Risky,
    /// Effects that cannot be undone from inside the run: push, deploy,
    /// publish, force-delete.
    Irreversible,
}

/// Fan-out over a collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FanoutSpec {
    /// Where the items come from. `None` when the items are inline literals,
    /// which `WorkflowSpec` explicitly permits — an inline list is a complete,
    /// self-contained source with no producer to point at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<DataRef>,
    /// Per-stage concurrency cap, if the source declared one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_parallelism: Option<u32>,
}

/// Resource ceilings the graph as a whole is allowed to consume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphBudget {
    /// Maximum nodes that may run concurrently.
    pub max_parallelism: u32,
    /// Maximum agents over the graph's whole lifetime — not a concurrency cap.
    pub max_agents: u32,
    /// Maximum loop iterations. No surface declares loops today, so lowerings
    /// derive this from retry budgets or report 1.
    pub max_rounds: u32,
}

impl Default for GraphBudget {
    fn default() -> Self {
        // Mirrors WorkflowSpec::default_max_parallelism / default_max_agents so
        // a lowering that finds no budget agrees with the workflow defaults.
        Self {
            max_parallelism: 8,
            max_agents: 200,
            max_rounds: 1,
        }
    }
}

/// One unit of work in the graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskNode {
    pub id: String,
    pub role: NodeRole,
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Declared dataflow.
    ///
    /// **Empty means *unknown*, not *nothing*.** The `Subtask` lowering has no
    /// dataflow to give, so an empty vec is the common case. Any analysis that
    /// reasons from dataflow must treat empty as "cannot conclude" and stay
    /// silent rather than emit a false positive. See
    /// [`TaskNode::dataflow_is_known`].
    #[serde(default)]
    pub consumes: Vec<DataRef>,
    /// File globs and artifact keys this node writes. Also unknown when empty.
    #[serde(default)]
    pub writes: Vec<WriteTarget>,
    /// File globs and artifact keys this node *reads*, in the same target
    /// vocabulary as [`TaskNode::writes`].
    ///
    /// This is the other half of the dataflow contract, and it is what makes
    /// the milestone 4 lints computable. [`TaskNode::consumes`] can only say
    /// "node P produced something I use" and therefore cannot be populated by a
    /// surface that names resources rather than producers; `reads` names the
    /// resource and lets the analysis resolve the producer itself.
    ///
    /// **Empty means *unknown*, not *reads nothing*** — the same rule as
    /// `consumes` and `writes`. See [`TaskNode::reads_are_known`].
    #[serde(default)]
    pub reads: Vec<WriteTarget>,
    #[serde(default)]
    pub permission: PermissionClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fanout: Option<FanoutSpec>,
}

impl TaskNode {
    /// A node with the given id and role and everything else unknown/default.
    #[must_use]
    pub fn new(id: impl Into<String>, role: NodeRole) -> Self {
        Self {
            id: id.into(),
            role,
            depends_on: Vec::new(),
            consumes: Vec::new(),
            writes: Vec::new(),
            reads: Vec::new(),
            permission: PermissionClass::Safe,
            agent: None,
            fanout: None,
        }
    }

    /// False when `consumes` is empty, i.e. when dataflow is *unknown*.
    ///
    /// A node that genuinely consumes nothing is indistinguishable from one
    /// whose dataflow was never declared, and the IR deliberately does not
    /// pretend otherwise. Callers must branch on this before concluding
    /// anything from `consumes`.
    #[must_use]
    pub fn dataflow_is_known(&self) -> bool {
        !self.consumes.is_empty()
    }

    /// False when `writes` is empty, i.e. when write targets are *unknown*.
    #[must_use]
    pub fn writes_are_known(&self) -> bool {
        !self.writes.is_empty()
    }

    /// False when `reads` is empty, i.e. when read targets are *unknown*.
    ///
    /// A node that genuinely reads nothing is indistinguishable from one whose
    /// reads were never declared or never observed, so the dataflow lints treat
    /// empty as "cannot conclude" and stay silent.
    #[must_use]
    pub fn reads_are_known(&self) -> bool {
        !self.reads.is_empty()
    }

    /// True when this node declares *some* consumption — either resolved
    /// producer references (`consumes`) or named read targets (`reads`).
    #[must_use]
    pub fn consumption_is_known(&self) -> bool {
        self.dataflow_is_known() || self.reads_are_known()
    }
}

/// The IR itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskGraph {
    pub id: String,
    pub origin: GraphOrigin,
    pub nodes: Vec<TaskNode>,
    #[serde(default)]
    pub budget: GraphBudget,
}

impl TaskGraph {
    #[must_use]
    pub fn new(id: impl Into<String>, origin: GraphOrigin) -> Self {
        Self {
            id: id.into(),
            origin,
            nodes: Vec::new(),
            budget: GraphBudget::default(),
        }
    }

    #[must_use]
    pub fn node(&self, id: &str) -> Option<&TaskNode> {
        self.nodes.iter().find(|node| node.id == id)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// True when *every* node declares its dataflow. Dataflow lints (milestone
    /// 4) are only meaningful on a graph for which this holds.
    #[must_use]
    pub fn dataflow_is_complete(&self) -> bool {
        !self.nodes.is_empty() && self.nodes.iter().all(TaskNode::dataflow_is_known)
    }
}
