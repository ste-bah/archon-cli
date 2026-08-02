//! Tap one: a tool attempt becomes trace records.
//!
//! Owns the projection of [`ToolRunAttemptOutcome`] — always an `tool_attempt`,
//! plus an `agent_spawned` when the tool launches a subagent and a
//! `file_written` when it names a file it wrote — and the naming and
//! classification that projection needs.

use archon_tools::tool::{PermissionLevel, ToolRunAttemptOutcome};
use archon_topology::ir::PermissionClass;
use archon_topology::reconstruct::ROOT_NODE_ID;
use archon_topology::trace::{TraceKind, TraceRecord};

use super::payload::{subagent_type, written_paths};
use super::{AmbientTrace, now};

/// Tools whose invocation launches a subagent. Seeing one of these in the tool
/// tap is how a spawn becomes a node without any new plumbing.
const SUBAGENT_TOOLS: &[&str] = &["Agent", "Task", "TaskCreate"];

impl AmbientTrace {
    /// Project a tool attempt outcome into trace records.
    ///
    /// Emits a `tool_attempt` always, plus an `agent_spawned` when the tool
    /// launches a subagent and a `file_written` when it names a file it wrote.
    /// Tool *input* is never recorded verbatim — only the extracted paths and
    /// the tool name — so the trace cannot become a secret sink.
    pub(crate) fn record_tool_outcome(&self, outcome: &ToolRunAttemptOutcome) {
        let ts = now();
        let node_id = ROOT_NODE_ID;

        let mut attempt = TraceRecord::new(&ts, &self.graph_id, TraceKind::ToolAttempt)
            .with_node(node_id)
            .with_tool(&outcome.tool_name)
            .with_permission(permission_class(outcome.permission_level))
            .with_outcome(outcome.blocked, outcome.is_error)
            .with_attempt(outcome.attempt);
        let writes = written_paths(&outcome.tool_name, &outcome.input);
        if !writes.is_empty() {
            attempt = attempt.with_writes(writes.clone());
        }
        self.record(attempt);

        if SUBAGENT_TOOLS.contains(&outcome.tool_name.as_str()) && !outcome.blocked {
            // The tool-use id is the only per-invocation identifier available
            // here, so it names the spawned node. It is stable within a turn
            // and meaningless across turns, which is exactly the lifetime a
            // node id needs.
            let child = spawn_node_id(&outcome.tool_use_id, outcome.attempt);
            let mut spawned = TraceRecord::new(&ts, &self.graph_id, TraceKind::AgentSpawned)
                .with_node(child)
                .with_parent(node_id)
                .with_outcome(outcome.blocked, outcome.is_error);
            if let Some(agent) = subagent_type(&outcome.input) {
                spawned = spawned.with_agent(agent);
            }
            self.record(spawned);
        }

        if !writes.is_empty() {
            self.record(
                TraceRecord::new(&ts, &self.graph_id, TraceKind::FileWritten)
                    .with_node(node_id)
                    .with_writes(writes)
                    .with_outcome(outcome.blocked, outcome.is_error),
            );
        }
    }
}

/// Node id for a subagent spawned by a tool call.
fn spawn_node_id(tool_use_id: &str, attempt: u32) -> String {
    if tool_use_id.is_empty() {
        format!("spawn-{attempt}")
    } else {
        format!("spawn-{tool_use_id}-{attempt}")
    }
}

/// The `PermissionLevel` a tool declared, mapped onto the IR's class.
///
/// `Dangerous` maps to `Irreversible` rather than `Risky`: milestone 3 gates on
/// irreversibility, and under-classifying there is the failure that matters.
pub(super) fn permission_class(level: PermissionLevel) -> PermissionClass {
    match level {
        PermissionLevel::Safe => PermissionClass::Safe,
        PermissionLevel::Risky => PermissionClass::Risky,
        PermissionLevel::Dangerous => PermissionClass::Irreversible,
    }
}
