use super::tool_tap::permission_class;
use super::workflow_tap::workflow_trace_record;
use super::*;

use archon_core::orchestrator::events::{Subtask, SubtaskStatus};
use archon_tools::tool::PermissionLevel;
use archon_topology::ir::{GraphOrigin, PermissionClass, WriteTarget};
use archon_topology::reconstruct::ROOT_NODE_ID;
use archon_topology::trace::read_trace;

fn outcome(tool: &str, input: serde_json::Value) -> ToolRunAttemptOutcome {
    ToolRunAttemptOutcome {
        session_id: "s1".into(),
        parent_action_id: "parent-1".into(),
        tool_use_id: "tu-1".into(),
        attempt: 0,
        tool_name: tool.into(),
        input,
        permission_level: PermissionLevel::Safe,
        blocked: false,
        is_error: false,
        admission_evaluated: false,
    }
}

fn trace_in(dir: &std::path::Path) -> AmbientTrace {
    AmbientTrace::open(dir, "g1", "s1").unwrap()
}

#[test]
fn a_tool_attempt_records_one_line() {
    let temp = tempfile::tempdir().unwrap();
    let trace = trace_in(temp.path());

    trace.record_tool_outcome(&outcome("Bash", serde_json::json!({"command": "ls"})));

    let readout = read_trace(&trace.paths().trace_jsonl("g1")).unwrap();
    assert_eq!(readout.records.len(), 1);
    assert_eq!(readout.records[0].kind, TraceKind::ToolAttempt);
    assert_eq!(readout.records[0].tool.as_deref(), Some("Bash"));
    assert_eq!(readout.records[0].node_id, ROOT_NODE_ID);
}

#[test]
fn tool_input_is_never_recorded_verbatim() {
    let temp = tempfile::tempdir().unwrap();
    let trace = trace_in(temp.path());

    trace.record_tool_outcome(&outcome(
        "Bash",
        serde_json::json!({"command": "curl -H 'Authorization: Bearer sk-secret-value'"}),
    ));

    let raw = std::fs::read_to_string(trace.paths().trace_jsonl("g1")).unwrap();
    assert!(
        !raw.contains("sk-secret-value"),
        "the trace must not become a secret sink: {raw}"
    );
}

#[test]
fn a_subagent_spawn_becomes_a_node_with_an_edge_to_the_turn_root() {
    let temp = tempfile::tempdir().unwrap();
    let trace = trace_in(temp.path());

    trace.record_tool_outcome(&outcome(
        "Agent",
        serde_json::json!({"subagent_type": "Explore", "prompt": "look"}),
    ));

    let readout = read_trace(&trace.paths().trace_jsonl("g1")).unwrap();
    let spawn = readout
        .records
        .iter()
        .find(|record| record.kind == TraceKind::AgentSpawned)
        .expect("a subagent tool call must record a spawn");
    assert_eq!(spawn.parent_node_id.as_deref(), Some(ROOT_NODE_ID));
    assert_eq!(spawn.agent.as_deref(), Some("Explore"));
    assert!(spawn.node_id.starts_with("spawn-tu-1"));
}

#[test]
fn a_blocked_subagent_call_records_no_spawn() {
    let temp = tempfile::tempdir().unwrap();
    let trace = trace_in(temp.path());

    let mut blocked = outcome("Agent", serde_json::json!({"subagent_type": "Explore"}));
    blocked.blocked = true;
    trace.record_tool_outcome(&blocked);

    let readout = read_trace(&trace.paths().trace_jsonl("g1")).unwrap();
    assert!(
        !readout
            .records
            .iter()
            .any(|record| record.kind == TraceKind::AgentSpawned),
        "a blocked call spawned nothing"
    );
}

#[test]
fn a_write_records_its_target_and_a_read_does_not() {
    let temp = tempfile::tempdir().unwrap();
    let trace = trace_in(temp.path());

    trace.record_tool_outcome(&outcome(
        "Write",
        serde_json::json!({"file_path": "C:\\repo\\src\\a.rs", "content": "x"}),
    ));
    trace.record_tool_outcome(&outcome(
        "Read",
        serde_json::json!({"file_path": "src/b.rs"}),
    ));

    let readout = read_trace(&trace.paths().trace_jsonl("g1")).unwrap();
    let written: Vec<&TraceRecord> = readout
        .records
        .iter()
        .filter(|record| record.kind == TraceKind::FileWritten)
        .collect();
    assert_eq!(written.len(), 1, "only the Write tool writes");
    assert_eq!(
        written[0].writes,
        vec![WriteTarget::Path("C:/repo/src/a.rs".into())],
        "separators must be normalized so exact-match conflict detection works"
    );
}

#[test]
fn permission_levels_map_onto_the_ir_classes() {
    assert_eq!(
        permission_class(PermissionLevel::Safe),
        PermissionClass::Safe
    );
    assert_eq!(
        permission_class(PermissionLevel::Risky),
        PermissionClass::Risky
    );
    assert_eq!(
        permission_class(PermissionLevel::Dangerous),
        PermissionClass::Irreversible,
        "under-classifying irreversibility is the failure that matters"
    );
}

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

#[test]
fn the_ambient_slot_installs_and_clears() {
    // The slot is process-wide, so this must not interleave with any other test
    // that installs into it — notably the fold's no-database-access test.
    let _guard = test_lock();
    let temp = tempfile::tempdir().unwrap();

    end();
    assert!(active().is_none());

    let trace = begin(temp.path(), "g-ambient", "s1").expect("begin must succeed on a temp dir");
    assert!(active().is_some());
    assert_eq!(active().unwrap().graph_id(), "g-ambient");

    // The free-function taps route to the installed trace.
    on_tool_run_outcome(&outcome("Bash", serde_json::json!({})));
    assert_eq!(
        read_trace(&trace.paths().trace_jsonl("g-ambient"))
            .unwrap()
            .records
            .len(),
        1
    );

    end();
    assert!(active().is_none());
    // With nothing installed the taps are no-ops rather than panics.
    on_tool_run_outcome(&outcome("Bash", serde_json::json!({})));
    on_orchestrator_event(&OrchestratorEvent::TeamCancelled);
    assert_eq!(
        read_trace(&trace.paths().trace_jsonl("g-ambient"))
            .unwrap()
            .records
            .len(),
        1,
        "a cleared slot must record nothing"
    );
}
