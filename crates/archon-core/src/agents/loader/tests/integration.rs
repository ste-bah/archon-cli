use super::*;

#[test]
fn loads_all_9_real_agents_from_custom_dir() {
    let custom_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.archon/agents/custom");
    if !custom_dir.exists() {
        eprintln!(
            "Skipping: .archon/agents/custom/ not found at {:?}",
            custom_dir
        );
        return;
    }

    let agents = load_custom_agents(&custom_dir, AgentSource::Project).unwrap();

    // Should load at least 9 agents (may be more if new ones added)
    assert!(
        agents.len() >= 9,
        "Expected at least 9 agents, got {}",
        agents.len()
    );

    // All agents should have non-empty agent_type
    for agent in &agents {
        assert!(
            !agent.agent_type.is_empty(),
            "agent_type should not be empty"
        );
    }

    // None should start with '_'
    for agent in &agents {
        assert!(
            !agent.agent_type.starts_with('_'),
            "agent '{}' starts with _ and should have been skipped",
            agent.agent_type
        );
    }
}

#[test]
fn code_reviewer_loads_with_nonempty_fields() {
    let custom_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.archon/agents/custom");
    if !custom_dir.exists() {
        eprintln!(
            "Skipping: .archon/agents/custom/ not found at {:?}",
            custom_dir
        );
        return;
    }

    let agents = load_custom_agents(&custom_dir, AgentSource::Project).unwrap();
    let reviewer = agents
        .iter()
        .find(|a| a.agent_type == "code-reviewer")
        .expect("code-reviewer agent should exist");

    assert!(
        !reviewer.system_prompt.is_empty(),
        "code-reviewer system_prompt should be non-empty"
    );
    assert!(
        !reviewer.description.is_empty(),
        "code-reviewer description should be non-empty"
    );
    assert!(
        reviewer.allowed_tools.is_some(),
        "code-reviewer should have explicit allowed_tools"
    );
    let tools = reviewer.allowed_tools.as_ref().unwrap();
    assert!(
        !tools.is_empty(),
        "code-reviewer allowed_tools should not be empty"
    );
}

// -----------------------------------------------------------------------
// Tool guidance extraction tests (AGT-002)
// -----------------------------------------------------------------------

#[test]
fn extract_tool_guidance_from_tools_md() {
    let md = "# Tool Workflow\n\nAlways read before editing.\n\n## Primary Tools\n- **Read**: read\n- **Edit**: edit\n\n## Usage Notes\nBe careful with Edit.\n";
    let guidance = extract_tool_guidance(md);
    assert!(guidance.contains("Always read before editing."));
    assert!(guidance.contains("Be careful with Edit."));
    assert!(!guidance.contains("- **Read**"));
}

#[test]
fn extract_tool_guidance_empty_tools_md() {
    assert!(extract_tool_guidance("").is_empty());
    assert!(extract_tool_guidance("   \n  ").is_empty());
}
