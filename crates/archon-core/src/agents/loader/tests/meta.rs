use super::*;

#[test]
fn exec_config_parsed_from_meta_json() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("configured");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
    fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false,"model":"opus","effort":"high","max_turns":20,"background":true}"#,
        )
        .unwrap();

    let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
    let agent = &agents[0];
    assert_eq!(agent.model.as_deref(), Some("opus"));
    assert_eq!(agent.effort.as_deref(), Some("high"));
    assert_eq!(agent.max_turns, Some(20));
    assert!(agent.background);
}

#[test]
fn isolation_parsed_from_meta_json() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("isolated");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
    fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false,"isolation":"worktree"}"#,
        )
        .unwrap();

    let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
    assert_eq!(agents[0].isolation.as_deref(), Some("worktree"));
}

#[test]
fn mcp_servers_parsed_from_meta_json() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("mcp-agent");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
    fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false,"mcp_servers":["github","slack"],"required_mcp_servers":["github"]}"#,
        )
        .unwrap();

    let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
    assert_eq!(
        agents[0].mcp_servers,
        Some(vec!["github".into(), "slack".into()])
    );
    assert_eq!(agents[0].required_mcp_servers, Some(vec!["github".into()]));
}

#[test]
fn mcp_servers_none_when_not_in_meta() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("no-mcp");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
    fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false}"#,
        )
        .unwrap();

    let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
    assert!(agents[0].mcp_servers.is_none());
    assert!(agents[0].required_mcp_servers.is_none());
}

#[test]
fn isolation_none_when_not_in_meta() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("no-isolation");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
    fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false}"#,
        )
        .unwrap();

    let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
    assert!(agents[0].isolation.is_none());
}

// -----------------------------------------------------------------------
// Session-scoped hooks tests (AGT-019)
// -----------------------------------------------------------------------

#[test]
fn hooks_parsed_from_meta_json() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("hooked");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
    fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false,"hooks":{"PreToolUse":[{"type":"command","command":"check.sh"}]}}"#,
        )
        .unwrap();

    let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
    let hooks = agents[0].hooks.as_ref().expect("hooks should be Some");
    assert!(hooks.is_object());
    assert!(hooks.get("PreToolUse").is_some());
}

#[test]
fn hooks_none_when_not_in_meta_json() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("no-hooks");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
    fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false}"#,
        )
        .unwrap();

    let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
    assert!(agents[0].hooks.is_none());
}

#[test]
fn parse_agent_hooks_valid_single_event() {
    use crate::hooks::{HookCommandType, HookEvent};
    let json: serde_json::Value = serde_json::json!({
        "PreToolUse": [
            {"type": "command", "command": "check.sh"},
            {"type": "command", "command": "lint.sh", "timeout": 30}
        ]
    });
    let hooks = parse_agent_hooks(&json).unwrap();
    assert_eq!(hooks.len(), 2);
    assert_eq!(hooks[0].0, HookEvent::PreToolUse);
    assert_eq!(hooks[0].1.hook_type, HookCommandType::Command);
    assert_eq!(hooks[0].1.command, "check.sh");
    assert_eq!(hooks[1].0, HookEvent::PreToolUse);
    assert_eq!(hooks[1].1.command, "lint.sh");
    assert_eq!(hooks[1].1.timeout, Some(30));
}

#[test]
fn parse_agent_hooks_multiple_events() {
    use crate::hooks::HookEvent;
    let json: serde_json::Value = serde_json::json!({
        "SessionStart": [{"type": "command", "command": "setup.sh"}],
        "PreToolUse": [{"type": "command", "command": "guard.sh"}]
    });
    let hooks = parse_agent_hooks(&json).unwrap();
    assert_eq!(hooks.len(), 2);
    let events: Vec<&HookEvent> = hooks.iter().map(|(e, _)| e).collect();
    assert!(events.contains(&&HookEvent::SessionStart));
    assert!(events.contains(&&HookEvent::PreToolUse));
}

#[test]
fn parse_agent_hooks_empty_object() {
    let json: serde_json::Value = serde_json::json!({});
    let hooks = parse_agent_hooks(&json).unwrap();
    assert!(hooks.is_empty());
}

#[test]
fn parse_agent_hooks_not_object_returns_error() {
    let json: serde_json::Value = serde_json::json!("not an object");
    let err = parse_agent_hooks(&json).unwrap_err();
    assert!(err.contains("must be a JSON object"));
}

#[test]
fn parse_agent_hooks_unknown_event_returns_error() {
    let json: serde_json::Value = serde_json::json!({
        "BogusEvent": [{"type": "command", "command": "x.sh"}]
    });
    let err = parse_agent_hooks(&json).unwrap_err();
    assert!(err.contains("unknown hook event"));
}

// -----------------------------------------------------------------------
// omitClaudeMd tests (AGT-021)
// -----------------------------------------------------------------------

