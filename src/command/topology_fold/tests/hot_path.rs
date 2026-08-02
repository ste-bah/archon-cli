//! The headline concurrency invariant: recording touches no database.
//!
//! One test drives a full session through all three taps with every guarded
//! Cozo operation armed to panic; a second proves the arming works, so the
//! first cannot pass for the wrong reason.

use super::*;

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
