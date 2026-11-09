use std::collections::HashMap;
use std::path::Path;

use tracing::debug;

use super::built_in::get_built_in_agents;
use super::definition::{AgentSource, CustomAgentDefinition};
use super::harness::get_harness_agents;
use super::loader::{
    AgentLoadError, load_custom_agents, load_flat_file_agents, load_plugin_agents,
};

// ---------------------------------------------------------------------------
// AgentRegistry
// ---------------------------------------------------------------------------

/// Central lookup for all agent definitions.
///
/// Priority (lowest → highest, later wins on name conflict):
///   built-in < project plugins < user plugins < project flat-file
///   < project custom (6-file) < user flat-file < user custom (6-file)
///   < host agents (highest; see `harness`)
///
/// User scope wins over project; 6-file format wins over flat-file at the
/// same scope (6-file is the more explicit, configurable surface).
#[derive(Debug)]
pub struct AgentRegistry {
    agents: HashMap<String, CustomAgentDefinition>,
    load_errors: Vec<AgentLoadError>,
}

impl AgentRegistry {
    /// Load agents from all sources using the real user home directory.
    pub fn load(project_dir: &Path) -> Self {
        Self::load_with_user_home(project_dir, dirs::home_dir().as_deref())
    }

    /// Load agents from all sources with an explicit user home directory.
    ///
    /// Priority (lowest → highest, later entries override earlier ones on
    /// key collision):
    ///
    /// 1. Built-in agents
    /// 2. Project plugin agents (`<project>/.archon/plugins/*/agents/*`)
    /// 3. User plugin agents (`<user_home>/.archon/plugins/*/agents/*`)
    /// 4. Project flat-file agents — YAML frontmatter (`<project>/.archon/agents/*.md`)
    /// 5. Project custom agents — 6-file format (`<project>/.archon/agents/custom/*`)
    /// 6. User flat-file agents — YAML frontmatter (`<user_home>/.archon/agents/*.md`)
    /// 7. User custom agents — 6-file format (`<user_home>/.archon/agents/custom/*`)
    /// 8. Host agents (`harness`) — fixed keys the workflow host launches
    ///
    /// User scope wins over project scope. 6-file format wins over flat-file
    /// at the same scope (6-file is the more explicit, configurable surface).
    pub fn load_with_user_home(project_dir: &Path, user_home: Option<&Path>) -> Self {
        let mut agents = HashMap::new();
        let mut errors = Vec::new();

        // 1. Built-in agents (lowest priority)
        for agent in get_built_in_agents() {
            agents.insert(agent.agent_type.clone(), agent);
        }

        // 2. Project plugin agents (<project>/.archon/plugins/)
        let project_plugins = project_dir.join(".archon/plugins");
        match load_plugin_agents(&project_plugins) {
            Ok(loaded) => {
                debug!(count = loaded.len(), "loaded project plugin agents");
                for a in loaded {
                    agents.insert(a.agent_type.clone(), a);
                }
            }
            Err(e) => errors.push(e),
        }

        // 3. User plugin agents (<user_home>/.archon/plugins/)
        if let Some(home) = user_home {
            let user_plugins = home.join(".archon/plugins");
            match load_plugin_agents(&user_plugins) {
                Ok(loaded) => {
                    debug!(count = loaded.len(), "loaded user plugin agents");
                    for a in loaded {
                        agents.insert(a.agent_type.clone(), a);
                    }
                }
                Err(e) => errors.push(e),
            }
        }

        // 4. Project flat-file agents (.archon/agents/*.md, recursive)
        //    — claude-flow YAML-frontmatter shape. Skips custom/ subdir
        //    (handled by step 5) and dotfiles/_-prefixed dirs.
        let project_flat = project_dir.join(".archon/agents");
        match load_flat_file_agents(&project_flat, AgentSource::Project) {
            Ok(loaded) => {
                debug!(count = loaded.len(), "loaded project flat-file agents");
                for a in loaded {
                    agents.insert(a.agent_type.clone(), a);
                }
            }
            Err(e) => errors.push(e),
        }

        // 5. Project custom agents (.archon/agents/custom/) — 6-file format.
        //    Loaded AFTER flat-file so the more explicit 6-file shape wins
        //    on key collision at the same scope.
        let project_custom = project_dir.join(".archon/agents/custom");
        if project_custom.is_dir() {
            match load_custom_agents(&project_custom, AgentSource::Project) {
                Ok(loaded) => {
                    debug!(count = loaded.len(), "loaded project agents");
                    for a in loaded {
                        agents.insert(a.agent_type.clone(), a);
                    }
                }
                Err(e) => errors.push(e),
            }
        }

        // 6. User flat-file agents (~/.archon/agents/*.md, recursive)
        if let Some(home) = user_home {
            let user_flat = home.join(".archon/agents");
            match load_flat_file_agents(&user_flat, AgentSource::User) {
                Ok(loaded) => {
                    debug!(count = loaded.len(), "loaded user flat-file agents");
                    for a in loaded {
                        agents.insert(a.agent_type.clone(), a);
                    }
                }
                Err(e) => errors.push(e),
            }
        }

        // 7. User custom agents (~/.archon/agents/custom/) — 6-file format.
        //    Loaded AFTER flat-file at the same scope for the same reason.
        if let Some(home) = user_home {
            let user_custom = home.join(".archon/agents/custom");
            if user_custom.is_dir() {
                match load_custom_agents(&user_custom, AgentSource::User) {
                    Ok(loaded) => {
                        debug!(count = loaded.len(), "loaded user agents");
                        for a in loaded {
                            agents.insert(a.agent_type.clone(), a);
                        }
                    }
                    Err(e) => errors.push(e),
                }
            }
        }

        // 8. Host agents last: a file of the same name never redefines them.
        for agent in get_harness_agents() {
            if let Some(replaced) = agents.insert(agent.agent_type.clone(), agent) {
                tracing::warn!(
                    agent_type = %replaced.agent_type,
                    source = ?replaced.source,
                    "ignoring an on-disk agent that reuses a host agent's name"
                );
            }
        }

        Self {
            agents,
            load_errors: errors,
        }
    }