#[test]
fn omit_claude_md_parsed_from_meta_json() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("readonly");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
    fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false,"omit_claude_md":true}"#,
        )
        .unwrap();

    let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
    assert!(agents[0].omit_claude_md);
}

#[test]
fn omit_claude_md_defaults_to_false() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("default");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
    fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false}"#,
        )
        .unwrap();

    let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
    assert!(!agents[0].omit_claude_md);
}

#[test]
fn parse_agent_hooks_malformed_config_returns_error() {
    let json: serde_json::Value = serde_json::json!({
        "PreToolUse": [{"missing_type_field": true}]
    });
    let err = parse_agent_hooks(&json).unwrap_err();
    assert!(err.contains("malformed hook configs"));
}

// -----------------------------------------------------------------------
// Skills preloading tests (AGT-020)
// -----------------------------------------------------------------------

#[test]
fn skills_parsed_from_meta_json() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("skilled");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
    fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false,"skills":["commit","review-pr"]}"#,
        )
        .unwrap();

    let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
    let skills = agents[0].skills.as_ref().expect("skills should be Some");
    assert_eq!(skills, &vec!["commit".to_string(), "review-pr".to_string()]);
}

#[test]
fn skills_none_when_not_in_meta_json() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("no-skills");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
    fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false}"#,
        )
        .unwrap();

    let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
    assert!(agents[0].skills.is_none());
}

// -----------------------------------------------------------------------
// criticalSystemReminder tests (AGT-022)
// -----------------------------------------------------------------------

#[test]
fn critical_system_reminder_parsed_from_meta_json() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("reminded");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
    fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false,"critical_system_reminder":"NEVER edit files"}"#,
        )
        .unwrap();

    let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
    assert_eq!(
        agents[0].critical_system_reminder.as_deref(),
        Some("NEVER edit files")
    );
}

#[test]
fn critical_system_reminder_none_when_absent() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("no-reminder");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
    fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false}"#,
        )
        .unwrap();

    let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
    assert!(agents[0].critical_system_reminder.is_none());
}

#[test]
fn critical_system_reminder_empty_string_treated_as_none() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("empty-reminder");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
    fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false,"critical_system_reminder":""}"#,
        )
        .unwrap();

    let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
    assert!(agents[0].critical_system_reminder.is_none());
}

#[test]
fn skills_empty_array_parsed_as_some_empty() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("empty-skills");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
    fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false,"skills":[]}"#,
        )
        .unwrap();

    let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
    let skills = agents[0].skills.as_ref().expect("skills should be Some");
    assert!(skills.is_empty());
}

// -----------------------------------------------------------------------
// memory_scope parsing tests (AGT-002)
// -----------------------------------------------------------------------

#[test]
fn memory_scope_parsed_from_memory_keys_json() {
    use crate::agents::definition::AgentMemoryScope;
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("scoped");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
    fs::write(
        agent_dir.join("memory-keys.json"),
        r#"{"recall_queries":[],"leann_queries":[],"tags":[],"memory_scope":"project"}"#,
    )
    .unwrap();

    let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
    assert_eq!(agents[0].memory_scope, Some(AgentMemoryScope::Project));
}

#[test]
fn memory_scope_none_when_absent_in_memory_keys() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("no-scope");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
    fs::write(
        agent_dir.join("memory-keys.json"),
        r#"{"recall_queries":["test"],"leann_queries":[],"tags":[]}"#,
    )
    .unwrap();

    let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
    assert!(agents[0].memory_scope.is_none());
}

#[test]
fn memory_scope_none_when_memory_keys_missing() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("no-keys");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
    // No memory-keys.json at all

    let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
    assert!(agents[0].memory_scope.is_none());
}

#[test]
fn memory_scope_invalid_value_defaults_to_none() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("bad-scope");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
    fs::write(
        agent_dir.join("memory-keys.json"),
        r#"{"recall_queries":[],"leann_queries":[],"tags":[],"memory_scope":"invalid_value"}"#,
    )
    .unwrap();

    let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
    assert!(agents[0].memory_scope.is_none());
}

#[test]
fn memory_scope_all_variants() {
    use crate::agents::definition::AgentMemoryScope;
    for (scope_str, expected) in [
        ("user", AgentMemoryScope::User),
        ("project", AgentMemoryScope::Project),
        ("local", AgentMemoryScope::Local),
    ] {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("scope-test");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
        fs::write(
            agent_dir.join("memory-keys.json"),
            format!(
                r#"{{"recall_queries":[],"leann_queries":[],"tags":[],"memory_scope":"{}"}}"#,
                scope_str
            ),
        )
        .unwrap();

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        assert_eq!(
            agents[0].memory_scope,
            Some(expected),
            "memory_scope '{}' should parse correctly",
            scope_str
        );
    }
}

// -----------------------------------------------------------------------
// Integration tests: load real agents from disk (AGT-002)
// -----------------------------------------------------------------------
