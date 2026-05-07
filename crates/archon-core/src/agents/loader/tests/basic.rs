use super::helpers::create_test_agent;
use super::*;

#[test]
fn loads_agent_from_complete_directory() {
    let tmp = TempDir::new().unwrap();
    create_test_agent(tmp.path(), "test-agent");

    let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
    assert_eq!(agents.len(), 1);

    let agent = &agents[0];
    assert_eq!(agent.agent_type, "test-agent");
    assert_eq!(agent.description, "This agent does testing.");
    assert!(agent.system_prompt.contains("Test Agent"));
    assert!(agent.system_prompt.contains("Be careful."));
    assert!(agent.system_prompt.contains("Context here."));
    assert_eq!(
        agent.allowed_tools,
        Some(vec!["Read".to_string(), "Grep".to_string()])
    );
    assert_eq!(agent.recall_queries, vec!["testing"]);
    assert_eq!(agent.leann_queries, vec!["test pattern"]);
    assert_eq!(agent.tags, vec!["test"]);
    assert_eq!(agent.meta.invocation_count, 5);
    assert_eq!(agent.source, AgentSource::Project);
}

#[test]
fn skips_underscore_directories() {
    let tmp = TempDir::new().unwrap();
    create_test_agent(tmp.path(), "real-agent");
    fs::create_dir_all(tmp.path().join("_template")).unwrap();
    fs::create_dir_all(tmp.path().join("_behavior_schema")).unwrap();

    let agents = load_custom_agents(tmp.path(), AgentSource::User).unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].agent_type, "real-agent");
}

#[test]
fn missing_files_use_defaults() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("minimal");
    fs::create_dir_all(&agent_dir).unwrap();
    // Only create agent.md — everything else missing
    fs::write(agent_dir.join("agent.md"), "# Minimal Agent\n").unwrap();

    let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
    assert_eq!(agents.len(), 1);

    let agent = &agents[0];
    assert_eq!(agent.agent_type, "minimal");
    assert!(agent.allowed_tools.is_none()); // No tools.md → all tools
    assert!(agent.recall_queries.is_empty());
    assert_eq!(agent.meta.version, "1.0"); // Default
}

#[test]
fn malformed_meta_json_uses_defaults() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("bad-meta");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
    fs::write(agent_dir.join("meta.json"), "not valid json!!!").unwrap();

    let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].meta.version, "1.0"); // Default
    assert_eq!(agents[0].meta.invocation_count, 0);
}

#[test]
fn malformed_memory_keys_uses_defaults() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("bad-memory");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
    fs::write(agent_dir.join("memory-keys.json"), "{broken").unwrap();

    let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
    assert_eq!(agents.len(), 1);
    assert!(agents[0].recall_queries.is_empty());
    assert!(agents[0].leann_queries.is_empty());
}

#[test]
fn legacy_meta_json_version_as_integer() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("legacy");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.md"), "# Legacy\n").unwrap();
    fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":1,"created":"2026-01-01T00:00:00Z","invocation_count":42,"quality":{"applied_rate":0.95,"completion_rate":0.88},"evolution_history_last_10":[]}"#,
        )
        .unwrap();

    let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
    assert_eq!(agents[0].meta.version, "1.0");
    assert_eq!(agents[0].meta.invocation_count, 42);
}

#[test]
fn extract_description_from_intent() {
    let md = "# Agent\n\n## INTENT\nFirst paragraph here.\n\n## SCOPE\nOther stuff\n";
    assert_eq!(extract_description(md), "First paragraph here.");
}

#[test]
fn extract_description_multiline_paragraph() {
    let md = "# Agent\n\n## INTENT\nLine one of\nthe description.\n\n## SCOPE\n";
    assert_eq!(extract_description(md), "Line one of the description.");
}

#[test]
fn extract_description_no_intent_uses_first_line() {
    let md = "# My Great Agent\n\nSome content\n";
    assert_eq!(extract_description(md), "My Great Agent");
}

#[test]
fn extract_tools_from_primary_section() {
    let md = "# Tools\n\n## Primary Tools\n- **Read**: read files\n- **Grep**: search\n\n## Workflow\nStuff\n";
    let tools = extract_tools(md);
    assert_eq!(tools, Some(vec!["Read".to_string(), "Grep".to_string()]));
}

#[test]
fn extract_tools_none_when_no_section() {
    let md = "# Tools\n\nSome general info\n";
    assert!(extract_tools(md).is_none());
}

#[test]
fn extract_tools_none_when_section_empty() {
    let md = "# Tools\n\n## Primary Tools\n\n## Workflow\nStuff\n";
    assert!(extract_tools(md).is_none());
}

#[test]
fn empty_directory_returns_empty_vec() {
    let tmp = TempDir::new().unwrap();
    let agents = load_custom_agents(tmp.path(), AgentSource::User).unwrap();
    assert!(agents.is_empty());
}
