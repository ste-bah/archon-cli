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
    let registry = AgentRegistry::load_with_user_home(tmp.path(), None);
    // Four built-ins plus acceptance-reauthor and requirement-breaker (internal host roles).
    assert_eq!(registry.len(), 6);
    assert!(registry.resolve("general-purpose").is_some());
    assert!(registry.resolve("explore").is_some());
    assert!(registry.resolve("plan").is_some());
    assert!(registry.resolve("fork").is_some());
    assert!(registry.resolve("acceptance-reauthor").is_some());
    assert!(registry.resolve("requirement-breaker").is_some());
    assert!(registry.load_errors().is_empty());
}

#[test]
fn load_project_agents() {
    let tmp = TempDir::new().unwrap();
    let custom_dir = tmp.path().join(".archon/agents/custom");
    fs::create_dir_all(&custom_dir).unwrap();
    create_agent(&custom_dir, "my-agent");

    let registry = AgentRegistry::load_with_user_home(tmp.path(), None);
    assert_eq!(registry.len(), 7); // 4 built-in + 2 host + 1 project
    let agent = registry.resolve("my-agent").unwrap();
    assert_eq!(agent.source, AgentSource::Project);
}

#[test]
fn resolve_nonexistent_returns_none() {
    let tmp = TempDir::new().unwrap();
    let registry = AgentRegistry::load_with_user_home(tmp.path(), None);
    assert!(registry.resolve("nonexistent").is_none());
}

#[test]
fn list_sorted_alphabetically() {
    let tmp = TempDir::new().unwrap();
    let registry = AgentRegistry::load_with_user_home(tmp.path(), None);
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
    let mut registry = AgentRegistry::load_with_user_home(tmp.path(), None);

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

    let mut registry = AgentRegistry::load_with_user_home(tmp.path(), None);
    assert_eq!(registry.len(), 7); // 4 built-in + 2 host + 1

    // Add another agent on disk
    create_agent(&custom_dir, "new-agent");
    registry.reload_with_user_home(tmp.path(), None);
    assert_eq!(registry.len(), 8); // 4 built-in + 2 host + 2
    assert!(registry.resolve("new-agent").is_some());
}

#[test]
fn available_agent_names_sorted() {
    let tmp = TempDir::new().unwrap();
    let registry = AgentRegistry::load_with_user_home(tmp.path(), None);
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

    let registry = AgentRegistry::load_with_user_home(tmp.path(), None);
    assert_eq!(registry.len(), 7); // 4 built-in + 2 host + 1 (template skipped)
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

    let registry = AgentRegistry::load_with_user_home(&project_root, None);
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

    let registry = AgentRegistry::load_with_user_home(tmp.path(), None);
    assert!(registry.load_errors().is_empty());
}

#[test]
fn empty_registry_has_no_agents() {
    let registry = AgentRegistry::empty();
    assert!(registry.is_empty());
    assert_eq!(registry.len(), 0);
    assert!(registry.list().is_empty());
}

include!("registry_scope_tests.rs");
