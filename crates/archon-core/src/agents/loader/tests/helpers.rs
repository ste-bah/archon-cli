use super::*;

pub(super) fn create_test_agent(dir: &Path, name: &str) {
    let agent_dir = dir.join(name);
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(
        agent_dir.join("agent.md"),
        "# Test Agent\n\n## INTENT\nThis agent does testing.\n\n## SCOPE\nStuff\n",
    )
    .unwrap();
    fs::write(agent_dir.join("behavior.md"), "Be careful.\n").unwrap();
    fs::write(agent_dir.join("context.md"), "Context here.\n").unwrap();
    fs::write(
        agent_dir.join("tools.md"),
        "# Tools\n\n## Primary Tools\n- **Read**: read files\n- **Grep**: search\n",
    )
    .unwrap();
    fs::write(
        agent_dir.join("memory-keys.json"),
        r#"{"recall_queries":["testing"],"leann_queries":["test pattern"],"tags":["test"]}"#,
    )
    .unwrap();
    fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":5,"quality":{"applied_rate":0.9,"completion_rate":0.8},"evolution_history":[],"archived":false}"#,
        )
        .unwrap();
}

pub(super) fn create_plugin_agent(
    plugins_root: &Path,
    plugin: &str,
    agent: &str,
    agent_md_body: &str,
) {
    let agent_dir = plugins_root.join(plugin).join("agents").join(agent);
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(
        agent_dir.join("agent.md"),
        format!("# {agent}\n\n## INTENT\n{agent_md_body}\n"),
    )
    .unwrap();
    fs::write(agent_dir.join("behavior.md"), "Be careful.\n").unwrap();
    fs::write(agent_dir.join("context.md"), "Plugin context.\n").unwrap();
    fs::write(
        agent_dir.join("tools.md"),
        "# Tools\n\n## Primary Tools\n- **Read**: read files\n",
    )
    .unwrap();
    fs::write(
        agent_dir.join("memory-keys.json"),
        r#"{"recall_queries":[],"leann_queries":[],"tags":[]}"#,
    )
    .unwrap();
    fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false}"#,
        )
        .unwrap();
}
