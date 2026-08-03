use super::*;

#[test]
fn workflow_events_project_by_kind_and_skip_the_rest() {
    use archon_workflow::{WorkflowEvent, WorkflowEventKind};

    let event = |kind, detail| WorkflowEvent {
        seq: 1,
        run_id: "wf-1".into(),
        ts: chrono::Utc::now(),
        kind,
        detail,
    };

    assert_eq!(
        workflow_trace_record(
            "g1",
            &event(
                WorkflowEventKind::StageStarted,
                serde_json::json!({"stage": "build"})
            )
        )
        .map(|record| (record.kind, record.node_id)),
        Some((TraceKind::NodeStarted, "build".to_string()))
    );
    assert_eq!(
        workflow_trace_record(
            "g1",
            &event(
                WorkflowEventKind::StageCompleted,
                serde_json::json!({"stage_id": "test"})
            )
        )
        .map(|record| (record.kind, record.node_id)),
        Some((TraceKind::NodeFinished, "test".to_string()))
    );
    assert_eq!(
        workflow_trace_record(
            "g1",
            &event(
                WorkflowEventKind::StageFailed,
                serde_json::json!({"stage": "test"})
            )
        )
        .map(|record| (record.kind, record.error)),
        Some((TraceKind::Retry, true))
    );
    assert_eq!(
        workflow_trace_record(
            "g1",
            &event(
                WorkflowEventKind::ForcedAccepted,
                serde_json::json!({"stage": "gate"})
            )
        )
        .map(|record| record.kind),
        Some(TraceKind::GatePassed)
    );
    // Unmapped kinds are skipped rather than guessed at.
    assert!(
        workflow_trace_record(
            "g1",
            &event(
                WorkflowEventKind::WriteCoordinationWaveScheduled,
                serde_json::json!({"stage": "x"})
            )
        )
        .is_none()
    );
    // No stage identifier anywhere: attribute to the turn root.
    assert_eq!(
        workflow_trace_record(
            "g1",
            &event(WorkflowEventKind::StageStarted, serde_json::json!({}))
        )
        .map(|record| record.node_id),
        Some(ROOT_NODE_ID.to_string())
    );
}

#[test]
fn a_workflow_run_projects_into_a_declared_graph_with_a_workflow_origin() {
    use archon_workflow::{WorkflowEvent, WorkflowEventKind, WorkflowStore};

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    std::fs::create_dir_all(store.run_dir("wf-1")).unwrap();

    for (seq, kind, stage) in [
        (1u64, WorkflowEventKind::StageStarted, "plan"),
        (2, WorkflowEventKind::StageCompleted, "plan"),
        (3, WorkflowEventKind::StageStarted, "build"),
        (4, WorkflowEventKind::StageCompleted, "build"),
    ] {
        let event = WorkflowEvent {
            seq,
            run_id: "wf-1".into(),
            ts: chrono::Utc::now(),
            kind,
            detail: serde_json::json!({"stage": stage}),
        };
        store
            .append_event_line("wf-1", &serde_json::to_string(&event).unwrap())
            .unwrap();
    }

    let projected = project_workflow_run(temp.path(), &store, "wf-1");
    assert_eq!(projected, 4);

    let paths = TopologyPaths::for_project(temp.path());
    let graph = paths
        .read_graph("wf-1")
        .unwrap()
        .expect("a projected run must declare a graph");
    assert!(
        matches!(graph.origin, GraphOrigin::Workflow { ref run_id } if run_id == "wf-1"),
        "a workflow run must not be relabelled a session: {:?}",
        graph.origin
    );
    assert_eq!(graph.len(), 2, "two stages");

    let readout = read_trace(&paths.trace_jsonl("wf-1")).unwrap();
    assert_eq!(
        readout
            .records
            .iter()
            .filter(|record| record.kind != TraceKind::GraphDeclared)
            .count(),
        4,
        "four projected stage records"
    );
    assert_eq!(
        readout
            .records
            .iter()
            .filter(|record| record.kind == TraceKind::GraphDeclared)
            .count(),
        1,
        "declaring the graph also leaves a marker record"
    );
}

#[test]
fn projecting_an_absent_workflow_run_is_a_no_op() {
    let temp = tempfile::tempdir().unwrap();
    let store = archon_workflow::WorkflowStore::project(temp.path());
    assert_eq!(project_workflow_run(temp.path(), &store, "wf-missing"), 0);
}

#[test]
fn a_partial_trailing_workflow_line_is_ignored() {
    use archon_workflow::{WorkflowEvent, WorkflowEventKind, WorkflowStore};

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    std::fs::create_dir_all(store.run_dir("wf-1")).unwrap();

    let event = WorkflowEvent {
        seq: 1,
        run_id: "wf-1".into(),
        ts: chrono::Utc::now(),
        kind: WorkflowEventKind::StageStarted,
        detail: serde_json::json!({"stage": "plan"}),
    };
    store
        .append_event_line("wf-1", &serde_json::to_string(&event).unwrap())
        .unwrap();
    // `WorkflowStore::append_event_line` writes body and newline separately, so
    // this fragment is exactly what a concurrent reader can catch there.
    {
        use std::io::Write as _;
        std::fs::OpenOptions::new()
            .append(true)
            .open(store.events_path("wf-1"))
            .unwrap()
            .write_all(br#"{"seq":2,"run_id":"wf-1"#)
            .unwrap();
    }

    assert_eq!(project_workflow_run(temp.path(), &store, "wf-1"), 1);
}
