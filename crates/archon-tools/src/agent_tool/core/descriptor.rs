//! What the model sees before it calls `Agent`: the tool's prose description
//! and its JSON input schema.
//!
//! Child module of `core` (not a sibling of it) so it can construct
//! [`AgentTool`] directly — the `description` field is private to `core`, and
//! only a descendant can see it.
//!
//! The parent module owns the other half: what happens once the model does
//! call `Agent`.

use serde_json::json;

use super::AgentTool;

const INLINE_AGENT_LIMIT: usize = 20;
pub(crate) const AGENT_DESCRIPTION_LIMIT_BYTES: usize = 4096;

impl AgentTool {
    /// Create an AgentTool with default description (no agent listing).
    pub fn new() -> Self {
        Self {
            description:
                "Spawn a subagent to handle a complex task autonomously. Returns a SubagentRequest \
                for the agent loop to execute. The subagent runs with its own conversation and \
                tool set. Use normal isolation for read-only work; only request worktree isolation \
                when the subagent needs isolated file edits."
                    .into(),
        }
    }

    /// Create an AgentTool with an injected agent listing.
    /// The listing is appended to the description so the LLM knows valid subagent_type values.
    pub fn with_agent_listing(agents: &[(String, String)]) -> Self {
        let mut desc =
            "Spawn a subagent to handle a complex task autonomously. Returns a SubagentRequest \
            for the agent loop to execute. The subagent runs with its own conversation and \
            tool set. Use known subagent_type names directly. Use AgentCatalog to list, search, \
            or inspect less-common agents before launching them. Use normal isolation for read-only \
            work; only request worktree isolation when the subagent needs isolated file edits."
                .to_string();

        if !agents.is_empty() {
            desc.push_str("\n\nCommon agents: ");
            let entries: Vec<String> = agents
                .iter()
                .take(INLINE_AGENT_LIMIT)
                .map(|(name, summary)| {
                    if summary.is_empty() {
                        name.clone()
                    } else {
                        format!("{name} ({summary})")
                    }
                })
                .collect();
            desc.push_str(&entries.join(", "));
        }

        if desc.len() > AGENT_DESCRIPTION_LIMIT_BYTES {
            desc.truncate(AGENT_DESCRIPTION_LIMIT_BYTES);
        }

        Self { description: desc }
    }
}

impl Default for AgentTool {
    fn default() -> Self {
        Self::new()
    }
}

/// The `Agent` tool's JSON input schema.
///
/// `pub(super)` rather than private: `Tool::input_schema` is implemented in
/// the parent module and delegates here.
pub(super) fn input_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "required": ["prompt"],
        "properties": {
            "prompt": {
                "type": "string",
                "description": "The task prompt for the subagent"
            },
            "model": {
                "type": "string",
                "description": "Optional model override. Omit this unless the user explicitly asks for a different model; omitted or empty inherits the parent model/provider. Do not invent provider model IDs."
            },
            "allowed_tools": {
                "type": "array",
                "items": { "type": "string" },
                "description": "List of tool names the subagent is allowed to use"
            },
            "subagent_type": {
                "type": "string",
                "description": "Optional agent type name. When set, loads the agent's custom prompt and tool filters."
            },
            "run_in_background": {
                "type": "boolean",
                "description": "When true, runs the subagent as a background task."
            },
            "cwd": {
                "type": "string",
                "description": "Working directory override for the subagent."
            },
            "isolation": {
                "type": "string",
                "enum": ["none", "worktree"],
                "description": "Optional isolation mode. Use 'none' or omit this field for normal/read-only subagents. Use 'worktree' only when isolated file edits are required."
            },
            "expected_target_files": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Optional file paths that must be changed by a foreground mutating subagent. Archon snapshots these paths before launch and fails the Agent result if they are unchanged after completion."
            },
            "intended_writes": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Optional paths or globs this subagent intends to write. Declaring them lets Archon warn at spawn time when another running agent has already declared overlapping writes, before either has edited anything. Advisory: it does not restrict what the agent may write. Distinct from expected_target_files, which asserts what MUST have changed by the end."
            },
            "task_id": {
                "type": "string",
                "description": "Optional id of an existing task this subagent is taking on. Sets the task's owner and marks it running, so TaskList shows who is working on what."
            }
        }
    })
}
