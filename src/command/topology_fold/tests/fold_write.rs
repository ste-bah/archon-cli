//! What one fold puts where.
//!
//! Row counts across the three topology relations, the single learning summary
//! and the shape metrics it carries, and the degraded inputs a fold must still
//! survive: an empty trace, a truncated one, and a missing learning store.

use std::io::Write;

use archon_topology::ir::{GraphOrigin, NodeRole, TaskGraph, TaskNode};
use cozo::{DataValue, ScriptMutability};

use super::*;

#[test]
fn a_fold_writes_all_three_relations_and_exactly_one_learning_row() {
    let _guard = store_lock();
    let fixture = fixture();
    write_sample_trace(&fixture.paths, "g1");

    let outcome = fold_graph(
        &fixture.paths,
        "g1",
        "s1",
        "fix the parser crash",
        &fixture.topology_db,
        Some(&fixture.learning_db),
        "workspace-1",
    )
    .unwrap();

    assert!(!outcome.already_ingested);
    assert!(outcome.reconstructed, "no graph.json was declared");
    assert_eq!(outcome.nodes_written, 3, "turn plus two children");
    assert_eq!(outcome.learning_rows_written, 1);

    assert_eq!(
        count_topology_rows(&fixture.topology_db, "topology_graph"),
        1
    );
    assert_eq!(
        count_topology_rows(&fixture.topology_db, "topology_node"),
        3
    );
    assert_eq!(
        count_topology_rows(&fixture.topology_db, "topology_outcome"),
        1
    );
    assert_eq!(
        count_learning_rows(&fixture.learning_db),
        1,
        "exactly one topology_outcome row per fold"
    );
    assert!(fixture.paths.is_ingested("g1"));
}

#[test]
fn a_truncated_trace_folds_without_error() {
    let _guard = store_lock();
    let fixture = fixture();
    let writer = write_sample_trace(&fixture.paths, "g1");
    std::fs::OpenOptions::new()
        .append(true)
        .open(writer.path())
        .unwrap()
        .write_all(br#"{"ts":"2026-08-02T00:00:05Z","graph_id":"g1","kind":"node_fin"#)
        .unwrap();

    let outcome = fold_graph(
        &fixture.paths,
        "g1",
        "s1",
        "fix the parser crash",
        &fixture.topology_db,
        Some(&fixture.learning_db),
        "workspace-1",
    )
    .unwrap();

    assert!(outcome.truncated_trace);
    assert_eq!(outcome.nodes_written, 3, "complete records still folded");
    assert_eq!(outcome.learning_rows_written, 1);
}

#[test]
fn an_empty_trace_folds_without_error() {
    let _guard = store_lock();
    let fixture = fixture();
    fixture.paths.writer("g-empty").unwrap();

    let outcome = fold_graph(
        &fixture.paths,
        "g-empty",
        "s1",
        "nothing happened",
        &fixture.topology_db,
        Some(&fixture.learning_db),
        "workspace-1",
    )
    .unwrap();

    assert_eq!(outcome.nodes_written, 0);
    assert_eq!(outcome.learning_rows_written, 1);
    assert!(fixture.paths.is_ingested("g-empty"));
}

#[test]
fn the_fold_writes_its_stores_with_no_learning_store_configured() {
    let _guard = store_lock();
    let fixture = fixture();
    write_sample_trace(&fixture.paths, "g1");

    let outcome = fold_graph(
        &fixture.paths,
        "g1",
        "s1",
        "fix the parser crash",
        &fixture.topology_db,
        None,
        "workspace-1",
    )
    .unwrap();

    assert_eq!(outcome.learning_rows_written, 0);
    assert_eq!(
        count_topology_rows(&fixture.topology_db, "topology_node"),
        3
    );
    assert!(fixture.paths.is_ingested("g1"));
}

#[test]
fn a_declared_graph_is_folded_instead_of_reconstructed() {
    let _guard = store_lock();
    let fixture = fixture();
    write_sample_trace(&fixture.paths, "g1");

    let mut declared = TaskGraph::new(
        "g1",
        GraphOrigin::Workflow {
            run_id: "wf-77".into(),
        },
    );
    declared.nodes.push(TaskNode::new("plan", NodeRole::Plan));
    let mut build = TaskNode::new("build", NodeRole::Work);
    build.depends_on = vec!["plan".into()];
    declared.nodes.push(build);
    fixture.paths.write_graph(&declared).unwrap();

    let outcome = fold_graph(
        &fixture.paths,
        "g1",
        "s1",
        "migrate the schema",
        &fixture.topology_db,
        Some(&fixture.learning_db),
        "workspace-1",
    )
    .unwrap();

    assert!(!outcome.reconstructed);
    assert_eq!(
        outcome.nodes_written, 2,
        "the declared graph, not the trace"
    );

    let rows = fixture
        .topology_db
        .run_script(
            "?[origin, run_id, task_hash, span] := *topology_graph{graph_id, origin, run_id, task_hash, span}, graph_id = 'g1'",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(rows.rows[0][0], DataValue::from("workflow"));
    assert_eq!(
        rows.rows[0][1],
        DataValue::from("wf-77"),
        "a declared graph keeps its own origin, not the fold's fallback"
    );
    assert!(
        rows.rows[0][2]
            .get_str()
            .is_some_and(|hash| hash.starts_with("migration:")),
        "unexpected task_hash: {:?}",
        rows.rows[0][2]
    );
    assert_eq!(
        rows.rows[0][3],
        DataValue::from(2i64),
        "span of plan -> build"
    );
}

#[test]
fn the_learning_summary_carries_the_shape_metrics() {
    let _guard = store_lock();
    let fixture = fixture();
    write_sample_trace(&fixture.paths, "g1");

    fold_graph(
        &fixture.paths,
        "g1",
        "s1",
        "fix the parser crash",
        &fixture.topology_db,
        Some(&fixture.learning_db),
        "workspace-1",
    )
    .unwrap();

    let events = archon_learning::store::list_all_learning_events(&fixture.learning_db).unwrap();
    let event = events
        .iter()
        .find(|event| {
            event.event_type == archon_learning::models::LearningEventType::TopologyOutcome
        })
        .expect("the summary row must exist");

    assert_eq!(event.event_id, "topology-outcome-g1");
    assert_eq!(event.source_artifact_id, "g1");
    assert_eq!(event.workspace_id, "workspace-1");

    let signal = &event.signal;
    assert_eq!(signal["graph_id"], "g1");
    assert!(
        signal["task_hash"]
            .as_str()
            .unwrap()
            .starts_with("bug-hunt:")
    );
    assert_eq!(signal["node_count"], 3);
    assert_eq!(signal["span"], 2, "root plus one child");
    assert_eq!(
        signal["max_parallelism_used"], 2,
        "two children in one wave"
    );
    assert_eq!(signal["retries_total"], 1);
    assert_eq!(signal["nodes_failed"], 1);
    assert_eq!(signal["reconstructed"], true);
    assert!(signal["wave_widths"].is_array());
    assert!(signal["fan_out_widths"].is_array());
    assert!(signal["gate_nodes"].is_array());
    assert!((0.0..=1.0).contains(&event.confidence));
}
