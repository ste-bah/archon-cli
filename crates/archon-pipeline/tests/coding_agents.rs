//! Tests for the 48-agent coding pipeline definitions.

use archon_pipeline::coding::agents::*;
use std::collections::HashSet;

#[test]
fn test_agents_count() {
    assert_eq!(AGENTS.len(), 50, "Pipeline must have exactly 50 agents");
}

#[test]
fn test_no_duplicate_keys() {
    let mut seen = HashSet::new();
    for agent in AGENTS.iter() {
        assert!(
            seen.insert(agent.key),
            "Duplicate agent key: {}",
            agent.key
        );
    }
}

#[test]
fn test_phase_1_2_3_read_only() {
    for agent in AGENTS.iter() {
        match agent.phase {
            Phase::Understanding | Phase::Design | Phase::WiringPlan => {
                assert_eq!(
                    agent.tool_access,
                    ToolAccess::ReadOnly,
                    "Agent '{}' in phase {:?} must have ReadOnly tool access",
                    agent.key,
                    agent.phase
                );
            }
            _ => {}
        }
    }
}

#[test]
fn test_phase_4_5_6_full() {
    for agent in AGENTS.iter() {
        match agent.phase {
            Phase::Implementation | Phase::Testing | Phase::Refinement => {
                // integration-verification-agent is ReadOnly by design (REQ-IMPROVE-004):
                // it only reads files to verify wiring, never writes.
                if agent.key == "integration-verification-agent" {
                    assert_eq!(
                        agent.tool_access,
                        ToolAccess::ReadOnly,
                        "integration-verification-agent must have ReadOnly access"
                    );
                } else {
                    assert_eq!(
                        agent.tool_access,
                        ToolAccess::Full,
                        "Agent '{}' in phase {:?} must have Full tool access",
                        agent.key,
                        agent.phase
                    );
                }
            }
            _ => {}
        }
    }
}

#[test]
fn test_critical_agents() {
    let expected_critical: HashSet<&str> = [
        "contract-agent",
        "interface-designer",
        "quality-gate",
        "sign-off-approver",
        "phase-1-reviewer",
        "phase-2-reviewer",
        "phase-3-reviewer",
        "phase-4-reviewer",
        "phase-5-reviewer",
        "phase-6-reviewer",
        "recovery-agent",
        "system-designer",
        "code-generator",
        "implementation-coordinator",
        "test-runner",
        "security-tester",
        "security-architect",
        "feasibility-analyzer",
        "integration-verification-agent",
        "wiring-obligation-agent",
    ]
    .into_iter()
    .collect();

    let actual_critical: HashSet<&str> = AGENTS
        .iter()
        .filter(|a| a.critical)
        .map(|a| a.key)
        .collect();

    assert_eq!(
        expected_critical, actual_critical,
        "Critical agents mismatch.\nExpected but missing: {:?}\nPresent but not expected: {:?}",
        expected_critical.difference(&actual_critical).collect::<Vec<_>>(),
        actual_critical.difference(&expected_critical).collect::<Vec<_>>()
    );
    assert_eq!(actual_critical.len(), 20, "Must have exactly 20 critical agents");
}

#[test]
fn test_all_agents_have_prompt_path() {
    for agent in AGENTS.iter() {
        assert!(
            !agent.prompt_source_path.is_empty(),
            "Agent '{}' has empty prompt_source_path",
            agent.key
        );
        assert!(
            agent.prompt_source_path.starts_with(".claude/agents/coding-pipeline/"),
            "Agent '{}' prompt_source_path '{}' must start with '.claude/agents/coding-pipeline/'",
            agent.key,
            agent.prompt_source_path
        );
    }
}

#[test]
fn test_all_agents_have_description() {
    for agent in AGENTS.iter() {
        assert!(
            !agent.description.is_empty(),
            "Agent '{}' has empty description",
            agent.key
        );
    }
}

