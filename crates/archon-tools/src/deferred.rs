//! Deferred tool loading manager.
//!
//! Tracks which tools are "active" (sent to the LLM in API calls) vs "deferred"
//! (known but not sent until explicitly requested via ToolSearch).

use std::collections::{HashMap, HashSet};

/// Core tools that are always active and never deferred.
const CORE_TOOLS: &[&str] = &[
    "Read",
    "Write",
    "Edit",
    "Glob",
    "Grep",
    "Bash",
    "Agent",
    "ToolSearch",
];

/// Manages the active/deferred state of tools.
///
/// Core tools are always active. MCP and optional tools start deferred and can
/// be promoted for a single turn, then demoted back after the turn completes.
pub struct DeferredToolManager {
    /// Tools that are always active (core set).
    core: HashSet<String>,
    /// Tools that have been promoted for the current turn.
    promoted: HashSet<String>,
    /// All known tool definitions keyed by name.
    all_tools: HashMap<String, serde_json::Value>,
}

impl DeferredToolManager {
    /// Create a new manager. All tools in `tool_definitions` whose names are in
    /// [`CORE_TOOLS`] start active; the rest start deferred.
    pub fn new(tool_definitions: Vec<serde_json::Value>) -> Self {
        let core: HashSet<String> = CORE_TOOLS.iter().map(|s| (*s).to_string()).collect();

        let all_tools: HashMap<String, serde_json::Value> = tool_definitions
            .into_iter()
            .filter_map(|def| {
                let name = def.get("name")?.as_str()?.to_string();
                Some((name, def))
            })
            .collect();

        Self {
            core,
            promoted: HashSet::new(),
            all_tools,
        }
    }

    /// Returns `true` if the tool is currently active (core or promoted).
    pub fn is_active(&self, name: &str) -> bool {
        self.core.contains(name) || self.promoted.contains(name)
    }

    /// Returns `true` if the tool is known but deferred (not currently active).
    pub fn is_deferred(&self, name: &str) -> bool {
        self.all_tools.contains_key(name) && !self.is_active(name)
    }

    /// Promote a tool from deferred to active for the current turn.
    /// Returns `true` if the tool was found and promoted (or was already active).
    pub fn promote(&mut self, tool_name: &str) -> bool {
        if !self.all_tools.contains_key(tool_name) {
            return false;
        }
        if self.core.contains(tool_name) {
            return true; // already permanently active
        }
        self.promoted.insert(tool_name.to_string());
        true
    }

    /// Demote all promoted tools back to deferred. Called after each turn.
    pub fn demote_all(&mut self) {
        self.promoted.clear();
    }

    /// Return JSON schema definitions for all currently active tools.
    pub fn active_tool_definitions(&self) -> Vec<serde_json::Value> {
        self.all_tools
            .iter()
            .filter(|(name, _)| self.is_active(name))
            .map(|(_, def)| def.clone())
            .collect()
    }

    /// Return the names of all deferred tools.
    pub fn deferred_tool_names(&self) -> Vec<String> {
        self.all_tools
            .keys()
            .filter(|name| !self.is_active(name))
            .cloned()
            .collect()
    }

    /// Return the total number of known tools.
    pub fn total_tools(&self) -> usize {
        self.all_tools.len()
    }

    /// Return the number of currently active tools.
    pub fn active_count(&self) -> usize {
        self.all_tools
            .keys()
            .filter(|name| self.is_active(name))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_defs() -> Vec<serde_json::Value> {
        vec![
            json!({"name": "Read", "description": "Read files", "input_schema": {"type": "object"}}),
            json!({"name": "Write", "description": "Write files", "input_schema": {"type": "object"}}),
            json!({"name": "Bash", "description": "Run commands", "input_schema": {"type": "object"}}),
            json!({"name": "ToolSearch", "description": "Search tools", "input_schema": {"type": "object"}}),
            json!({"name": "mcp__memory__store", "description": "Store memory", "input_schema": {"type": "object"}}),
            json!({"name": "mcp__serena__find_symbol", "description": "Find symbol", "input_schema": {"type": "object"}}),
            json!({"name": "WebFetch", "description": "Fetch web", "input_schema": {"type": "object"}}),
        ]
    }

    #[test]
    fn core_tools_are_active_by_default() {
        let mgr = DeferredToolManager::new(sample_defs());
        assert!(mgr.is_active("Read"));
        assert!(mgr.is_active("Write"));
        assert!(mgr.is_active("Bash"));
        assert!(mgr.is_active("ToolSearch"));
    }

    #[test]
    fn mcp_tools_start_deferred() {
        let mgr = DeferredToolManager::new(sample_defs());
        assert!(mgr.is_deferred("mcp__memory__store"));
        assert!(mgr.is_deferred("mcp__serena__find_symbol"));
        assert!(!mgr.is_active("mcp__memory__store"));
    }

    #[test]
    fn promote_moves_to_active() {
        let mut mgr = DeferredToolManager::new(sample_defs());
        assert!(mgr.is_deferred("mcp__memory__store"));
        let ok = mgr.promote("mcp__memory__store");
        assert!(ok);
        assert!(mgr.is_active("mcp__memory__store"));
        assert!(!mgr.is_deferred("mcp__memory__store"));
    }

    #[test]
    fn promote_unknown_returns_false() {
        let mut mgr = DeferredToolManager::new(sample_defs());
        assert!(!mgr.promote("NonExistent"));
    }

    #[test]
    fn demote_all_resets_promoted() {
        let mut mgr = DeferredToolManager::new(sample_defs());
        mgr.promote("mcp__memory__store");
        mgr.promote("WebFetch");
        assert!(mgr.is_active("mcp__memory__store"));
        assert!(mgr.is_active("WebFetch"));

        mgr.demote_all();
        assert!(mgr.is_deferred("mcp__memory__store"));
        assert!(mgr.is_deferred("WebFetch"));
        // Core tools remain active after demote
        assert!(mgr.is_active("Read"));
    }

    #[test]
    fn active_definitions_returns_only_active() {
        let mgr = DeferredToolManager::new(sample_defs());
        let active = mgr.active_tool_definitions();
        let names: Vec<&str> = active.iter().filter_map(|d| d["name"].as_str()).collect();
        // Only core tools from sample_defs are active: Read, Write, Bash, ToolSearch
        assert_eq!(names.len(), 4);
        assert!(names.contains(&"Read"));
        assert!(!names.contains(&"mcp__memory__store"));
    }

    #[test]
    fn deferred_tool_names_lists_non_active() {
        let mgr = DeferredToolManager::new(sample_defs());
        let mut deferred = mgr.deferred_tool_names();
        deferred.sort();
        assert_eq!(
            deferred,
            vec!["WebFetch", "mcp__memory__store", "mcp__serena__find_symbol"]
        );
    }

    #[test]
    fn counts_are_correct() {
        let mut mgr = DeferredToolManager::new(sample_defs());
        assert_eq!(mgr.total_tools(), 7);
        assert_eq!(mgr.active_count(), 4); // Read, Write, Bash, ToolSearch
        mgr.promote("WebFetch");
        assert_eq!(mgr.active_count(), 5);
    }
}
