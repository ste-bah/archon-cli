//! `ToolRunAdmissionRequest` translation, and the block reasons the model reads.

use super::super::*;
use super::*;

#[test]
fn a_blocked_write_names_the_holder_the_path_and_the_invariant() {
    let _guard = store_lock();
    let config = config_with(archon_core::config::TopologyConfig::default());

    with_tracker(&config, || {
        on_node_started(SESSION, "holder");
        active().expect("installed").on_write_intent(
            SESSION,
            &archon_topology::live::WriteIntent {
                node_id: "holder".into(),
                paths: vec!["src/lib.rs".into()],
                shared_append: Vec::new(),
            },
        );

        let ToolRunAdmission::Blocked { reason } = admit(&write_request("tu-1", "src/lib.rs"))
        else {
            panic!("a claimed path must block");
        };

        assert!(reason.contains("holder"), "names the holder: {reason}");
        assert!(reason.contains("src/lib.rs"), "names the path: {reason}");
        assert!(
            reason.contains("single_writer"),
            "names the invariant: {reason}"
        );
    });
}

#[test]
fn a_read_is_not_treated_as_a_write() {
    // `Read` also carries `file_path`. Treating that as a write would
    // manufacture single-writer conflicts out of nothing.
    let _guard = store_lock();
    let config = config_with(archon_core::config::TopologyConfig::default());

    with_tracker(&config, || {
        on_node_started(SESSION, "holder");
        active().expect("installed").on_write_intent(
            SESSION,
            &archon_topology::live::WriteIntent {
                node_id: "holder".into(),
                paths: vec!["src/lib.rs".into()],
                shared_append: Vec::new(),
            },
        );

        let read = request(
            "Read",
            PermissionLevel::Risky,
            serde_json::json!({"file_path": "src/lib.rs"}),
        );
        assert_eq!(admit(&read), ToolRunAdmission::Allowed);
    });
}

#[test]
fn a_spawning_tool_call_is_admitted_against_the_agent_cap() {
    let _guard = store_lock();
    let config = config_with(archon_core::config::TopologyConfig {
        max_agents: 1,
        ..Default::default()
    });

    with_tracker(&config, || {
        // No declared graph: the ceiling comes from `[topology] max_agents`,
        // which is the whole point of that key existing.
        let spawn = |id: &str| ToolRunAdmissionRequest {
            tool_use_id: id.into(),
            ..request(
                "Agent",
                PermissionLevel::Risky,
                serde_json::json!({"subagent_type": "Explore", "prompt": "look"}),
            )
        };

        assert_eq!(admit(&spawn("tu-1")), ToolRunAdmission::Allowed);

        let ToolRunAdmission::Blocked { reason } = admit(&spawn("tu-2")) else {
            panic!("the second spawn exceeds a budget of one");
        };
        assert!(reason.contains("agent_cap"), "{reason}");
    });
}

#[test]
fn a_dangerous_tool_lowers_to_irreversible() {
    let _guard = store_lock();
    let config = config_with(archon_core::config::TopologyConfig {
        // The literal reading, so an undeclared session enforces and the
        // classification is observable.
        ungated_irreversible: GateEnforcementConfig::Always,
        ..Default::default()
    });

    with_tracker(&config, || {
        let ToolRunAdmission::Blocked { reason } = admit(&request(
            "Bash",
            PermissionLevel::Dangerous,
            serde_json::json!({"command": "git push --force"}),
        )) else {
            panic!("a Dangerous tool must lower to Irreversible and block with no gate passed");
        };
        assert!(reason.contains("ungated_irreversible"), "{reason}");

        // Risky is the near-miss and must be admitted.
        assert_eq!(
            admit(&request(
                "Bash",
                PermissionLevel::Risky,
                serde_json::json!({"command": "cargo test"})
            )),
            ToolRunAdmission::Allowed
        );
    });
}

#[test]
fn the_default_config_does_not_block_an_irreversible_call_in_a_plain_session() {
    // The design defect, pinned at the wiring layer: the literal reading would
    // block every `git push` in every ordinary turn.
    let _guard = store_lock();
    let config = config_with(archon_core::config::TopologyConfig::default());

    with_tracker(&config, || {
        assert_eq!(
            admit(&request(
                "Bash",
                PermissionLevel::Dangerous,
                serde_json::json!({"command": "git push"})
            )),
            ToolRunAdmission::Allowed
        );
    });
}

#[test]
fn the_outcome_tap_releases_write_claims() {
    let _guard = store_lock();
    let config = config_with(archon_core::config::TopologyConfig::default());

    with_tracker(&config, || {
        assert_eq!(
            admit(&write_request("tu-1", "src/lib.rs")),
            ToolRunAdmission::Allowed
        );

        on_tool_run_outcome(&ToolRunAttemptOutcome {
            session_id: SESSION.into(),
            parent_action_id: "parent".into(),
            tool_use_id: "tu-1".into(),
            attempt: 0,
            tool_name: "Write".into(),
            input: serde_json::json!({"file_path": "src/lib.rs"}),
            permission_level: PermissionLevel::Risky,
            blocked: false,
            is_error: false,
            admission_evaluated: true,
        });

        let live = active().expect("installed");
        let state = live.snapshot(SESSION).expect("tracked");
        assert!(
            !state.is_live("turn") || state.gates_passed().is_empty(),
            "claims released without disturbing the rest of the prefix"
        );
        // A second, unrelated node can now take the path.
        on_node_started(SESSION, "other");
        assert!(
            live.on_write_intent(
                SESSION,
                &archon_topology::live::WriteIntent {
                    node_id: "other".into(),
                    paths: vec!["src/lib.rs".into()],
                    shared_append: Vec::new(),
                },
            )
            .is_allowed()
        );
    });
}
