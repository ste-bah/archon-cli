//! Describing a tool surface against the world a session is actually in.
//!
//! `Tool::input_schema` is answered once, when the registry is built, and at
//! that point no sandbox backend exists — `create_default_registry` runs before
//! the session decides where its commands will execute, and `/sandbox on` can
//! install a world later still. A tool whose arguments differ per world
//! therefore cannot describe itself there, and `TerminalCreate` was offering
//! PowerShell and cmd to sessions running inside Linux containers.
//!
//! `Tool::input_schema_for` is where such a tool answers instead, and this is
//! the pass that asks it.

use std::sync::Arc;

use archon_tools::tool::{Tool, ToolContext};

use super::ToolRegistry;

impl ToolRegistry {
    /// Every registered tool, ordered by name.
    ///
    /// For a consumer that has to describe tools later, against a context this
    /// registry does not have yet. Rendered definitions cannot do that, which
    /// is what made `ToolSearchTool` a second copy of the surface that
    /// contradicted the live one once `TerminalCreate` learned to vary.
    pub fn tool_handles(&self) -> Vec<Arc<dyn Tool>> {
        let mut tools: Vec<_> = self.tools.iter().collect();
        tools.sort_by_key(|(name, _)| *name);
        tools
            .into_iter()
            .map(|(_, tool)| Arc::clone(tool))
            .collect()
    }

    /// Re-describe an existing definition list against the world `ctx` runs in.
    ///
    /// A pass over the list the caller already has, not a rebuild from the
    /// registry. That list is the tool surface the session was booted with —
    /// already filtered by agent definition, whitelist and mode — and
    /// rebuilding it here would change *which* tools are advertised as a side
    /// effect of changing how one of them is described.
    ///
    /// A tool that declares nothing for this context — every tool but
    /// `TerminalCreate` today — has its entry left alone, not rebuilt, so a
    /// session with no backend gets back the bytes it handed in and the
    /// prompt-cache prefix survives a per-turn call. Anything this registry
    /// does not hold is left alone for the same reason: it is not a definition
    /// this registry has any standing to rewrite.
    ///
    /// Takes the list by value and edits in place. Every caller already owned
    /// one, and borrowing would have added a whole-list deep clone to the
    /// subagent spawn path, which previously moved its definitions straight
    /// through.
    pub fn redescribe(
        &self,
        mut definitions: Vec<serde_json::Value>,
        ctx: &ToolContext,
    ) -> Vec<serde_json::Value> {
        for definition in &mut definitions {
            // `get` on a non-object is `None`, so reaching an assignment below
            // means this definition is an object and indexing it is safe.
            let Some(tool) = definition
                .get("name")
                .and_then(serde_json::Value::as_str)
                .and_then(|name| self.tools.get(name))
            else {
                continue;
            };
            if let Some(schema) = tool.input_schema_for(ctx) {
                definition["input_schema"] = schema;
            }
            if let Some(description) = tool.description_for(ctx) {
                definition["description"] = serde_json::Value::String(description);
            }
        }
        definitions
    }
}

#[cfg(test)]
#[path = "dispatch_world_schema_tests.rs"]
mod tests;
