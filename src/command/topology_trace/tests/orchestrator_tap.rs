use super::*;

#[test]
fn task_decomposition_declares_a_graph() {
    let temp = tempfile::tempdir().unwrap();
    let trace = trace_in(temp.path());

    let subtasks = vec![
        Subtask {
            id: "a".into(),
            description: "first".into(),
            agent_type: "worker".into(),
            dependencies: vec![],
            status: SubtaskStatus::Pending,
            retries: 0,
            max_retries: 2,
        },
        Subtask {
            id: "b".into(),
            description: "second".into(),
            agent_type: "worker".into(),
            dependencies: vec!["a".into()],
            status: SubtaskStatus::Pending,
            retries: 0,
            max_retries: 2,
        },
    ];

    trace.record_orchestrator_event(&OrchestratorEvent::TaskDecomposed { subtasks });

    let graph = trace
        .paths()
        .read_graph("g1")
        .unwrap()
        .expect("decomposition must persist a graph");
    assert_eq!(
        graph.id, "g1",
        "the graph id must be the trace's, not the lowering's"
    );
    assert!(matches!(graph.origin, GraphOrigin::Team { .. }));
    assert_eq!(graph.len(), 2);
    assert_eq!(graph.node("b").unwrap().depends_on, vec!["a"]);
}

#[test]
fn orchestrator_lifecycle_events_project_to_node_records() {
    let temp = tempfile::tempdir().unwrap();
    let trace = trace_in(temp.path());

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
        OrchestratorEvent::AgentFailed {
            agent_id: "agent-2".into(),
            subtask_id: "b".into(),
            error: "boom".into(),
            will_retry: true,
        },
        OrchestratorEvent::AgentFailed {
            agent_id: "agent-2".into(),
            subtask_id: "b".into(),
            error: "boom".into(),
            will_retry: false,
        },
        OrchestratorEvent::TeamCancelled,
    ] {
        trace.record_orchestrator_event(&event);
    }

    let readout = read_trace(&trace.paths().trace_jsonl("g1")).unwrap();
    let kinds: Vec<TraceKind> = readout.records.iter().map(|record| record.kind).collect();
    assert_eq!(
        kinds,
        vec![
            TraceKind::AgentSpawned,
            TraceKind::NodeStarted,
            TraceKind::NodeFinished,
            TraceKind::Retry,
            TraceKind::NodeFinished,
            TraceKind::NodeFinished,
        ]
    );
    assert!(
        readout.records.last().unwrap().error,
        "cancellation is a failure"
    );
}

#[test]
fn never_emitted_orchestrator_variants_record_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let trace = trace_in(temp.path());

    trace.record_orchestrator_event(&OrchestratorEvent::AgentProgress {
        agent_id: "a".into(),
        message: "working".into(),
    });
    trace.record_orchestrator_event(&OrchestratorEvent::TeamFailed { error: "x".into() });

    assert!(
        read_trace(&trace.paths().trace_jsonl("g1"))
            .unwrap()
            .is_empty()
    );
}
