//! Unit tests for the five pure analyses.

use archon_topology::{
    DataRef, GateKind, GraphBudget, GraphOrigin, NodeRole, PermissionClass, TaskGraph, TaskNode,
    TopologyError, WriteTarget,
};

fn graph(nodes: Vec<TaskNode>) -> TaskGraph {
    TaskGraph {
        id: "g".into(),
        origin: GraphOrigin::Session {
            session_id: "s".into(),
        },
        nodes,
        budget: GraphBudget::default(),
    }
}

fn work(id: &str, depends_on: &[&str]) -> TaskNode {
    TaskNode {
        depends_on: depends_on.iter().map(|d| (*d).to_string()).collect(),
        ..TaskNode::new(id, NodeRole::Work)
    }
}

// ---------------------------------------------------------------- waves

#[test]
fn waves_linear_chain_is_one_node_per_wave() {
    let g = graph(vec![work("A", &[]), work("B", &["A"]), work("C", &["B"])]);
    assert_eq!(
        g.waves().expect("valid dag"),
        vec![vec!["A"], vec!["B"], vec!["C"]]
    );
}

#[test]
fn waves_independent_nodes_share_one_wave_in_graph_order() {
    let g = graph(vec![work("A", &[]), work("B", &[]), work("C", &[])]);
    assert_eq!(g.waves().expect("valid dag"), vec![vec!["A", "B", "C"]]);
}

#[test]
fn waves_diamond_puts_both_middles_in_one_wave() {
    let g = graph(vec![
        work("root", &[]),
        work("left", &["root"]),
        work("right", &["root"]),
        work("join", &["left", "right"]),
    ]);
    assert_eq!(
        g.waves().expect("valid dag"),
        vec![vec!["root"], vec!["left", "right"], vec!["join"]]
    );
}

#[test]
fn waves_node_sits_one_past_its_deepest_dependency() {
    // `join` depends on a depth-0 node and a depth-2 node, so it lands at 3 —
    // not at 1, which a breadth-first assignment would produce.
    let g = graph(vec![
        work("a", &[]),
        work("b", &["a"]),
        work("c", &["b"]),
        work("shallow", &[]),
        work("join", &["shallow", "c"]),
    ]);
    let waves = g.waves().expect("valid dag");
    assert_eq!(waves.len(), 4);
    assert_eq!(waves[3], vec!["join"]);
}

#[test]
fn waves_empty_graph_is_empty() {
    assert!(graph(Vec::new()).waves().expect("valid").is_empty());
}

#[test]
fn waves_rejects_cycles() {
    let g = graph(vec![work("A", &["B"]), work("B", &["A"])]);
    assert_eq!(g.waves(), Err(TopologyError::Cycle));
}

#[test]
fn waves_rejects_unknown_dependency_before_checking_cycles() {
    // Both defects present: the unknown id must win, so a typo is reported as
    // a typo rather than as a cycle.
    let g = graph(vec![work("A", &["ghost", "B"]), work("B", &["A"])]);
    assert_eq!(
        g.waves(),
        Err(TopologyError::UnknownDependency {
            node: "A".into(),
            dependency: "ghost".into(),
        })
    );
}

#[test]
fn waves_rejects_duplicate_ids() {
    let g = graph(vec![work("A", &[]), work("A", &[])]);
    assert_eq!(
        g.waves(),
        Err(TopologyError::DuplicateNode { id: "A".into() })
    );
}

// -------------------------------------------------------- critical path

#[test]
fn critical_path_follows_the_longest_chain() {
    let g = graph(vec![
        work("a", &[]),
        work("b", &["a"]),
        work("c", &["b"]),
        work("shallow", &[]),
        work("join", &["shallow", "c"]),
    ]);
    let path = g.critical_path().expect("valid dag");
    assert_eq!(path.nodes, vec!["a", "b", "c", "join"]);
    assert_eq!(path.span(), 4);
}

