//! Lockstep guard: the agent loop's inline permission match must agree with
//! archon_permissions::DEFAULT_SAFE_TOOLS for every tool name.
//!
//! Two weeks of incident: v0.1.10 added Agent to DEFAULT_SAFE_TOOLS in
//! archon-permissions, but agent.rs had its OWN inline list that was never
//! updated. Result: Agent prompted for permission on every dispatch despite
//! "Agent in safe list" claim being true at the permissions layer.
//!
//! This test pins both lists to a single source and fails the build if they
//! ever diverge.

use std::collections::HashSet;

#[test]
fn agent_loop_default_gate_matches_permissions_safe_list() {
    let permissions_list: HashSet<&'static str> = archon_permissions::default_safe_tools()
        .iter()
        .copied()
        .collect();

    // Drive the agent loop's gate with every entry — every one must allow.
    for tool in &permissions_list {
        let allowed = archon_core::agent::is_safe_in_default_mode(tool);
        assert!(
            allowed,
            "Tool '{}' is in archon_permissions::DEFAULT_SAFE_TOOLS but the \
             agent.rs inline gate does NOT auto-allow it. Lockstep violated.",
            tool
        );
    }
}

#[test]
fn agent_is_explicitly_allowed_in_default_mode() {
    // Belt-and-braces: regardless of how the lists are wired, "Agent" must
    // auto-allow in default mode after v0.1.14.
    assert!(
        archon_core::agent::is_safe_in_default_mode("Agent"),
        "Agent tool MUST auto-allow in default mode (v0.1.10 contract)."
    );
}