    /// Create an empty registry (no agents, no built-ins). Used for testing.
    pub fn empty() -> Self {
        Self {
            agents: HashMap::new(),
            load_errors: Vec::new(),
        }
    }

    /// Look up an agent by type name.
    pub fn resolve(&self, agent_type: &str) -> Option<&CustomAgentDefinition> {
        self.agents.get(agent_type)
    }

    /// List all agents, sorted alphabetically by agent_type.
    pub fn list(&self) -> Vec<&CustomAgentDefinition> {
        let mut agents: Vec<&CustomAgentDefinition> = self.agents.values().collect();
        agents.sort_by_key(|a| &a.agent_type);
        agents
    }

    /// List agents filtered by available MCP servers.
    ///
    /// Agents with `required_mcp_servers` that aren't satisfied by `available`
    /// are excluded. Agents without requirements are always included.
    pub fn list_with_mcp_filter(&self, available: &[String]) -> Vec<&CustomAgentDefinition> {
        let mut agents: Vec<&CustomAgentDefinition> = self
            .agents
            .values()
            .filter(|a| a.has_required_mcp_servers(available))
            .collect();
        agents.sort_by_key(|a| &a.agent_type);
        agents
    }

    /// Re-read all agent definitions from disk.
    pub fn reload(&mut self, project_dir: &Path) {
        *self = Self::load(project_dir);
    }

    /// Reload from all sources with an explicit user home directory.
    ///
    /// Mirrors [`Self::load_with_user_home`]. Tests must use this rather than
    /// [`Self::reload`]: `load` resolves the user scope from
    /// `dirs::home_dir()`, so a test that only controls `project_dir` still
    /// picks up whatever the developer has in `~/.archon`, and user scope wins
    /// over project scope.
    pub fn reload_with_user_home(&mut self, project_dir: &Path, user_home: Option<&Path>) {
        *self = Self::load_with_user_home(project_dir, user_home);
    }

    /// Errors encountered during the last load/reload.
    pub fn load_errors(&self) -> &[AgentLoadError] {
        &self.load_errors
    }

    /// Every agent name a task or stage may be routed to (for error messages
    /// and workflow routing). Host agents resolve by key but are never offered:
    /// a task routed to the read-only re-author could not do its work.
    pub fn available_agent_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .agents
            .keys()
            .map(|s| s.as_str())
            .filter(|name| !super::harness::is_host_agent(name))
            .collect();
        names.sort();
        names
    }

    /// Number of agents currently registered.
    pub fn len(&self) -> usize {
        self.agents.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }

    /// Return a map of agent_type → color for agents that have a color defined.
    /// Used by the TUI to render colored agent type labels.
    pub fn color_map(&self) -> HashMap<String, String> {
        self.agents
            .iter()
            .filter_map(|(name, def)| def.color.as_ref().map(|c| (name.clone(), c.clone())))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)] // Tests mutate constructed defaults; refactoring all sites to struct-init is out of scope.
#[path = "registry_tests.rs"]
mod tests;
