use super::helpers::create_plugin_agent;
use super::*;

#[test]
fn load_plugin_agents_discovers_single_plugin_agent() {
    let tmp = TempDir::new().unwrap();
    create_plugin_agent(tmp.path(), "foo", "bar", "Bar agent body.");

    let agents = load_plugin_agents(tmp.path()).unwrap();

    assert_eq!(agents.len(), 1, "should discover exactly one plugin agent");
    let bar = &agents[0];
    assert_eq!(
        bar.agent_type, "foo:bar",
        "agent_type must be prefixed with plugin name"
    );
    assert_eq!(
        bar.source,
        AgentSource::Plugin("foo".to_string()),
        "source must be Plugin(plugin_name)"
    );
    assert!(
        bar.description.contains("Bar agent body"),
        "description should come from agent.md INTENT"
    );
}

#[test]
fn load_plugin_agents_skips_underscore_plugin_dirs() {
    let tmp = TempDir::new().unwrap();
    create_plugin_agent(tmp.path(), "_internal", "bar", "Internal bar.");
    create_plugin_agent(tmp.path(), "real", "bar", "Real bar.");

    let agents = load_plugin_agents(tmp.path()).unwrap();

    assert_eq!(agents.len(), 1, "_-prefixed plugin dirs must be skipped");
    assert_eq!(agents[0].agent_type, "real:bar");
}

#[test]
fn load_plugin_agents_skips_underscore_agent_dirs() {
    let tmp = TempDir::new().unwrap();
    create_plugin_agent(tmp.path(), "foo", "_template", "Template.");
    create_plugin_agent(tmp.path(), "foo", "real", "Real agent.");

    let agents = load_plugin_agents(tmp.path()).unwrap();

    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].agent_type, "foo:real");
}

#[test]
fn load_plugin_agents_nonexistent_root_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let nonexistent = tmp.path().join("does-not-exist");
    let agents = load_plugin_agents(&nonexistent).unwrap();
    assert!(agents.is_empty());
}

#[test]
fn load_plugin_agents_empty_plugins_dir_returns_empty() {
    let tmp = TempDir::new().unwrap();
    // Create an empty plugins root.
    fs::create_dir_all(tmp.path()).unwrap();
    let agents = load_plugin_agents(tmp.path()).unwrap();
    assert!(agents.is_empty());
}

#[test]
fn load_plugin_agents_plugin_without_agents_dir_skipped() {
    let tmp = TempDir::new().unwrap();
    // A plugin with no agents/ subdir should be silently skipped.
    fs::create_dir_all(tmp.path().join("no-agents")).unwrap();
    // Plus a real one to ensure the loop continues.
    create_plugin_agent(tmp.path(), "has-agents", "bar", "Bar.");

    let agents = load_plugin_agents(tmp.path()).unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].agent_type, "has-agents:bar");
}

#[test]
fn load_plugin_agents_multiple_plugins_and_agents() {
    let tmp = TempDir::new().unwrap();
    create_plugin_agent(tmp.path(), "alpha", "one", "alpha one");
    create_plugin_agent(tmp.path(), "alpha", "two", "alpha two");
    create_plugin_agent(tmp.path(), "beta", "one", "beta one");

    let mut agents = load_plugin_agents(tmp.path()).unwrap();
    agents.sort_by(|a, b| a.agent_type.cmp(&b.agent_type));

    assert_eq!(agents.len(), 3);
    assert_eq!(agents[0].agent_type, "alpha:one");
    assert_eq!(agents[0].source, AgentSource::Plugin("alpha".into()));
    assert_eq!(agents[1].agent_type, "alpha:two");
    assert_eq!(agents[1].source, AgentSource::Plugin("alpha".into()));
    assert_eq!(agents[2].agent_type, "beta:one");
    assert_eq!(agents[2].source, AgentSource::Plugin("beta".into()));
}

// -----------------------------------------------------------------------
// Flat-file agent loader tests (v0.1.11)
// -----------------------------------------------------------------------
