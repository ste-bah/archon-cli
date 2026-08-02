//! Wiring tests for milestone 3.
//!
//! The invariants themselves are tested where they live
//! (`archon_topology::live::tests`). This suite covers the parts that only
//! exist here: translation from `ToolRunAdmissionRequest`, the composed
//! callback, the config surface, and the headline concurrency invariant — that
//! admission touches no database.

mod absent;
mod hot_path;
mod translate;

use archon_core::config::{ArchonConfig, GateEnforcementConfig};
use archon_tools::tool::{
    PermissionLevel, ToolRunAdmission, ToolRunAdmissionRequest, ToolRunAttemptOutcome,
};

use super::*;

/// The tracker slot and `archon_cozo`'s script poison are both process-global,
/// so every test here serializes on the shared topology test lock — the same
/// one milestone 2's suites use, deliberately one lock rather than two.
use crate::command::topology_trace::test_lock as store_lock;

const SESSION: &str = "s-admit";

fn config_with(topology: archon_core::config::TopologyConfig) -> ArchonConfig {
    ArchonConfig {
        topology,
        ..ArchonConfig::default()
    }
}

fn request(
    tool: &str,
    level: PermissionLevel,
    input: serde_json::Value,
) -> ToolRunAdmissionRequest {
    ToolRunAdmissionRequest {
        session_id: SESSION.into(),
        parent_action_id: "parent".into(),
        tool_use_id: format!("tu-{tool}"),
        attempt: 0,
        tool_name: tool.into(),
        input,
        permission_level: level,
    }
}

fn write_request(tool_use: &str, path: &str) -> ToolRunAdmissionRequest {
    ToolRunAdmissionRequest {
        tool_use_id: tool_use.into(),
        ..request(
            "Write",
            PermissionLevel::Risky,
            serde_json::json!({ "file_path": path, "content": "x" }),
        )
    }
}

/// Install a tracker, run `body`, then always uninstall.
fn with_tracker<T>(config: &ArchonConfig, body: impl FnOnce() -> T) -> T {
    uninstall();
    install(config, SESSION);
    let result = body();
    uninstall();
    result
}
