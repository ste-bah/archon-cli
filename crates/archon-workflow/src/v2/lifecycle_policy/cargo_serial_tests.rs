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
            "item": { "commands": ["ls -la crates/archon-trading/src"] }
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
        "expected_evidence": ["cargo fmt --check passes"]
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
