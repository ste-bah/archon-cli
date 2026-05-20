//! Tests for the 46-agent research pipeline definitions.

use archon_pipeline::research::agents::*;
use std::collections::HashSet;

#[test]
fn test_agent_count() {
    assert_eq!(
        RESEARCH_AGENTS.len(),
        46,
        "Research pipeline must have exactly 46 agents"
    );
}

#[test]
fn test_no_duplicate_keys() {
    let mut seen = HashSet::new();
    for agent in RESEARCH_AGENTS.iter() {
        assert!(seen.insert(agent.key), "Duplicate agent key: {}", agent.key);
    }
}

#[test]
fn test_all_agents_have_web_tools() {
    let required = [
        ResearchToolAccess::WebSearch,
        ResearchToolAccess::WebFetch,
        ResearchToolAccess::Read,
        ResearchToolAccess::Glob,
        ResearchToolAccess::Grep,
    ];
    for agent in RESEARCH_AGENTS.iter() {
        for tool in &required {
            assert!(
                agent.tool_access.contains(tool),
                "Agent '{}' missing required tool {:?}",
                agent.key,
                tool
            );
        }
    }
}

#[test]
fn test_writing_agents_have_write() {
    let write_agent_keys: HashSet<&str> = [
        "introduction-writer",
        "literature-review-writer",
        "results-writer",
        "discussion-writer",
        "conclusion-writer",
        "abstract-writer",
        "chapter-synthesizer",
    ]
    .into_iter()
    .collect();

    for agent in RESEARCH_AGENTS.iter() {
        if agent.phase == 6 || agent.phase == 8 {
            assert!(
                write_agent_keys.contains(agent.key),
                "Unexpected writing/final assembly agent: {}",
                agent.key
            );
            assert!(
                agent.tool_access.contains(&ResearchToolAccess::Write),
                "Writing/final assembly agent '{}' must have Write tool access",
                agent.key
            );
        }
    }

    // Also verify non-writing agents do NOT have Write.
    for agent in RESEARCH_AGENTS.iter() {
        if !write_agent_keys.contains(agent.key) {
            assert!(
                !agent.tool_access.contains(&ResearchToolAccess::Write),
                "Non-writing agent '{}' should not have Write tool access",
                agent.key
            );
        }
    }
}

#[test]
fn test_phase_counts() {
    let mut counts = [0u32; 9]; // index 0 unused, 1-8 for phases
    for agent in RESEARCH_AGENTS.iter() {
        assert!(
            (1..=8).contains(&agent.phase),
            "Agent '{}' has invalid phase {}",
            agent.key,
            agent.phase
        );
        counts[agent.phase as usize] += 1;
    }
    assert_eq!(counts[1], 6, "Phase 1 should have 6 agents");
    assert_eq!(counts[2], 4, "Phase 2 should have 4 agents");
    assert_eq!(counts[3], 4, "Phase 3 should have 4 agents");
    assert_eq!(counts[4], 5, "Phase 4 should have 5 agents");
    assert_eq!(counts[5], 9, "Phase 5 should have 9 agents");
    assert_eq!(counts[6], 6, "Phase 6 should have 6 agents");
    assert_eq!(counts[7], 11, "Phase 7 should have 11 agents");
    assert_eq!(counts[8], 1, "Phase 8 should have 1 agent");
}

#[test]
fn test_validate_configuration() {
    assert!(
        validate_configuration().is_ok(),
        "Configuration validation must pass: {:?}",
        validate_configuration()
    );
}

#[test]
fn test_agent_lookup_by_key() {
    let agent = get_agent_by_key("step-back-analyzer");
    assert!(agent.is_some(), "step-back-analyzer must be found");
    let agent = agent.unwrap();
    assert_eq!(agent.key, "step-back-analyzer");
    assert_eq!(agent.display_name, "Step-Back Analyzer");
    assert_eq!(agent.phase, 1);

    // Check a phase 7 agent
    let agent = get_agent_by_key("file-length-manager");
    assert!(agent.is_some(), "file-length-manager must be found");
    let agent = agent.unwrap();
    assert_eq!(agent.phase, 7);

    // Check final assembly
    let agent = get_agent_by_key("chapter-synthesizer");
    assert!(agent.is_some(), "chapter-synthesizer must be found");
    let agent = agent.unwrap();
    assert_eq!(agent.phase, 8);

    // Non-existent key
    assert!(get_agent_by_key("nonexistent").is_none());
}

