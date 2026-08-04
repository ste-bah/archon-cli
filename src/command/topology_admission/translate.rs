//! `ToolRunAdmissionRequest` → [`ToolIntent`].
//!
//! `archon-topology` cannot name `archon-tools` — the dependency budget is
//! petgraph + serde + archon-workflow, and `archon-tools` brings tokio, the
//! whole tool registry, and a network stack. The design sketch wrote
//! `on_tool(&self, req: &ToolRunAdmissionRequest)` as though it could. This is
//! the translation instead, and it is the only place in the wiring that knows
//! about either vocabulary.
//!
//! Key names guessed at rather than declared are shared with the milestone 2
//! trace tap (`topology_trace::payload`) so the two agree about what a tool
//! writes and what a spawn names; disagreeing would mean a write recorded in
//! the trace but not claimed at admission.

use archon_core::orchestrator::topology::permission_class_for_level;
use archon_tools::tool::ToolRunAdmissionRequest;
use archon_topology::ir::WriteTarget;
use archon_topology::live::{SpawnIntent, ToolIntent};

/// Tools whose invocation launches a subagent.
///
/// Same list as the milestone 2 trace tap. This is the seam through which
/// "wire subagent spawn" is satisfied: `AgentTool` and `TaskCreate` both
/// declare `PermissionLevel::Risky`, so every spawn already passes through the
/// admission callback and no new plumbing is needed.
const SUBAGENT_TOOLS: &[&str] = &["Agent", "Task", "TaskCreate"];

/// The node id a tool call with no per-node attribution belongs to.
///
/// `ToolContext::tool_run_parent_action_id` is copied verbatim into every
/// subagent context, so a call made inside a spawned agent is indistinguishable
/// from one made by the top-level agent. Everything therefore attributes to the
/// turn root except a spawn, which names itself.
pub(super) fn turn_root() -> &'static str {
    archon_topology::reconstruct::ROOT_NODE_ID
}

/// Whether `tool_name` launches a subagent.
pub(super) fn is_spawn(tool_name: &str) -> bool {
    SUBAGENT_TOOLS.contains(&tool_name)
}

/// Node id for a subagent spawned by a tool call.
///
/// The tool-use id is the only per-invocation identifier available: stable
/// within a turn, meaningless across turns, which is exactly the lifetime a
/// node id needs. Shared with `topology_trace::tool_tap` so the trace and
/// admission name the same node.
pub(super) fn node_id(tool_use_id: &str, attempt: u32) -> String {
    if tool_use_id.is_empty() {
        format!("spawn-{attempt}")
    } else {
        format!("spawn-{tool_use_id}-{attempt}")
    }
}

/// Project an admission request into a topology intent.
pub(super) fn tool_intent(request: &ToolRunAdmissionRequest) -> ToolIntent {
    let intent = ToolIntent::new(
        turn_root(),
        &request.tool_name,
        permission_class_for_level(request.permission_level),
    )
    .with_writes(declared_writes(&request.tool_name, &request.input));

    if is_spawn(&request.tool_name) {
        let child = node_id(&request.tool_use_id, request.attempt);
        return intent.with_spawn(SpawnIntent {
            node_id: child,
            parent_id: Some(turn_root().to_string()),
            agent: agent_type(&request.input).unwrap_or_else(|| request.tool_name.clone()),
        });
    }
    intent
}

/// Paths a tool call declares it writes.
///
/// Reuses the milestone 2 extractor verbatim, so a path recorded in the trace
/// as written is the same path claimed at admission. Restricting extraction to
/// tools known to write matters here as much as there: a `Read` also carries
/// `file_path`, and treating that as a write would manufacture single-writer
/// conflicts out of nothing.
pub(super) fn declared_writes(tool_name: &str, input: &serde_json::Value) -> Vec<String> {
    crate::command::topology_trace::written_paths(tool_name, input)
        .into_iter()
        .filter_map(|target| match target {
            WriteTarget::Path(path) => Some(path),
            // An artifact key is not a filesystem resource and the write
            // coordinator's overlap table is about paths. Nothing produces one
            // from a tool input today.
            WriteTarget::Artifact(_) => None,
        })
        .collect()
}

/// The agent type a spawning call named, if any.
fn agent_type(input: &serde_json::Value) -> Option<String> {
    crate::command::topology_trace::subagent_type(input)
}
