//! The headline concurrency invariant: admission touches no database.

use super::super::*;
use super::*;

/// Admission must not reach a database, not even for a read.
///
/// Proved two ways, as milestone 2's equivalent test is.
///
/// 1. *Structural.* `archon-topology` declares no `cozo` dependency, so nothing
///    reachable from [`admit`] can open a store even in principle. That is
///    enforced by `crates/archon-topology/Cargo.toml`, not by this test.
/// 2. *Behavioural, below.* Every guarded Cozo operation on this thread is
///    armed to panic and then a full session is driven through admission —
///    installation, a declared graph, spawns, writes, an irreversible call, a
///    block, the outcome release, and teardown.
///
/// The poison is **thread-local and one-shot**, and both properties are
/// load-bearing. Process-global would abort every other test sharing the binary.
/// One-shot matters because the process panic hook (`src/panic_save.rs`)
/// persists session state, which is itself a guarded operation — a poison left
/// armed would re-enter the check from inside the hook, and a panic during
/// panic handling aborts the whole test process instead of failing one test.
#[test]
fn a_full_admission_pass_performs_no_database_access() {
    let _guard = store_lock();
    let temp = tempfile::tempdir().unwrap();

    // A real, registered store, so a stray call would find a live target rather
    // than failing for the wrong reason.
    let db_path = temp.path().join("admission.db");
    let path = db_path.to_string_lossy().to_string();
    let _db = archon_cozo::open_sqlite_guarded_instance(
        &path,
        "open admission test store",
        archon_cozo::CozoGuardConfig::for_db_path(&path),
    )
    .expect("store opens before the poison is armed");

    let config = config_with(archon_core::config::TopologyConfig {
        ungated_irreversible: GateEnforcementConfig::Always,
        ..Default::default()
    });

    uninstall();
    archon_cozo::poison_guarded_scripts();
    let session = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        install(&config, SESSION);
        begin_session("second-session");

        declare_graph(
            SESSION,
            &archon_topology::ir::TaskGraph {
                nodes: vec![archon_topology::ir::TaskNode::new(
                    "turn",
                    archon_topology::ir::NodeRole::Work,
                )],
                budget: archon_topology::ir::GraphBudget {
                    max_agents: 1,
                    ..Default::default()
                },
                ..archon_topology::ir::TaskGraph::new(
                    "g",
                    archon_topology::ir::GraphOrigin::Session {
                        session_id: SESSION.into(),
                    },
                )
            },
        );
        on_node_started(SESSION, "turn");
        on_gate_passed(SESSION, "ask");

        let mut blocked = 0usize;
        for request in [
            request(
                "Read",
                PermissionLevel::Risky,
                serde_json::json!({"file_path": "src/lib.rs"}),
            ),
            write_request("tu-w1", "src/lib.rs"),
            write_request("tu-w2", "src/main.rs"),
            request(
                "Agent",
                PermissionLevel::Risky,
                serde_json::json!({"subagent_type": "Explore", "prompt": "look"}),
            ),
            // Over the budget of one: this one blocks.
            ToolRunAdmissionRequest {
                tool_use_id: "tu-a2".into(),
                ..request(
                    "Agent",
                    PermissionLevel::Risky,
                    serde_json::json!({"subagent_type": "Explore", "prompt": "again"}),
                )
            },
            request(
                "Bash",
                PermissionLevel::Dangerous,
                serde_json::json!({"command": "git push"}),
            ),
        ] {
            if matches!(admit(&request), ToolRunAdmission::Blocked { .. }) {
                blocked += 1;
            }
            on_tool_run_outcome(&ToolRunAttemptOutcome {
                session_id: request.session_id.clone(),
                parent_action_id: request.parent_action_id.clone(),
                tool_use_id: request.tool_use_id.clone(),
                attempt: request.attempt,
                tool_name: request.tool_name.clone(),
                input: request.input.clone(),
                permission_level: request.permission_level,
                blocked: false,
                is_error: false,
                admission_evaluated: true,
            });
        }

        on_node_finished(SESSION, "turn");
        end_session(SESSION);
        end_session("second-session");
        blocked
    }));
    archon_cozo::clear_guarded_script_poison();
    uninstall();

    let blocked = match session {
        Ok(blocked) => blocked,
        Err(panic) => std::panic::resume_unwind(panic),
    };

    // And the pass really did decide something, so the test is not vacuous.
    assert!(
        blocked >= 1,
        "the session admitted everything, so it proves nothing"
    );
}

#[test]
fn the_poison_actually_fires_on_a_guarded_write() {
    // Guards the guard: if `poison_guarded_scripts` silently did nothing, the
    // test above would pass for the wrong reason.
    let _guard = store_lock();
    let temp = tempfile::tempdir().unwrap();
    let path = temp
        .path()
        .join("poisoned.db")
        .to_string_lossy()
        .to_string();
    let db = archon_cozo::open_sqlite_guarded_instance(
        &path,
        "open poison test store",
        archon_cozo::CozoGuardConfig::for_db_path(&path),
    )
    .expect("store opens");

    archon_cozo::poison_guarded_scripts();
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = archon_cozo::run_bound_script_guarded(
            &db,
            ":create topology_poison_probe { id: String }",
            std::collections::BTreeMap::new(),
            cozo::ScriptMutability::Mutable,
            "poison probe",
        );
    }))
    .is_err();
    archon_cozo::clear_guarded_script_poison();

    assert!(panicked, "the poison must make a guarded write panic");
}
