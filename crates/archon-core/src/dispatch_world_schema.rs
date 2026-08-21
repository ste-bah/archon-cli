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

use archon_tools::tool::ToolContext;

use super::ToolRegistry;

impl ToolRegistry {
    /// Re-describe an existing definition list against the world `ctx` runs in.
    ///
    /// A pass over the list the caller already has, not a rebuild from the
    /// registry. That list is the tool surface the session was booted with —
    /// already filtered by agent definition, whitelist and mode — and
    /// rebuilding it here would change *which* tools are advertised as a side
    /// effect of changing how one of them is described.
    ///
    /// A tool that declares nothing for this context — every tool but
    /// `TerminalCreate` today — has its entry passed through untouched, so a
    /// session with no backend gets back the bytes it handed in and the
    /// prompt-cache prefix survives a per-turn call. Anything this registry
    /// does not hold is passed through for the same reason: it is not a
    /// definition this registry has any standing to rewrite.
    pub fn redescribe(
        &self,
        definitions: &[serde_json::Value],
        ctx: &ToolContext,
    ) -> Vec<serde_json::Value> {
        definitions
            .iter()
            .map(|definition| {
                // `get` on a non-object is `None`, so reaching the assignment
                // below means this definition is an object and indexing it is
                // safe.
                let described = definition
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|name| self.tools.get(name))
                    .and_then(|tool| tool.input_schema_for(ctx));
                let mut definition = definition.clone();
                if let Some(schema) = described {
                    definition["input_schema"] = schema;
                }
                definition
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "dispatch_world_schema_tests.rs"]
mod tests;
