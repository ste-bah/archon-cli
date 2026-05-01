use std::collections::HashMap;
use std::path::Path;

use tracing::debug;

use super::built_in::get_built_in_agents;
use super::definition::{AgentSource, CustomAgentDefinition};
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
///   < project custom (6-file) < user flat-file < user custom (6-file) (highest)
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

    /// Errors encountered during the last load/reload.
    pub fn load_errors(&self) -> &[AgentLoadError] {
        &self.load_errors
    }

    /// All registered agent names (for error messages listing available agents).
    pub fn available_agent_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.agents.keys().map(|s| s.as_str()).collect();
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
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Create a minimal agent directory for testing.
    fn create_agent(dir: &Path, name: &str) {
        let agent_dir = dir.join(name);
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(
            agent_dir.join("agent.md"),
            format!("# {name}\n\n## INTENT\nTest agent {name}.\n"),
        )
        .unwrap();
        fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false}"#,
        )
        .unwrap();
    }

    #[test]
    fn load_with_no_agent_dirs_returns_builtins_only() {
        let tmp = TempDir::new().unwrap();
        let registry = AgentRegistry::load(tmp.path());
        // Only 4 built-in agents (general-purpose, explore, plan, fork)
        assert_eq!(registry.len(), 4);
        assert!(registry.resolve("general-purpose").is_some());
        assert!(registry.resolve("explore").is_some());
        assert!(registry.resolve("plan").is_some());
        assert!(registry.resolve("fork").is_some());
        assert!(registry.load_errors().is_empty());
    }

    #[test]
    fn load_project_agents() {
        let tmp = TempDir::new().unwrap();
        let custom_dir = tmp.path().join(".archon/agents/custom");
        fs::create_dir_all(&custom_dir).unwrap();
        create_agent(&custom_dir, "my-agent");

        let registry = AgentRegistry::load(tmp.path());
        assert_eq!(registry.len(), 5); // 4 built-in + 1 project
        let agent = registry.resolve("my-agent").unwrap();
        assert_eq!(agent.source, AgentSource::Project);
    }

    #[test]
    fn resolve_nonexistent_returns_none() {
        let tmp = TempDir::new().unwrap();
        let registry = AgentRegistry::load(tmp.path());
        assert!(registry.resolve("nonexistent").is_none());
    }

    #[test]
    fn list_sorted_alphabetically() {
        let tmp = TempDir::new().unwrap();
        let registry = AgentRegistry::load(tmp.path());
        let list = registry.list();
        let names: Vec<&str> = list.iter().map(|a| a.agent_type.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    #[test]
    fn user_agent_overrides_project_agent() {
        let tmp = TempDir::new().unwrap();
        // Simulate project agent by creating directly in registry
        let mut registry = AgentRegistry::load(tmp.path());

        // Insert project-level agent
        let project_agent = CustomAgentDefinition {
            agent_type: "shared".into(),
            description: "project version".into(),
            source: AgentSource::Project,
            ..CustomAgentDefinition::default()
        };
        registry.agents.insert("shared".into(), project_agent);

        // Insert user-level agent (higher priority — same name, overwrites)
        let user_agent = CustomAgentDefinition {
            agent_type: "shared".into(),
            description: "user version".into(),
            source: AgentSource::User,
            ..CustomAgentDefinition::default()
        };
        registry.agents.insert("shared".into(), user_agent);

        let resolved = registry.resolve("shared").unwrap();
        assert_eq!(resolved.description, "user version");
        assert_eq!(resolved.source, AgentSource::User);
    }

    #[test]
    fn reload_picks_up_new_agents() {
        let tmp = TempDir::new().unwrap();
        let custom_dir = tmp.path().join(".archon/agents/custom");
        fs::create_dir_all(&custom_dir).unwrap();
        create_agent(&custom_dir, "original");

        let mut registry = AgentRegistry::load(tmp.path());
        assert_eq!(registry.len(), 5); // 4 built-in + 1

        // Add another agent on disk
        create_agent(&custom_dir, "new-agent");
        registry.reload(tmp.path());
        assert_eq!(registry.len(), 6); // 4 built-in + 2
        assert!(registry.resolve("new-agent").is_some());
    }

    #[test]
    fn available_agent_names_sorted() {
        let tmp = TempDir::new().unwrap();
        let registry = AgentRegistry::load(tmp.path());
        let names = registry.available_agent_names();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
        assert!(names.contains(&"general-purpose"));
    }

    #[test]
    fn list_with_mcp_filter_excludes_unmet() {
        let mut registry = AgentRegistry::empty();

        let mut agent_github = CustomAgentDefinition::default();
        agent_github.agent_type = "github-helper".into();
        agent_github.required_mcp_servers = Some(vec!["github".into()]);
        registry.agents.insert("github-helper".into(), agent_github);

        let mut agent_plain = CustomAgentDefinition::default();
        agent_plain.agent_type = "plain-agent".into();
        registry.agents.insert("plain-agent".into(), agent_plain);

        // No MCP servers available — github-helper excluded
        let list = registry.list_with_mcp_filter(&[]);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].agent_type, "plain-agent");

        // github server available — both included
        let list = registry.list_with_mcp_filter(&["mcp__github__api".into()]);
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn underscore_dirs_skipped_in_project() {
        let tmp = TempDir::new().unwrap();
        let custom_dir = tmp.path().join(".archon/agents/custom");
        fs::create_dir_all(&custom_dir).unwrap();
        create_agent(&custom_dir, "real-agent");
        fs::create_dir_all(custom_dir.join("_template")).unwrap();

        let registry = AgentRegistry::load(tmp.path());
        assert_eq!(registry.len(), 5); // 4 built-in + 1 (template skipped)
        assert!(registry.resolve("_template").is_none());
    }

    // -----------------------------------------------------------------------
    // Additional validation criteria tests (AGT-003)
    // -----------------------------------------------------------------------

    #[test]
    fn load_real_agents_from_project_dir() {
        // Load from the actual project root which has .archon/agents/custom/
        let project_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let custom_dir = project_root.join(".archon/agents/custom");
        if !custom_dir.exists() {
            eprintln!(
                "Skipping: .archon/agents/custom/ not found at {:?}",
                custom_dir
            );
            return;
        }

        let registry = AgentRegistry::load(&project_root);
        // 4 built-in + 9 project agents = at least 13
        assert!(
            registry.len() >= 13,
            "Expected at least 13 agents (4 built-in + 9 project), got {}",
            registry.len()
        );

        // code-reviewer should be resolvable
        let reviewer = registry.resolve("code-reviewer");
        assert!(reviewer.is_some(), "code-reviewer should be resolvable");
        assert_eq!(reviewer.unwrap().source, AgentSource::Project);
    }

    #[test]
    fn load_errors_empty_when_no_failures() {
        let tmp = TempDir::new().unwrap();
        let custom_dir = tmp.path().join(".archon/agents/custom");
        fs::create_dir_all(&custom_dir).unwrap();
        create_agent(&custom_dir, "good-agent");

        let registry = AgentRegistry::load(tmp.path());
        assert!(registry.load_errors().is_empty());
    }

    #[test]
    fn empty_registry_has_no_agents() {
        let registry = AgentRegistry::empty();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        assert!(registry.list().is_empty());
    }

    // -----------------------------------------------------------------------
    // G5 — plugin agent loading + priority tests
    // -----------------------------------------------------------------------

    /// Create a minimal plugin agent fixture at
    /// `<root>/<plugin>/agents/<agent>/` with the 6-file structure. The
    /// `marker` string is embedded in the INTENT body so tests can assert
    /// which version (project vs user vs custom) won a priority collision.
    fn create_plugin_fixture(plugins_root: &Path, plugin: &str, agent: &str, marker: &str) {
        let dir = plugins_root.join(plugin).join("agents").join(agent);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("agent.md"),
            format!("# {agent}\n\n## INTENT\n{marker}\n"),
        )
        .unwrap();
        fs::write(dir.join("behavior.md"), "").unwrap();
        fs::write(dir.join("context.md"), "").unwrap();
        fs::write(dir.join("tools.md"), "").unwrap();
        fs::write(
            dir.join("memory-keys.json"),
            r#"{"recall_queries":[],"leann_queries":[],"tags":[]}"#,
        )
        .unwrap();
        fs::write(
            dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false}"#,
        )
        .unwrap();
    }

    #[test]
    fn registry_loads_project_plugin_agent() {
        let project = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();

        create_plugin_fixture(
            &project.path().join(".archon/plugins"),
            "foo",
            "bar",
            "project-foo-bar",
        );

        let registry = AgentRegistry::load_with_user_home(project.path(), Some(user.path()));

        let agent = registry
            .resolve("foo:bar")
            .expect("foo:bar should be resolvable from project plugin");
        assert_eq!(
            agent.source,
            AgentSource::Plugin("foo".to_string()),
            "source must be Plugin(foo)"
        );
        assert!(
            agent.description.contains("project-foo-bar"),
            "description should come from project plugin fixture"
        );
    }

    #[test]
    fn registry_user_plugin_beats_project_plugin_on_same_key() {
        // G5 priority: user plugin > project plugin (but custom beats both).
        let project = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();

        create_plugin_fixture(
            &project.path().join(".archon/plugins"),
            "foo",
            "bar",
            "project-version",
        );
        create_plugin_fixture(
            &user.path().join(".archon/plugins"),
            "foo",
            "bar",
            "user-version",
        );

        let registry = AgentRegistry::load_with_user_home(project.path(), Some(user.path()));
        let agent = registry.resolve("foo:bar").expect("foo:bar must resolve");

        assert_eq!(
            agent.source,
            AgentSource::Plugin("foo".to_string()),
            "source remains Plugin(foo) — both versions belong to the same plugin name"
        );
        assert!(
            agent.description.contains("user-version"),
            "user plugin must win over project plugin on key collision; \
             got description: {:?}",
            agent.description
        );
        assert!(
            !agent.description.contains("project-version"),
            "project-version must have been overwritten by user-version"
        );
    }

    #[test]
    fn registry_underscore_plugin_dir_skipped() {
        let project = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();

        create_plugin_fixture(
            &project.path().join(".archon/plugins"),
            "_internal",
            "bar",
            "internal-bar",
        );

        let registry = AgentRegistry::load_with_user_home(project.path(), Some(user.path()));
        assert!(
            registry.resolve("_internal:bar").is_none(),
            "_-prefixed plugin dirs must be skipped entirely"
        );
    }

    // #234: Windows does not allow `:` in filenames. The fixture below
    // creates `.archon/agents/custom/foo:bar` which `fs::create_dir_all`
    // rejects with `Os { code: 267, kind: NotADirectory }` on Windows.
    // The source comment a few lines below ("our test tmp on Linux allows
    // colons in filenames") already acknowledges this constraint — gate
    // the test to non-Windows platforms.
    #[cfg(not(windows))]
    #[test]
    fn registry_custom_agent_beats_plugin_with_same_key() {
        // G5 priority: project custom > user plugin.
        // Create a plugin providing "foo:bar" in USER scope (higher of the
        // two plugin levels) and a custom agent literally named "foo:bar"
        // in PROJECT scope. Custom must still win.
        let project = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();

        create_plugin_fixture(
            &user.path().join(".archon/plugins"),
            "foo",
            "bar",
            "plugin-version",
        );

        // Custom agent with literal type "foo:bar" (the agent dir name
        // includes the colon on platforms that allow it; our test tmp on
        // Linux allows colons in filenames).
        let custom_dir = project.path().join(".archon/agents/custom/foo:bar");
        fs::create_dir_all(&custom_dir).unwrap();
        fs::write(
            custom_dir.join("agent.md"),
            "# foo:bar\n\n## INTENT\ncustom-version\n",
        )
        .unwrap();
        fs::write(
            custom_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false}"#,
        )
        .unwrap();

        let registry = AgentRegistry::load_with_user_home(project.path(), Some(user.path()));
        let agent = registry.resolve("foo:bar").expect("foo:bar must resolve");

        assert_eq!(
            agent.source,
            AgentSource::Project,
            "custom project agent must override user plugin"
        );
        assert!(
            agent.description.contains("custom-version"),
            "custom agent must win; got {:?}",
            agent.description
        );
    }

    #[test]
    fn color_map_returns_only_colored_agents() {
        let mut registry = AgentRegistry::empty();

        let mut colored = CustomAgentDefinition::default();
        colored.agent_type = "colored".into();
        colored.color = Some("#ff0000".into());
        registry.agents.insert("colored".into(), colored);

        let mut plain = CustomAgentDefinition::default();
        plain.agent_type = "plain".into();
        registry.agents.insert("plain".into(), plain);

        let map = registry.color_map();
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("colored").unwrap(), "#ff0000");
    }

    // -----------------------------------------------------------------------
    // Flat-file agent loading tests (v0.1.11)
    // -----------------------------------------------------------------------

    #[test]
    fn load_picks_up_flat_file_agents() {
        let project = TempDir::new().unwrap();
        let agents_dir = project.path().join(".archon/agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("foo.md"),
            "---\nname: foo\ndescription: A flat-file agent\n---\n\nFlat file body.\n",
        )
        .unwrap();

        let registry = AgentRegistry::load_with_user_home(project.path(), None);
        let agent = registry
            .resolve("foo")
            .expect("flat-file agent foo must resolve");
        assert_eq!(agent.source, AgentSource::Project);
        assert_eq!(agent.description, "A flat-file agent");
    }

    #[test]
    fn load_combines_6file_and_flat_file() {
        let project = TempDir::new().unwrap();
        fs::create_dir_all(project.path().join(".archon/agents/custom/six-file")).unwrap();
        create_agent(&project.path().join(".archon/agents/custom"), "six-file");
        fs::write(
            project.path().join(".archon/agents/flat.md"),
            "---\nname: flat\ndescription: A flat-file agent\n---\n\nFlat body.\n",
        )
        .unwrap();

        let registry = AgentRegistry::load_with_user_home(project.path(), None);
        assert!(
            registry.resolve("six-file").is_some(),
            "6-file agent must resolve"
        );
        assert!(
            registry.resolve("flat").is_some(),
            "flat-file agent must resolve"
        );
        // 4 built-ins + 1 custom 6-file + 1 flat-file = 6
        assert_eq!(registry.len(), 6);
    }

    #[test]
    fn load_user_flat_file_overrides_project_flat_file() {
        let project = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();

        fs::create_dir_all(project.path().join(".archon/agents")).unwrap();
        fs::write(
            project.path().join(".archon/agents/shared.md"),
            "---\nname: shared\ndescription: project flat-file version\n---\n\nProject body.\n",
        )
        .unwrap();

        fs::create_dir_all(user.path().join(".archon/agents")).unwrap();
        fs::write(
            user.path().join(".archon/agents/shared.md"),
            "---\nname: shared\ndescription: user flat-file version\n---\n\nUser body.\n",
        )
        .unwrap();

        let registry = AgentRegistry::load_with_user_home(project.path(), Some(user.path()));
        let agent = registry.resolve("shared").expect("shared must resolve");
        assert_eq!(
            agent.description, "user flat-file version",
            "user flat-file must override project flat-file"
        );
        assert_eq!(agent.source, AgentSource::User);
    }

    #[test]
    fn load_6file_overrides_flat_file_at_same_scope() {
        let project = TempDir::new().unwrap();

        // Flat-file agent named "collision"
        fs::create_dir_all(project.path().join(".archon/agents")).unwrap();
        fs::write(
            project.path().join(".archon/agents/collision.md"),
            "---\nname: collision\ndescription: flat-file version\n---\n\nFlat body.\n",
        )
        .unwrap();

        // 6-file agent also named "collision" in custom/
        create_agent(&project.path().join(".archon/agents/custom"), "collision");

        let registry = AgentRegistry::load_with_user_home(project.path(), None);
        let agent = registry
            .resolve("collision")
            .expect("collision must resolve");
        assert_eq!(
            agent.source,
            AgentSource::Project,
            "6-file format must win over flat-file at the same project scope"
        );
        assert!(
            agent.description.contains("Test agent collision"),
            "6-file agent description must appear; got {:?}",
            agent.description
        );
    }

    /// Lockstep regression test: flat-file agents from a real fixture are
    /// loaded and counted. Without this, the two-system split (DiscoveryCatalog
    /// vs AgentRegistry) can drift again.
    #[test]
    fn load_real_flat_file_agents_from_archon_dir() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/flat_file_agents");
        let registry = AgentRegistry::load_with_user_home(&fixture, None);

        let mut names: Vec<&str> = registry
            .list()
            .iter()
            .filter(|d| matches!(d.source, AgentSource::Project))
            .map(|d| d.agent_type.as_str())
            .collect();
        names.sort();

        // Expected: a, b, c (from sub/).
        // custom/skip.md skipped by filter_entry (custom/ excluded).
        // README.md skipped (no frontmatter).
        assert_eq!(
            names,
            vec!["a", "b", "c"],
            "expected 3 flat-file project agents; got {:?}",
            names
        );
    }
}
