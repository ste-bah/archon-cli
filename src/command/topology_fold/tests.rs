use std::io::Write;
use std::sync::Arc;

use super::*;
use archon_topology::ir::TaskNode;
use archon_topology::trace::{TraceRecord, TraceWriter};

/// The Cozo script poison and the ambient trace slot are both process-global,
/// so every test here serializes on the shared topology test lock.
use crate::command::topology_trace::test_lock as store_lock;

struct Fixture {
    _temp: tempfile::TempDir,
    root: std::path::PathBuf,
    paths: TopologyPaths,
    topology_db: Arc<DbInstance>,
    learning_db: Arc<DbInstance>,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    std::fs::create_dir_all(root.join(".archon")).unwrap();

    let topology_db = open_db(&topology_db_path(&root));
    let learning_db = open_db(&root.join(".archon").join("learning-state.db"));

    Fixture {
        paths: TopologyPaths::for_project(&root),
        _temp: temp,
        root,
        topology_db,
        learning_db,
    }
}

fn open_db(path: &Path) -> Arc<DbInstance> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let path = path.to_string_lossy().to_string();
    archon_cozo::open_sqlite_guarded_instance(
        &path,
        "open topology test store",
        archon_cozo::CozoGuardConfig::for_db_path(&path),
    )
    .unwrap()
    .db_arc()
}

/// Write a small trace: a root that spawned two children, one of which failed.
fn write_sample_trace(paths: &TopologyPaths, graph_id: &str) -> TraceWriter {
    let writer = paths.writer(graph_id).unwrap();
    for record in [
        TraceRecord::new("2026-08-02T00:00:00Z", graph_id, TraceKind::AgentSpawned)
            .with_node("child-a")
            .with_parent("turn")
            .with_agent("worker"),
        TraceRecord::new("2026-08-02T00:00:01Z", graph_id, TraceKind::AgentSpawned)
            .with_node("child-b")
            .with_parent("turn")
            .with_agent("verifier"),
        TraceRecord::new("2026-08-02T00:00:02Z", graph_id, TraceKind::ToolAttempt)
            .with_node("child-a")
            .with_tool("Write")
            .with_attempt(1)
            .with_writes(vec![WriteTarget::Path("src/a.rs".into())])
            .with_duration_ms(120),
        TraceRecord::new("2026-08-02T00:00:03Z", graph_id, TraceKind::NodeFinished)
            .with_node("child-a"),
        TraceRecord::new("2026-08-02T00:00:04Z", graph_id, TraceKind::NodeFinished)
            .with_node("child-b")
            .with_outcome(false, true),
    ] {
        writer.append(&record).unwrap();
    }
    writer
}

fn count_learning_rows(db: &DbInstance) -> usize {
    archon_learning::store::list_all_learning_events(db)
        .unwrap()
        .iter()
        .filter(|event| {
            event.event_type == archon_learning::models::LearningEventType::TopologyOutcome
        })
        .count()
}

/// Count rows in a topology relation.
///
/// The projection must name **every** key column. Cozo query results are sets,
/// so projecting `topology_node` on `graph_id` alone collapses all of a graph's
/// nodes into one row and silently reports 1 no matter how many were written.
fn count_topology_rows(db: &DbInstance, relation: &str) -> usize {
    let projection = match relation {
        "topology_node" => "graph_id, node_id",
        _ => "graph_id",
    };
    db.run_script(
        &format!("?[{projection}] := *{relation}{{{projection}}}"),
        Default::default(),
        ScriptMutability::Immutable,
    )
    .map(|rows| rows.rows.len())
    .unwrap_or(0)
}

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
fn re_folding_the_same_trace_does_not_double_count() {
    let _guard = store_lock();
    let fixture = fixture();
    write_sample_trace(&fixture.paths, "g1");

    let first = fold_graph(
        &fixture.paths,
        "g1",
        "s1",
        "fix the parser crash",
        &fixture.topology_db,
        Some(&fixture.learning_db),
        "workspace-1",
    )
    .unwrap();
    assert_eq!(first.learning_rows_written, 1);

    let second = fold_graph(
        &fixture.paths,
        "g1",
        "s1",
        "fix the parser crash",
        &fixture.topology_db,
        Some(&fixture.learning_db),
        "workspace-1",
    )
    .unwrap();

    assert!(second.already_ingested);
    assert_eq!(
        second.learning_rows_written, 0,
        "a repeat fold must produce no learning row"
    );
    assert_eq!(second.nodes_written, 0);
    assert_eq!(count_learning_rows(&fixture.learning_db), 1);
    assert_eq!(
        count_topology_rows(&fixture.topology_db, "topology_node"),
        3
    );
}