#[test]
fn critical_path_of_independent_nodes_is_a_single_node() {
    let g = graph(vec![work("A", &[]), work("B", &[]), work("C", &[])]);
    let path = g.critical_path().expect("valid dag");
    assert_eq!(path.span(), 1);
    // Ties break toward the earliest node, so the result is stable run to run.
    assert_eq!(path.nodes, vec!["A"]);
}

#[test]
fn critical_path_of_empty_graph_is_empty() {
    assert_eq!(graph(Vec::new()).critical_path().expect("valid").span(), 0);
}

// -------------------------------------------------- parallelism profile

#[test]
fn parallelism_profile_reports_unusable_budget() {
    // Peak width 2 against the default budget of 8: six reserved slots the
    // graph's shape can never fill.
    let g = graph(vec![
        work("root", &[]),
        work("left", &["root"]),
        work("right", &["root"]),
    ]);
    let profile = g.parallelism_profile().expect("valid dag");
    assert_eq!(profile.wave_widths, vec![1, 2]);
    assert_eq!(profile.peak_width, 2);
    assert_eq!(profile.unusable_slots, 6);
    assert!(profile.over_provisioned());
    assert!(!profile.budget_limited());
}

#[test]
fn parallelism_profile_reports_waves_wider_than_budget() {
    let mut g = graph(vec![work("a", &[]), work("b", &[]), work("c", &[])]);
    g.budget.max_parallelism = 2;
    let profile = g.parallelism_profile().expect("valid dag");
    assert_eq!(profile.budget_limited_waves, vec![(0, 3)]);
    assert!(profile.budget_limited());
    assert_eq!(profile.unusable_slots, 0);
    assert!(!profile.over_provisioned());
}

// ------------------------------------------------------ write conflicts

fn writing(id: &str, depends_on: &[&str], paths: &[&str]) -> TaskNode {
    TaskNode {
        writes: paths
            .iter()
            .map(|p| WriteTarget::Path((*p).to_string()))
            .collect(),
        ..work(id, depends_on)
    }
}

#[test]
fn write_conflicts_flags_unordered_overlapping_writers() {
    let g = graph(vec![
        writing("left", &[], &["src/lib.rs", "src/a.rs"]),
        writing("right", &[], &["src/lib.rs"]),
    ]);
    let conflicts = g.write_conflicts().expect("valid dag");
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].left, "left");
    assert_eq!(conflicts[0].right, "right");
    assert_eq!(
        conflicts[0].targets,
        vec![WriteTarget::Path("src/lib.rs".into())]
    );
}

#[test]
fn write_conflicts_ignores_writers_ordered_by_a_dependency_path() {
    // Transitive ordering counts: `late` cannot start until `early` finished.
    let g = graph(vec![
        writing("early", &[], &["src/lib.rs"]),
        writing("middle", &["early"], &[]),
        writing("late", &["middle"], &["src/lib.rs"]),
    ]);
    assert!(g.write_conflicts().expect("valid dag").is_empty());
}

#[test]
fn write_conflicts_is_silent_when_writes_are_unknown() {
    // Empty `writes` means unknown, not "writes nothing" — two nodes with no
    // declared targets must not be reported as conflict-free *or* conflicting.
    let g = graph(vec![work("a", &[]), work("b", &[])]);
    assert!(g.write_conflicts().expect("valid dag").is_empty());

    // And a known writer must not conflict with an unknown one.
    let mixed = graph(vec![writing("a", &[], &["x.rs"]), work("b", &[])]);
    assert!(mixed.write_conflicts().expect("valid dag").is_empty());
}

#[test]
fn write_conflicts_distinguishes_paths_from_artifacts() {
    let g = graph(vec![
        TaskNode {
            writes: vec![WriteTarget::Path("report".into())],
            ..work("a", &[])
        },
        TaskNode {
            writes: vec![WriteTarget::Artifact("report".into())],
            ..work("b", &[])
        },
    ]);
    assert!(g.write_conflicts().expect("valid dag").is_empty());
}

// --------------------------------------------------- gate dominance

