use super::*;
use crate::trace::TraceRecord;

fn record(kind: TraceKind, node: &str) -> TraceRecord {
    TraceRecord::new("2026-08-02T00:00:00Z", "g1", kind).with_node(node)
}

#[test]
fn spawns_become_nodes_and_parentage_becomes_edges() {
    let records = vec![
        record(TraceKind::AgentSpawned, "child-a")
            .with_parent("turn")
            .with_agent("explorer"),
        record(TraceKind::AgentSpawned, "child-b")
            .with_parent("turn")
            .with_agent("reviewer"),
        record(TraceKind::AgentSpawned, "grandchild")
            .with_parent("child-a")
            .with_agent("worker"),
    ];

    let graph = reconstruct_graph(
        "g1",
        GraphOrigin::Session {
            session_id: "s1".into(),
        },
        &records,
    );

    assert_eq!(graph.len(), 4, "three spawns plus the synthesized root");
    assert_eq!(graph.node("child-a").unwrap().depends_on, vec!["turn"]);
    assert_eq!(graph.node("child-b").unwrap().depends_on, vec!["turn"]);
    assert_eq!(
        graph.node("grandchild").unwrap().depends_on,
        vec!["child-a"]
    );
    assert!(graph.node("turn").unwrap().depends_on.is_empty());
    assert_eq!(
        graph.node("child-b").unwrap().agent.as_deref(),
        Some("reviewer")
    );
    assert!(matches!(
        graph.origin,
        GraphOrigin::Session { ref session_id } if session_id == "s1"
    ));
}

/// The read side of the same accumulation. Without it a reconstructed graph
/// carries only half of every dataflow and the fusion lint can never see a
/// coupling.
#[test]
fn files_read_become_reads_without_becoming_writes() {
    let records = vec![
        record(TraceKind::FileRead, "a").with_reads(vec![WriteTarget::Path("src/a.rs".into())]),
        record(TraceKind::FileRead, "a").with_reads(vec![WriteTarget::Path("src/b.rs".into())]),
        record(TraceKind::FileRead, "a").with_reads(vec![WriteTarget::Path("src/a.rs".into())]),
    ];

    let graph = reconstruct_graph(
        "g1",
        GraphOrigin::Session {
            session_id: "s1".into(),
        },
        &records,
    );

    let node = graph.node("a").expect("node a");
    assert_eq!(
        node.reads,
        vec![
            WriteTarget::Path("src/a.rs".into()),
            WriteTarget::Path("src/b.rs".into()),
        ],
        "sorted and deduplicated, like writes"
    );
    assert!(node.writes.is_empty(), "a read is not a write");
    assert!(node.reads_are_known());
}

/// A record written before `reads` existed decodes with the field absent, which
/// must read as *unknown* rather than as an assertion that nothing was read.
#[test]
fn a_record_without_the_reads_field_leaves_reads_unknown() {
    let line = r#"{"ts":"2026-08-02T00:00:00Z","graph_id":"g1","node_id":"a","kind":"file_written","writes":[{"kind":"path","value":"src/a.rs"}]}"#;
    let record: TraceRecord = serde_json::from_str(line).expect("older record still parses");
    assert!(record.reads.is_empty());

    let graph = reconstruct_graph(
        "g1",
        GraphOrigin::Session {
            session_id: "s1".into(),
        },
        &[record],
    );
    assert!(!graph.node("a").expect("node a").reads_are_known());
}

#[test]
fn files_written_become_writes() {
    let records = vec![
        record(TraceKind::FileWritten, "a").with_writes(vec![WriteTarget::Path("src/a.rs".into())]),
        record(TraceKind::FileWritten, "a").with_writes(vec![WriteTarget::Path("src/b.rs".into())]),
        // Duplicate: must be deduplicated, not double-counted.
        record(TraceKind::FileWritten, "a").with_writes(vec![WriteTarget::Path("src/a.rs".into())]),
    ];

    let graph = reconstruct_graph(
        "g1",
        GraphOrigin::Session {
            session_id: "s1".into(),
        },
        &records,
    );
    assert_eq!(
        graph.node("a").unwrap().writes,
        vec![
            WriteTarget::Path("src/a.rs".into()),
            WriteTarget::Path("src/b.rs".into()),
        ]
    );
}

#[test]
fn unattributed_records_land_on_the_synthesized_root() {
    let records = vec![
        TraceRecord::new("t", "g1", TraceKind::ToolAttempt).with_tool("Read"),
        TraceRecord::new("t", "g1", TraceKind::ToolAttempt).with_tool("Bash"),
    ];

    let graph = reconstruct_graph(
        "g1",
        GraphOrigin::Session {
            session_id: "s1".into(),
        },
        &records,
    );
    assert_eq!(graph.len(), 1);
    assert_eq!(graph.nodes[0].id, ROOT_NODE_ID);
    assert_eq!(graph.nodes[0].role, NodeRole::Tool);
}

#[test]
fn consumes_stays_empty_because_dataflow_is_unknown() {
    let records = vec![record(TraceKind::AgentSpawned, "a").with_parent("turn")];
    let graph = reconstruct_graph(
        "g1",
        GraphOrigin::Session {
            session_id: "s1".into(),
        },
        &records,
    );

    assert!(graph.nodes.iter().all(|node| node.consumes.is_empty()));
    assert!(
        !graph.dataflow_is_complete(),
        "a reconstruction must never claim complete dataflow"
    );
}