#[test]
fn a_replayed_fold_after_a_lost_marker_upserts_rather_than_duplicating() {
    // Simulates a crash between the store writes and the marker: the marker is
    // written last precisely so this case replays. It must be harmless.
    let _guard = store_lock();
    let fixture = fixture();
    write_sample_trace(&fixture.paths, "g1");

    for _ in 0..3 {
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
        std::fs::remove_file(fixture.paths.ingested_marker("g1")).unwrap();
    }

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
        "the learning row id is derived from the graph id, so it upserts"
    );
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

#[test]
fn the_event_type_matches_the_relations_pascal_case_convention() {
    assert_eq!(
        archon_learning::models::LearningEventType::TopologyOutcome.as_str(),
        TOPOLOGY_OUTCOME_EVENT_TYPE
    );
    assert_eq!(
        archon_learning::models::LearningEventType::from_str(TOPOLOGY_OUTCOME_EVENT_TYPE),
        Some(archon_learning::models::LearningEventType::TopologyOutcome),
        "the design's snake_case spelling would not round-trip"
    );
}

#[test]
fn folding_all_pending_skips_already_ingested_graphs() {
    let _guard = store_lock();
    let fixture = fixture();
    write_sample_trace(&fixture.paths, "g1");
    write_sample_trace(&fixture.paths, "g2");
    fixture.paths.mark_ingested("g2", "earlier").unwrap();

    let outcomes = fold_pending_blocking(
        &fixture.paths,
        "s1",
        "fix the parser crash",
        &fixture.topology_db,
        Some(&fixture.learning_db),
        "workspace-1",
    );

    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].graph_id, "g1");
    assert_eq!(count_learning_rows(&fixture.learning_db), 1);
}

#[test]
fn folding_all_pending_tolerates_a_missing_topology_root() {
    let _guard = store_lock();
    let fixture = fixture();
    let paths = TopologyPaths::at_root(fixture.root.join("never-created"));

    let outcomes = fold_pending_blocking(
        &paths,
        "s1",
        "goal",
        &fixture.topology_db,
        Some(&fixture.learning_db),
        "workspace-1",
    );
    assert!(outcomes.is_empty());
}

#[test]
fn the_topology_store_is_a_separate_database_file() {
    // The write lock key is per canonicalized path, so isolation is exactly the
    // question of whether these two resolve to different files.
    let root = std::path::Path::new("/project");
    assert_eq!(
        topology_db_path(root),
        root.join(".archon").join("topology.db")
    );
    assert_ne!(
        archon_cozo::write_lock_path_for_db(topology_db_path(root)),
        archon_cozo::write_lock_path_for_db(root.join(".archon").join("learning-state.db")),
    );
}

#[test]
fn schema_creation_is_idempotent() {
    let _guard = store_lock();
    let fixture = fixture();
    for _ in 0..3 {
        ensure_topology_schema(&fixture.topology_db).unwrap();
    }
}