fn irreversible(id: &str, depends_on: &[&str]) -> TaskNode {
    TaskNode {
        permission: PermissionClass::Irreversible,
        ..work(id, depends_on)
    }
}

fn gate(id: &str, depends_on: &[&str]) -> TaskNode {
    TaskNode {
        depends_on: depends_on.iter().map(|d| (*d).to_string()).collect(),
        ..TaskNode::new(id, NodeRole::Gate(GateKind::Human))
    }
}

#[test]
fn ungated_irreversible_flags_a_deploy_with_no_gate() {
    let g = graph(vec![work("build", &[]), irreversible("deploy", &["build"])]);
    assert_eq!(g.ungated_irreversible().expect("valid dag"), vec!["deploy"]);
}

#[test]
fn ungated_irreversible_is_silent_when_a_gate_dominates() {
    let g = graph(vec![
        work("build", &[]),
        gate("approve", &["build"]),
        irreversible("deploy", &["approve"]),
    ]);
    assert!(g.ungated_irreversible().expect("valid dag").is_empty());
    assert_eq!(
        g.dominating_gates().expect("valid dag")["deploy"],
        vec!["approve"]
    );
}

#[test]
fn ungated_irreversible_flags_a_gate_that_can_be_bypassed() {
    // `deploy` is reachable both through the gate and around it, so the gate
    // does not dominate it. This is the case a naive "is there a gate
    // upstream?" check gets wrong, and the reason a real dominator
    // computation is required.
    let g = graph(vec![
        work("build", &[]),
        gate("approve", &["build"]),
        work("hotfix", &["build"]),
        irreversible("deploy", &["approve", "hotfix"]),
    ]);
    assert_eq!(g.ungated_irreversible().expect("valid dag"), vec!["deploy"]);
    assert!(g.dominating_gates().expect("valid dag")["deploy"].is_empty());
}

#[test]
fn ungated_irreversible_flags_an_irreversible_root() {
    let g = graph(vec![irreversible("rm-rf", &[]), gate("approve", &[])]);
    assert_eq!(g.ungated_irreversible().expect("valid dag"), vec!["rm-rf"]);
}

#[test]
fn ungated_irreversible_is_silent_when_nothing_is_irreversible() {
    // The common case for the Subtask lowering, which has no permission
    // information and reports everything Safe.
    let g = graph(vec![work("a", &[]), work("b", &["a"])]);
    assert!(g.ungated_irreversible().expect("valid dag").is_empty());
}

#[test]
fn gate_nodes_lists_gates_in_graph_order() {
    let g = graph(vec![
        work("a", &[]),
        gate("g1", &["a"]),
        gate("g2", &["g1"]),
    ]);
    assert_eq!(g.gate_nodes(), vec!["g1", "g2"]);
}

// ------------------------------------------------ unknown-dataflow rule

#[test]
fn empty_consumes_reads_as_unknown_not_as_nothing() {
    let node = TaskNode::new("a", NodeRole::Work);
    assert!(!node.dataflow_is_known());
    assert!(!node.writes_are_known());

    let known = TaskNode {
        consumes: vec![DataRef::new("producer", "items")],
        ..TaskNode::new("b", NodeRole::Work)
    };
    assert!(known.dataflow_is_known());

    assert!(!graph(vec![work("a", &[])]).dataflow_is_complete());
    assert!(!graph(Vec::new()).dataflow_is_complete());
    assert!(graph(vec![known]).dataflow_is_complete());
}

#[test]
fn ir_round_trips_through_json() {
    let g = graph(vec![
        TaskNode {
            consumes: vec![DataRef::new("producer", "items")],
            writes: vec![WriteTarget::Path("src/lib.rs".into())],
            permission: PermissionClass::Irreversible,
            agent: Some("coder".into()),
            ..work("a", &[])
        },
        gate("g", &["a"]),
    ]);
    let json = serde_json::to_string(&g).expect("serialize");
    let back: TaskGraph = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, g);
}