#[test]
fn test_get_agents_by_phase() {
    assert_eq!(get_agents_by_phase(1).len(), 6);
    assert_eq!(get_agents_by_phase(2).len(), 4);
    assert_eq!(get_agents_by_phase(3).len(), 4);
    assert_eq!(get_agents_by_phase(4).len(), 5);
    assert_eq!(get_agents_by_phase(5).len(), 9);
    assert_eq!(get_agents_by_phase(6).len(), 6);
    assert_eq!(get_agents_by_phase(7).len(), 11);
    assert_eq!(get_agents_by_phase(8).len(), 1);
}

#[test]
fn test_get_agent_index() {
    assert_eq!(get_agent_index("step-back-analyzer"), Some(0));
    assert_eq!(get_agent_index("self-ask-decomposer"), Some(1));
    assert_eq!(get_agent_index("file-length-manager"), Some(44));
    assert_eq!(get_agent_index("chapter-synthesizer"), Some(45));
    assert_eq!(get_agent_index("nonexistent"), None);
}

#[test]
fn test_get_phase_by_id() {
    let phase = get_phase_by_id(1);
    assert!(phase.is_some());
    let phase = phase.unwrap();
    assert_eq!(phase.id, 1);
    assert_eq!(phase.name, "Foundation");
    assert_eq!(phase.agent_keys.len(), 6);

    let phase = get_phase_by_id(7).unwrap();
    assert_eq!(phase.name, "Validation");
    assert_eq!(phase.agent_keys.len(), 11);

    let phase = get_phase_by_id(8).unwrap();
    assert_eq!(phase.name, "Final Assembly");
    assert_eq!(phase.agent_keys, &["chapter-synthesizer"]);

    assert!(get_phase_by_id(0).is_none());
    assert!(get_phase_by_id(9).is_none());
}

#[test]
fn test_memory_keys_non_empty() {
    for agent in RESEARCH_AGENTS.iter() {
        assert!(
            !agent.memory_keys.is_empty(),
            "Agent '{}' must have at least 1 memory key",
            agent.key
        );
    }
}

#[test]
fn test_output_artifacts_non_empty() {
    for agent in RESEARCH_AGENTS.iter() {
        assert!(
            !agent.output_artifacts.is_empty(),
            "Agent '{}' must have at least 1 output artifact",
            agent.key
        );
    }
}

#[test]
fn test_prompt_source_paths() {
    for agent in RESEARCH_AGENTS.iter() {
        let expected = format!(".archon/agents/phdresearch/{}.md", agent.key);
        assert_eq!(
            agent.prompt_source_path, expected,
            "Agent '{}' has wrong prompt_source_path: expected '{}', got '{}'",
            agent.key, expected, agent.prompt_source_path
        );
    }
}

#[test]
fn test_phase_names() {
    let expected = [
        (1, "Foundation"),
        (2, "Discovery"),
        (3, "Architecture"),
        (4, "Synthesis"),
        (5, "Design"),
        (6, "Writing"),
        (7, "Validation"),
        (8, "Final Assembly"),
    ];
    for (id, name) in &expected {
        let phase = get_phase_by_id(*id).unwrap();
        assert_eq!(phase.name, *name);
    }
}

#[test]
fn test_research_phases_count() {
    assert_eq!(RESEARCH_PHASES.len(), 8, "Must have exactly 8 phases");
}

#[test]
fn test_serialization_roundtrip() {
    // Verify agents can be serialized to JSON
    let json = serde_json::to_string(&RESEARCH_AGENTS[0]).unwrap();
    assert!(json.contains("step-back-analyzer"));

    // Verify phases can be serialized
    let json = serde_json::to_string(&RESEARCH_PHASES[0]).unwrap();
    assert!(json.contains("Foundation"));
}