#[test]
fn derived_metrics_are_pure_and_repeatable() {
    let readout = TraceReadout {
        records: vec![
            TraceRecord::new("t", "g1", TraceKind::AgentSpawned)
                .with_node("a")
                .with_parent("turn"),
            TraceRecord::new("t", "g1", TraceKind::AgentSpawned)
                .with_node("b")
                .with_parent("turn"),
        ],
        malformed_lines: 0,
        truncated_tail: false,
    };

    let origin = || GraphOrigin::Session {
        session_id: "s1".into(),
    };
    let first = derive("g1", origin(), None, &readout, "refactor the coordinator");
    let second = derive("g1", origin(), None, &readout, "refactor the coordinator");

    assert_eq!(first.task_hash, second.task_hash);
    assert_eq!(first.span, second.span);
    assert_eq!(first.node_count, 3);
    assert_eq!(first.max_parallelism_used, 2);
    assert!(first.task_hash.starts_with("refactor:"));
}

#[test]
fn a_terminal_failure_is_sticky_across_a_later_retry() {
    let records = vec![
        TraceRecord::new("t", "g", TraceKind::NodeFinished)
            .with_node("a")
            .with_outcome(false, true),
        TraceRecord::new("t", "g", TraceKind::NodeStarted).with_node("a"),
    ];
    assert_eq!(
        node_outcomes(&records).get("a").map(String::as_str),
        Some("failed")
    );
}

#[test]
fn fan_out_widths_report_only_real_fan_outs() {
    let mut graph = TaskGraph::new(
        "g",
        GraphOrigin::Session {
            session_id: "s".into(),
        },
    );
    graph.nodes.push(TaskNode::new("root", NodeRole::Work));
    for id in ["a", "b", "c"] {
        let mut node = TaskNode::new(id, NodeRole::Work);
        node.depends_on = vec!["root".into()];
        graph.nodes.push(node);
    }
    let mut tail = TaskNode::new("tail", NodeRole::Work);
    tail.depends_on = vec!["a".into()];
    graph.nodes.push(tail);

    // `root` fans out to three; `a` has a single dependent and is not a fan-out.
    assert_eq!(fan_out_widths(&graph), vec![3]);
}

#[test]
fn verifier_independence_counts_only_disjoint_verifiers() {
    let mut graph = TaskGraph::new(
        "g",
        GraphOrigin::Session {
            session_id: "s".into(),
        },
    );
    graph.nodes.push(TaskNode::new("p", NodeRole::Work));
    graph.nodes.push(TaskNode::new("q", NodeRole::Work));

    let mut shared_a = TaskNode::new("v1", NodeRole::Verify);
    shared_a.depends_on = vec!["p".into()];
    let mut shared_b = TaskNode::new("v2", NodeRole::Verify);
    shared_b.depends_on = vec!["p".into()];
    let mut disjoint = TaskNode::new("v3", NodeRole::Verify);
    disjoint.depends_on = vec!["q".into()];
    graph.nodes.extend([shared_a, shared_b, disjoint]);

    assert_eq!(
        verifier_independence(&graph),
        1,
        "two verifiers behind the same producer are one check, not two"
    );
}