#[test]
fn permission_is_the_high_water_mark_across_a_nodes_records() {
    let records = vec![
        record(TraceKind::ToolAttempt, "a").with_permission(PermissionClass::Safe),
        record(TraceKind::ToolAttempt, "a").with_permission(PermissionClass::Irreversible),
        record(TraceKind::ToolAttempt, "a").with_permission(PermissionClass::Risky),
    ];

    let graph = reconstruct_graph(
        "g1",
        GraphOrigin::Session {
            session_id: "s1".into(),
        },
        &records,
    );
    assert_eq!(
        graph.node("a").unwrap().permission,
        PermissionClass::Irreversible
    );
}

#[test]
fn unknown_and_graph_declared_records_are_skipped() {
    let records = vec![
        TraceRecord::new("t", "g1", TraceKind::GraphDeclared),
        TraceRecord::new("t", "g1", TraceKind::Unknown).with_node("ghost"),
        record(TraceKind::ToolAttempt, "real"),
    ];

    let graph = reconstruct_graph(
        "g1",
        GraphOrigin::Session {
            session_id: "s1".into(),
        },
        &records,
    );
    assert_eq!(graph.len(), 1);
    assert_eq!(graph.nodes[0].id, "real");
}

#[test]
fn reconstruction_is_deterministic() {
    let records = vec![
        record(TraceKind::AgentSpawned, "b").with_parent("turn"),
        record(TraceKind::AgentSpawned, "a").with_parent("turn"),
        record(TraceKind::FileWritten, "a").with_writes(vec![
            WriteTarget::Path("z.rs".into()),
            WriteTarget::Path("a.rs".into()),
        ]),
    ];

    let first = reconstruct_graph(
        "g1",
        GraphOrigin::Session {
            session_id: "s1".into(),
        },
        &records,
    );
    for _ in 0..8 {
        assert_eq!(
            reconstruct_graph(
                "g1",
                GraphOrigin::Session {
                    session_id: "s1".into()
                },
                &records
            ),
            first
        );
    }
    // First-appearance order for nodes, sorted order within a node.
    assert_eq!(
        first
            .nodes
            .iter()
            .map(|n| n.id.as_str())
            .collect::<Vec<_>>(),
        vec!["b", "turn", "a"]
    );
    assert_eq!(
        first.node("a").unwrap().writes,
        vec![
            WriteTarget::Path("a.rs".into()),
            WriteTarget::Path("z.rs".into())
        ]
    );
}

#[test]
fn a_reconstructed_graph_is_analysable() {
    let records = vec![
        record(TraceKind::AgentSpawned, "a").with_parent("turn"),
        record(TraceKind::AgentSpawned, "b").with_parent("turn"),
        record(TraceKind::AgentSpawned, "c").with_parent("a"),
    ];

    let graph = reconstruct_graph(
        "g1",
        GraphOrigin::Session {
            session_id: "s1".into(),
        },
        &records,
    );
    let waves = graph.waves().expect("a spawn tree is always acyclic");
    assert_eq!(waves, vec![vec!["turn"], vec!["a", "b"], vec!["c"]]);
    assert_eq!(graph.critical_path().unwrap().span(), 3);
}

#[test]
fn a_gate_pass_produces_a_gate_node() {
    let records = vec![record(TraceKind::GatePassed, "checkpoint-1")];
    let graph = reconstruct_graph(
        "g1",
        GraphOrigin::Session {
            session_id: "s1".into(),
        },
        &records,
    );
    assert_eq!(
        graph.node("checkpoint-1").unwrap().role,
        NodeRole::Gate(crate::ir::GateKind::Checkpoint)
    );
}

#[test]
fn verification_only_nodes_read_as_verifiers() {
    let records = vec![record(TraceKind::Verification, "v1")];
    let graph = reconstruct_graph(
        "g1",
        GraphOrigin::Session {
            session_id: "s1".into(),
        },
        &records,
    );
    assert_eq!(graph.node("v1").unwrap().role, NodeRole::Verify);
}

#[test]
fn retries_are_counted_from_explicit_records_and_from_attempt_numbers() {
    let records = vec![
        record(TraceKind::ToolAttempt, "a").with_attempt(0),
        record(TraceKind::ToolAttempt, "a").with_attempt(1),
        record(TraceKind::ToolAttempt, "a").with_attempt(2),
        record(TraceKind::Retry, "b"),
        record(TraceKind::ToolAttempt, "c").with_attempt(0),
    ];

    let retries = observed_retries(&records);
    assert_eq!(retries.get("a"), Some(&2));
    assert_eq!(retries.get("b"), Some(&1));
    assert_eq!(retries.get("c"), None);
}

#[test]
fn an_empty_trace_reconstructs_an_empty_graph() {
    let graph = reconstruct_graph(
        "g1",
        GraphOrigin::Session {
            session_id: "s1".into(),
        },
        &[],
    );
    assert!(graph.is_empty());
    assert_eq!(graph.budget.max_agents, 1);
}

#[test]
fn the_caller_chooses_the_origin() {
    // A workflow run whose events.jsonl is projected after the fact is still a
    // workflow; the reconstruction must not relabel it a session.
    let graph = reconstruct_graph(
        "wf-1",
        GraphOrigin::Workflow {
            run_id: "wf-1".into(),
        },
        &[record(TraceKind::NodeStarted, "stage-a")],
    );
    assert!(matches!(
        graph.origin,
        GraphOrigin::Workflow { ref run_id } if run_id == "wf-1"
    ));
}

#[test]
fn a_self_parenting_record_does_not_create_a_cycle() {
    let records = vec![record(TraceKind::AgentSpawned, "a").with_parent("a")];
    let graph = reconstruct_graph(
        "g1",
        GraphOrigin::Session {
            session_id: "s1".into(),
        },
        &records,
    );
    assert!(graph.node("a").unwrap().depends_on.is_empty());
    assert!(graph.waves().is_ok());
}
