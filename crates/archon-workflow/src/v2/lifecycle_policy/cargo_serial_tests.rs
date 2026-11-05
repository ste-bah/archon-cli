use super::*;
use crate::{WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions};

fn fanout_item(input: serde_json::Value) -> WorkflowV2FanoutItem {
    WorkflowV2FanoutItem {
        id: "branch-1".to_string(),
        role: "coder".to_string(),
        call: WorkflowV2HostCall {
            id: "verification-wave-1-branch-1".to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        input,
    }
}

/// The live wave shape: a cargo test branch and a file-inspection branch. Only
/// the cargo branch is serialized; the inspection keeps the wave's width.
#[test]
fn only_cargo_items_are_retagged() {
    let items = vec![
        fanout_item(serde_json::json!({
            "item": { "commands": ["cargo test -p archon-trading validation"] }
        })),
        fanout_item(serde_json::json!({
            "item": { "commands": ["ls -la crates/archon-trading/src"] },
            "task": "Inspect the requested files"
        })),
    ];

    let tagged = tag_cargo_serial_roles(items);

    assert_eq!(tagged[0].role, CARGO_SERIAL_ROLE);
    assert_eq!(tagged[1].role, "coder");
}

/// The tag must be a scheduling identity only: retagging may not rewrite the
/// input, because input hashes drive branch-outcome reuse across resumes.
#[test]
fn retagging_never_touches_the_input() {
    let input = serde_json::json!({
        "item": { "commands": ["cargo clippy --workspace"] }
    });
    let items = vec![fanout_item(input.clone())];

    let tagged = tag_cargo_serial_roles(items);

    assert_eq!(tagged[0].input, input);
}

/// Bare items (no `item` wrapper) still get recognised.
#[test]
fn a_bare_item_layout_is_also_recognised() {
    let items = vec![fanout_item(serde_json::json!({
        "commands": ["cargo fmt --check"]
    }))];

    assert_eq!(tag_cargo_serial_roles(items)[0].role, CARGO_SERIAL_ROLE);
}

/// "cargo " is matched as a command word: prose naming the cargo directory or
/// a path does not serialize a branch.
#[test]
fn a_path_mentioning_cargo_without_a_command_is_not_retagged() {
    let items = vec![fanout_item(serde_json::json!({
        "item": { "commands": ["ls .cargo/config.toml"] }
    }))];

    assert_eq!(tag_cargo_serial_roles(items)[0].role, "coder");
}

#[test]
fn the_role_limit_serializes_exactly_the_cargo_role() {
    let limits = cargo_serial_role_limits();
    assert_eq!(limits.get(CARGO_SERIAL_ROLE), Some(&1));
    assert_eq!(limits.len(), 1);
}

/// The live bypass: told the host owns the shared target directory, agents
/// hand-roll the compiler. `rustc` takes the same lock `cargo` does, matched
/// nothing under the old `contains("cargo ")` test, and ran at full wave width.
#[test]
fn a_hand_rolled_rustc_invocation_is_serialized_like_cargo() {
    for command in [
        "rustc --edition 2024 --crate-type lib src/lib.rs",
        "/usr/local/bin/rustc --extern serde=target/debug/libserde.rmeta x.rs",
        "rustdoc --test README.md",
    ] {
        let item = serde_json::json!({ "focused_verification": [command] });
        assert!(
            super::item_has_cargo_commands(&item),
            "{command} must take the serial role"
        );
    }
}

/// Word matching, not substring: naming the role in prose is not running it.
#[test]
fn prose_naming_the_role_does_not_claim_the_serial_slot() {
    let item = serde_json::json!({
        "expected_evidence": ["the cargo-serial role caps these at one branch"]
    });

    assert!(!super::item_has_cargo_commands(&item));
}

/// Cargo itself must keep matching in every shape it is written.
#[test]
fn cargo_still_matches_in_its_usual_shapes() {
    for command in [
        "cargo test -p archon-workflow",
        "run cargo",
        "cd repo && cargo check",
        "(cargo build)",
    ] {
        let item = serde_json::json!({ "command": command });
        assert!(
            super::item_has_cargo_commands(&item),
            "{command} must take the serial role"
        );
    }
}

#[test]
fn all_build_tools_and_custom_commands_share_the_resource_lane() {
    for command in [
        "npm test",
        "pytest a b",
        "go test ./pkg",
        "./gradlew test",
        "custom-check --all",
    ] {
        let input = serde_json::json!({"command": command});
        assert_eq!(
            tag_cargo_serial_roles(vec![fanout_item(input)])[0].role,
            CARGO_SERIAL_ROLE
        );
    }
    let input = serde_json::json!({"task": "Discuss cargo test", "commands": ["cat README.md"], "expected_evidence": "cargo output"});
    assert_eq!(
        tag_cargo_serial_roles(vec![fanout_item(input)])[0].role,
        "coder"
    );
}

#[test]
fn free_form_agent_tasks_without_declared_commands_take_the_safe_lane() {
    for task in [
        "Run the project checks",
        "Inspect and report",
        "Run the custom builder",
    ] {
        assert!(item_has_cargo_commands(&serde_json::json!({"task":task})));
    }
}