/// The headline concurrency invariant: **nothing on the hot path touches a
/// database.**
///
/// Proving an absence needs two independent arguments and this test makes both.
///
/// 1. *Structural.* `archon-topology` declares no `cozo` dependency, so the
///    trace writer cannot reach a database even in principle. That is enforced
///    by the build graph and by `crates/archon-topology/Cargo.toml`, not by
///    this test.
/// 2. *Behavioural, below.* Every guarded Cozo operation in the process is
///    armed to panic, then a full session is driven — tool attempts including
///    subagent spawns and file writes, orchestrator decomposition and lifecycle
///    events, and a workflow run projection. If any of it reached the store,
///    the panic would fail the test.
#[test]
fn a_full_session_performs_no_database_access() {
    let _guard = store_lock();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    // A real, registered store, so a stray call would find a live target rather
    // than failing for the wrong reason.
    let db = open_db(&topology_db_path(root));
    ensure_topology_schema(&db).expect("schema setup happens before the poison is armed");

    let trace = crate::command::topology_trace::begin(root, "g-hot", "s-hot")
        .expect("ambient trace must open");

    archon_cozo::poison_guarded_scripts();
    let session = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        use archon_core::orchestrator::events::{OrchestratorEvent, Subtask, SubtaskStatus};

        // Tap 1: tool attempts, including a spawn and a write.
        for (tool, input) in [
            ("Read", serde_json::json!({"file_path": "src/lib.rs"})),
            ("Bash", serde_json::json!({"command": "cargo test"})),
            (
                "Write",
                serde_json::json!({"file_path": "src/new.rs", "content": "fn main() {}"}),
            ),
            (
                "Agent",
                serde_json::json!({"subagent_type": "Explore", "prompt": "look"}),
            ),
        ] {
            crate::command::topology_trace::on_tool_run_outcome(
                &archon_tools::tool::ToolRunAttemptOutcome {
                    session_id: "s-hot".into(),
                    parent_action_id: "parent".into(),
                    tool_use_id: format!("tu-{tool}"),
                    attempt: 0,
                    tool_name: tool.into(),
                    input,
                    permission_level: archon_tools::tool::PermissionLevel::Safe,
                    blocked: false,
                    is_error: false,
                    admission_evaluated: false,
                },
            );
        }

        // Tap 2: orchestrator events, decomposition included — that path
        // lowers a subtask list and persists graph.json.
        crate::command::topology_trace::on_orchestrator_event(&OrchestratorEvent::TaskDecomposed {
            subtasks: vec![Subtask {
                id: "a".into(),
                description: "work".into(),
                agent_type: "worker".into(),
                dependencies: vec![],
                status: SubtaskStatus::Pending,
                retries: 0,
                max_retries: 2,
            }],
        });
        for event in [
            OrchestratorEvent::AgentSpawned {
                agent_id: "agent-1".into(),
                agent_type: "worker".into(),
                subtask_id: "a".into(),
            },
            OrchestratorEvent::AgentComplete {
                agent_id: "agent-1".into(),
                subtask_id: "a".into(),
                result: "ok".into(),
            },
            OrchestratorEvent::TeamComplete {
                result: "done".into(),
            },
        ] {
            crate::command::topology_trace::on_orchestrator_event(&event);
        }

        // Tap 3: a workflow run projection.
        let store = archon_workflow::WorkflowStore::project(root);
        std::fs::create_dir_all(store.run_dir("wf-hot")).unwrap();
        let event = archon_workflow::WorkflowEvent {
            seq: 1,
            run_id: "wf-hot".into(),
            ts: chrono::Utc::now(),
            kind: archon_workflow::WorkflowEventKind::StageStarted,
            detail: serde_json::json!({"stage": "plan"}),
        };
        store
            .append_event_line("wf-hot", &serde_json::to_string(&event).unwrap())
            .unwrap();
        crate::command::topology_trace::project_workflow_run(root, &store, "wf-hot");
    }));
    archon_cozo::clear_guarded_script_poison();
    crate::command::topology_trace::end();

    if let Err(panic) = session {
        std::panic::resume_unwind(panic);
    }

    // And the session really did record something, so the test is not vacuous.
    let readout = archon_topology::trace::read_trace(&trace.paths().trace_jsonl("g-hot")).unwrap();
    assert!(
        readout.records.len() >= 8,
        "the session recorded too little to prove anything: {}",
        readout.records.len()
    );
    assert_eq!(readout.malformed_lines, 0);

    // The fold, by contrast, *must* reach the store — otherwise the previous
    // assertion would pass for a recorder that does nothing at all.
    let paths = TopologyPaths::for_project(root);
    let outcome = fold_graph(&paths, "g-hot", "s-hot", "goal", &db, None, "workspace-1").unwrap();
    assert!(outcome.nodes_written > 0);
}

#[test]
fn the_poison_actually_fires_on_a_guarded_write() {
    // Guards the guard: if `poison_guarded_scripts` silently did nothing, the
    // test above would pass for the wrong reason.
    let _guard = store_lock();
    let temp = tempfile::tempdir().unwrap();
    let db = open_db(&temp.path().join("poisoned.db"));

    archon_cozo::poison_guarded_scripts();
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = ensure_topology_schema(&db);
    }))
    .is_err();
    archon_cozo::clear_guarded_script_poison();

    assert!(panicked, "the poison must make a guarded write panic");
}