#[test]
fn test_phase_distribution() {
    let mut counts = std::collections::HashMap::new();
    for agent in AGENTS.iter() {
        *counts.entry(agent.phase).or_insert(0u32) += 1;
    }

    assert_eq!(
        counts.get(&Phase::Understanding).copied().unwrap_or(0),
        8,
        "Phase::Understanding should have 8 agents"
    );
    assert_eq!(
        counts.get(&Phase::Design).copied().unwrap_or(0),
        10,
        "Phase::Design should have 10 agents"
    );
    assert_eq!(
        counts.get(&Phase::WiringPlan).copied().unwrap_or(0),
        3,
        "Phase::WiringPlan should have 3 agents"
    );
    assert_eq!(
        counts.get(&Phase::Implementation).copied().unwrap_or(0),
        11,
        "Phase::Implementation should have 11 agents"
    );
    assert_eq!(
        counts.get(&Phase::Testing).copied().unwrap_or(0),
        9,
        "Phase::Testing should have 9 agents"
    );
    assert_eq!(
        counts.get(&Phase::Refinement).copied().unwrap_or(0),
        9,
        "Phase::Refinement should have 9 agents"
    );

    let total: u32 = counts.values().sum();
    assert_eq!(total, 50, "Total agents across all phases must be 50");
}

#[test]
fn test_phase_enum_has_six_variants() {
    // PRD REQ-CODE-007 requires exactly 6 phases
    let all_phases = [
        Phase::Understanding,
        Phase::Design,
        Phase::WiringPlan,
        Phase::Implementation,
        Phase::Testing,
        Phase::Refinement,
    ];
    assert_eq!(all_phases.len(), 6, "Must have exactly 6 phase variants");
    // Verify discriminants
    assert_eq!(Phase::Understanding as u8, 1);
    assert_eq!(Phase::Design as u8, 2);
    assert_eq!(Phase::WiringPlan as u8, 3);
    assert_eq!(Phase::Implementation as u8, 4);
    assert_eq!(Phase::Testing as u8, 5);
    assert_eq!(Phase::Refinement as u8, 6);
}

#[test]
fn test_get_agent_by_key() {
    let agent = get_agent_by_key("contract-agent");
    assert!(agent.is_some(), "contract-agent must exist");
    let agent = agent.unwrap();
    assert_eq!(agent.key, "contract-agent");
    assert_eq!(agent.phase, Phase::Understanding);
    assert!(agent.critical);

    // Verify PRD phase reassignments
    let sa = get_agent_by_key("security-architect").unwrap();
    assert_eq!(sa.phase, Phase::Design, "security-architect should be in Design per PRD");

    let ia = get_agent_by_key("integration-architect").unwrap();
    assert_eq!(ia.phase, Phase::WiringPlan, "integration-architect should be in WiringPlan");

    assert!(get_agent_by_key("nonexistent-agent").is_none());
}

#[test]
fn test_get_agents_by_phase() {
    assert_eq!(get_agents_by_phase(Phase::Understanding).len(), 8);
    assert_eq!(get_agents_by_phase(Phase::Design).len(), 10);
    assert_eq!(get_agents_by_phase(Phase::WiringPlan).len(), 3);
    assert_eq!(get_agents_by_phase(Phase::Implementation).len(), 11);
    assert_eq!(get_agents_by_phase(Phase::Testing).len(), 9);
    assert_eq!(get_agents_by_phase(Phase::Refinement).len(), 9);
}

#[test]
fn test_agent_count() {
    assert_eq!(agent_count(), 50);
}

#[test]
fn test_serialization_roundtrip() {
    let agent = get_agent_by_key("contract-agent").unwrap();
    let json = serde_json::to_string(agent).expect("serialize");
    let deser: CodingAgent = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(deser.key, agent.key);
    assert_eq!(deser.phase, agent.phase);
    assert_eq!(deser.algorithm, agent.algorithm);
}
