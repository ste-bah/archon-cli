//! Tests for the 50-agent coding pipeline definitions.

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
        assert!(seen.insert(agent.key), "Duplicate agent key: {}", agent.key);
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
        expected_critical,
        actual_critical,
        "Critical agents mismatch.\nExpected but missing: {:?}\nPresent but not expected: {:?}",
        expected_critical
            .difference(&actual_critical)
            .collect::<Vec<_>>(),
        actual_critical
            .difference(&expected_critical)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        actual_critical.len(),
        20,
        "Must have exactly 20 critical agents"
    );
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
            agent
                .prompt_source_path
                .starts_with(".archon/agents/coding-pipeline/"),
            "Agent '{}' prompt_source_path '{}' must start with '.archon/agents/coding-pipeline/'",
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
    assert_eq!(
        sa.phase,
        Phase::Design,
        "security-architect should be in Design per PRD"
    );

    let ia = get_agent_by_key("integration-architect").unwrap();
    assert_eq!(
        ia.phase,
        Phase::WiringPlan,
        "integration-architect should be in WiringPlan"
    );

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

const EXPECTED_CODING_KEYS: &[&str] = &[
    r###"contract-agent"###,
    r###"requirement-extractor"###,
    r###"requirement-prioritizer"###,
    r###"scope-definer"###,
    r###"context-gatherer"###,
    r###"feasibility-analyzer"###,
    r###"pattern-explorer"###,
    r###"technology-scout"###,
    r###"research-planner"###,
    r###"codebase-analyzer"###,
    r###"phase-1-reviewer"###,
    r###"phase-2-reviewer"###,
    r###"system-designer"###,
    r###"component-designer"###,
    r###"interface-designer"###,
    r###"data-architect"###,
    r###"integration-architect"###,
    r###"wiring-obligation-agent"###,
    r###"phase-3-reviewer"###,
    r###"code-generator"###,
    r###"type-implementer"###,
    r###"unit-implementer"###,
    r###"service-implementer"###,
    r###"data-layer-implementer"###,
    r###"api-implementer"###,
    r###"frontend-implementer"###,
    r###"error-handler-implementer"###,
    r###"config-implementer"###,
    r###"logger-implementer"###,
    r###"integration-verification-agent"###,
    r###"dependency-manager"###,
    r###"implementation-coordinator"###,
    r###"phase-4-reviewer"###,
    r###"test-generator"###,
    r###"test-runner"###,
    r###"integration-tester"###,
    r###"regression-tester"###,
    r###"security-tester"###,
    r###"coverage-analyzer"###,
    r###"quality-gate"###,
    r###"test-fixer"###,
    r###"phase-5-reviewer"###,
    r###"performance-optimizer"###,
    r###"performance-architect"###,
    r###"code-quality-improver"###,
    r###"security-architect"###,
    r###"final-refactorer"###,
    r###"sign-off-approver"###,
    r###"phase-6-reviewer"###,
    r###"recovery-agent"###,
];

const EXPECTED_CODING_JSON: &[&str] = &[
    r###"{"key":"contract-agent","phase":"Understanding","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/contract-agent.md","tool_access":"ReadOnly","algorithm":"ToT","fallback_algorithm":"Reflexion","depends_on":[],"memory_reads":["coding/input/task","coding/context/project"],"memory_writes":["coding/understanding/task-analysis","coding/understanding/parsed-intent"],"xp_reward":50,"parallelizable":false,"critical":true,"description":"Parses and structures coding requests into actionable components. CRITICAL agent - pipeline entry point."}"###,
    r###"{"key":"requirement-extractor","phase":"Understanding","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/requirement-extractor.md","tool_access":"ReadOnly","algorithm":"ToT","fallback_algorithm":"Reflexion","depends_on":["contract-agent"],"memory_reads":["coding/understanding/task-analysis"],"memory_writes":["coding/understanding/requirements","coding/understanding/functional-requirements"],"xp_reward":45,"parallelizable":true,"critical":false,"description":"Extracts functional and non-functional requirements from parsed task analysis."}"###,
    r###"{"key":"requirement-prioritizer","phase":"Understanding","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/requirement-prioritizer.md","tool_access":"ReadOnly","algorithm":"PoT","fallback_algorithm":"ReAct","depends_on":["requirement-extractor"],"memory_reads":["coding/understanding/requirements"],"memory_writes":["coding/understanding/prioritized-requirements"],"xp_reward":40,"parallelizable":false,"critical":false,"description":"Applies MoSCoW prioritization to requirements, enabling focused delivery."}"###,
    r###"{"key":"scope-definer","phase":"Understanding","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/scope-definer.md","tool_access":"ReadOnly","algorithm":"ToT","fallback_algorithm":"ReAct","depends_on":["requirement-prioritizer"],"memory_reads":["coding/understanding/prioritized-requirements"],"memory_writes":["coding/understanding/scope","coding/understanding/boundaries"],"xp_reward":45,"parallelizable":false,"critical":false,"description":"Defines clear boundaries, deliverables, and milestones for the coding task."}"###,
    r###"{"key":"context-gatherer","phase":"Understanding","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/context-gatherer.md","tool_access":"ReadOnly","algorithm":"ReAct","fallback_algorithm":"Reflexion","depends_on":["contract-agent"],"memory_reads":["coding/understanding/task-analysis","coding/context/project"],"memory_writes":["coding/understanding/context","coding/understanding/existing-code"],"xp_reward":45,"parallelizable":true,"critical":false,"description":"Gathers codebase context via LEANN semantic search. Produces EvidencePack JSON with file:line evidence for every claim."}"###,
    r###"{"key":"feasibility-analyzer","phase":"Design","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/feasibility-analyzer.md","tool_access":"ReadOnly","algorithm":"PoT","fallback_algorithm":"ReAct","depends_on":["scope-definer","context-gatherer"],"memory_reads":["coding/understanding/scope","coding/understanding/context"],"memory_writes":["coding/understanding/feasibility","coding/understanding/constraints"],"xp_reward":50,"parallelizable":false,"critical":true,"description":"Assesses technical, resource, and timeline feasibility of proposed implementation."}"###,
    r###"{"key":"pattern-explorer","phase":"Understanding","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/pattern-explorer.md","tool_access":"ReadOnly","algorithm":"LATS","fallback_algorithm":"ToT","depends_on":["phase-1-reviewer"],"memory_reads":["coding/understanding/requirements","coding/understanding/constraints"],"memory_writes":["coding/exploration/patterns","coding/exploration/best-practices"],"xp_reward":45,"parallelizable":false,"critical":false,"description":"Explores and documents existing code patterns that can guide implementation decisions."}"###,
    r###"{"key":"technology-scout","phase":"Understanding","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/technology-scout.md","tool_access":"ReadOnly","algorithm":"ReAct","fallback_algorithm":"Reflexion","depends_on":["pattern-explorer"],"memory_reads":["coding/exploration/patterns","coding/understanding/requirements"],"memory_writes":["coding/exploration/technologies","coding/exploration/recommendations"],"xp_reward":40,"parallelizable":true,"critical":false,"description":"Evaluates technology options and external solutions that could address implementation needs."}"###,
    r###"{"key":"research-planner","phase":"Design","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/research-planner.md","tool_access":"ReadOnly","algorithm":"ToT","fallback_algorithm":"ReAct","depends_on":["pattern-explorer"],"memory_reads":["coding/exploration/patterns","coding/understanding/scope"],"memory_writes":["coding/exploration/research-plan","coding/exploration/unknowns"],"xp_reward":35,"parallelizable":true,"critical":false,"description":"Creates structured research plans to investigate implementation approaches and unknowns."}"###,
    r###"{"key":"codebase-analyzer","phase":"Understanding","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/codebase-analyzer.md","tool_access":"ReadOnly","algorithm":"ReAct","fallback_algorithm":"Reflexion","depends_on":["technology-scout","research-planner"],"memory_reads":["coding/exploration/technologies","coding/understanding/context"],"memory_writes":["coding/exploration/codebase-analysis","coding/exploration/integration-points"],"xp_reward":50,"parallelizable":false,"critical":false,"description":"Performs deep analysis of relevant code sections to understand implementation context."}"###,
    r###"{"key":"phase-1-reviewer","phase":"Design","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/phase-1-reviewer.md","tool_access":"ReadOnly","algorithm":"Reflexion","fallback_algorithm":"ToT","depends_on":["feasibility-analyzer"],"memory_reads":["coding/understanding/task-analysis","coding/understanding/requirements","coding/understanding/scope","coding/understanding/context","coding/understanding/feasibility"],"memory_writes":["coding/forensic/phase-1-verdict","coding/forensic/phase-1-evidence"],"xp_reward":100,"parallelizable":false,"critical":true,"description":"Sherlock #42: Phase 1 Understanding forensic review. CRITICAL: Gates progression to Phase 2."}"###,
    r###"{"key":"phase-2-reviewer","phase":"Design","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/phase-2-reviewer.md","tool_access":"ReadOnly","algorithm":"Reflexion","fallback_algorithm":"ToT","depends_on":["codebase-analyzer"],"memory_reads":["coding/exploration/patterns","coding/exploration/technologies","coding/exploration/research-plan","coding/exploration/codebase-analysis"],"memory_writes":["coding/forensic/phase-2-verdict","coding/forensic/phase-2-evidence"],"xp_reward":100,"parallelizable":false,"critical":true,"description":"Sherlock #43: Phase 2 Exploration forensic review. CRITICAL: Gates progression to Phase 3."}"###,
    r###"{"key":"system-designer","phase":"Design","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/system-designer.md","tool_access":"ReadOnly","algorithm":"ToT","fallback_algorithm":"LATS","depends_on":["phase-2-reviewer"],"memory_reads":["coding/exploration/codebase-analysis","coding/understanding/requirements"],"memory_writes":["coding/architecture/design","coding/architecture/structure"],"xp_reward":60,"parallelizable":false,"critical":true,"description":"Designs high-level system architecture, module boundaries, and component relationships."}"###,
    r###"{"key":"component-designer","phase":"Design","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/component-designer.md","tool_access":"ReadOnly","algorithm":"ToT","fallback_algorithm":"Reflexion","depends_on":["system-designer"],"memory_reads":["coding/architecture/design"],"memory_writes":["coding/architecture/components","coding/architecture/modules"],"xp_reward":45,"parallelizable":true,"critical":false,"description":"Designs internal component structure, class hierarchies, and implementation details."}"###,
    r###"{"key":"interface-designer","phase":"Design","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/interface-designer.md","tool_access":"ReadOnly","algorithm":"ToT","fallback_algorithm":"Reflexion","depends_on":["component-designer"],"memory_reads":["coding/architecture/components"],"memory_writes":["coding/architecture/interfaces","coding/architecture/contracts"],"xp_reward":50,"parallelizable":true,"critical":true,"description":"Designs API contracts, type definitions, and interface specifications."}"###,
    r###"{"key":"data-architect","phase":"Design","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/data-architect.md","tool_access":"ReadOnly","algorithm":"ReAct","fallback_algorithm":"PoT","depends_on":["component-designer"],"memory_reads":["coding/architecture/components","coding/architecture/interfaces"],"memory_writes":["coding/architecture/data-models","coding/architecture/schemas"],"xp_reward":45,"parallelizable":true,"critical":false,"description":"Designs data models, database schemas, and data persistence strategies."}"###,
    r###"{"key":"integration-architect","phase":"WiringPlan","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/integration-architect.md","tool_access":"ReadOnly","algorithm":"ToT","fallback_algorithm":"Reflexion","depends_on":["interface-designer","data-architect"],"memory_reads":["coding/architecture/interfaces","coding/architecture/data-models"],"memory_writes":["coding/architecture/integrations","coding/architecture/dependencies"],"xp_reward":55,"parallelizable":false,"critical":false,"description":"Designs integration patterns, external API connections, and system interoperability."}"###,
    r###"{"key":"wiring-obligation-agent","phase":"WiringPlan","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/wiring-obligation-agent.md","tool_access":"ReadOnly","algorithm":"ToT","fallback_algorithm":"ReAct","depends_on":["integration-architect"],"memory_reads":["coding/architecture/integrations","coding/architecture/dependencies","coding/contract"],"memory_writes":["coding/wiring-plan"],"xp_reward":60,"parallelizable":false,"critical":true,"description":"Produces WiringPlan with typed obligations before implementation begins. Gates Phase 4."}"###,
    r###"{"key":"phase-3-reviewer","phase":"WiringPlan","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/phase-3-reviewer.md","tool_access":"ReadOnly","algorithm":"Reflexion","fallback_algorithm":"ToT","depends_on":["wiring-obligation-agent"],"memory_reads":["coding/architecture/design","coding/architecture/components","coding/architecture/interfaces","coding/architecture/data-models","coding/architecture/integrations","coding/wiring-plan"],"memory_writes":["coding/forensic/phase-3-verdict","coding/forensic/phase-3-evidence"],"xp_reward":100,"parallelizable":false,"critical":true,"description":"Sherlock #44: Phase 3 Architecture forensic review. CRITICAL: Gates progression to Phase 4."}"###,
    r###"{"key":"code-generator","phase":"Implementation","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/code-generator.md","tool_access":"Full","algorithm":"SelfDebug","fallback_algorithm":"ReAct","depends_on":["phase-3-reviewer"],"memory_reads":["coding/architecture/design","coding/architecture/interfaces"],"memory_writes":["coding/implementation/generated-code","coding/implementation/core-files"],"xp_reward":70,"parallelizable":false,"critical":true,"description":"Generates clean, production-ready code following architecture specifications."}"###,
    r###"{"key":"type-implementer","phase":"Implementation","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/type-implementer.md","tool_access":"Full","algorithm":"SelfDebug","fallback_algorithm":"ReAct","depends_on":["code-generator"],"memory_reads":["coding/architecture/interfaces","coding/implementation/generated-code"],"memory_writes":["coding/implementation/types","coding/implementation/type-files"],"xp_reward":55,"parallelizable":true,"critical":false,"description":"Implements TypeScript type definitions, interfaces, generics, and type utilities."}"###,
    r###"{"key":"unit-implementer","phase":"Implementation","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/unit-implementer.md","tool_access":"Full","algorithm":"SelfDebug","fallback_algorithm":"Reflexion","depends_on":["type-implementer"],"memory_reads":["coding/implementation/types","coding/architecture/components"],"memory_writes":["coding/implementation/units","coding/implementation/entities"],"xp_reward":55,"parallelizable":true,"critical":false,"description":"Implements domain entities, value objects, and core business logic units."}"###,
    r###"{"key":"service-implementer","phase":"Implementation","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/service-implementer.md","tool_access":"Full","algorithm":"LATS","fallback_algorithm":"SelfDebug","depends_on":["unit-implementer"],"memory_reads":["coding/implementation/units","coding/architecture/design"],"memory_writes":["coding/implementation/services","coding/implementation/business-logic"],"xp_reward":60,"parallelizable":false,"critical":false,"description":"Implements domain services, business logic, and application use cases."}"###,
    r###"{"key":"data-layer-implementer","phase":"Implementation","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/data-layer-implementer.md","tool_access":"Full","algorithm":"SelfDebug","fallback_algorithm":"ReAct","depends_on":["unit-implementer"],"memory_reads":["coding/implementation/units","coding/architecture/data-models"],"memory_writes":["coding/implementation/data-layer","coding/implementation/repositories"],"xp_reward":55,"parallelizable":true,"critical":false,"description":"Implements repositories, database access, and data persistence layer."}"###,
    r###"{"key":"api-implementer","phase":"Implementation","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/api-implementer.md","tool_access":"Full","algorithm":"ReAct","fallback_algorithm":"SelfDebug","depends_on":["service-implementer","data-layer-implementer"],"memory_reads":["coding/implementation/services","coding/architecture/interfaces"],"memory_writes":["coding/implementation/api","coding/implementation/endpoints"],"xp_reward":60,"parallelizable":false,"critical":false,"description":"Implements REST/GraphQL API endpoints, controllers, and request validation."}"###,
    r###"{"key":"frontend-implementer","phase":"Implementation","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/frontend-implementer.md","tool_access":"Full","algorithm":"SelfDebug","fallback_algorithm":"ReAct","depends_on":["api-implementer"],"memory_reads":["coding/implementation/api","coding/architecture/components"],"memory_writes":["coding/implementation/frontend","coding/implementation/ui-components"],"xp_reward":55,"parallelizable":true,"critical":false,"description":"Implements UI components, pages, state management, and client-side logic."}"###,
    r###"{"key":"error-handler-implementer","phase":"Implementation","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/error-handler-implementer.md","tool_access":"Full","algorithm":"ReAct","fallback_algorithm":"Reflexion","depends_on":["api-implementer"],"memory_reads":["coding/implementation/api","coding/implementation/services"],"memory_writes":["coding/implementation/error-handling","coding/implementation/exceptions"],"xp_reward":50,"parallelizable":true,"critical":false,"description":"Implements error handling strategies, recovery mechanisms, and error reporting."}"###,
    r###"{"key":"config-implementer","phase":"Implementation","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/config-implementer.md","tool_access":"Full","algorithm":"ReAct","fallback_algorithm":"SelfDebug","depends_on":["frontend-implementer"],"memory_reads":["coding/implementation/api","coding/architecture/dependencies"],"memory_writes":["coding/implementation/config","coding/implementation/settings"],"xp_reward":40,"parallelizable":true,"critical":false,"description":"Implements configuration management, environment handling, and feature flags."}"###,
    r###"{"key":"logger-implementer","phase":"Implementation","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/logger-implementer.md","tool_access":"Full","algorithm":"ReAct","fallback_algorithm":"SelfDebug","depends_on":["error-handler-implementer"],"memory_reads":["coding/implementation/error-handling","coding/implementation/services"],"memory_writes":["coding/implementation/logging","coding/implementation/observability"],"xp_reward":45,"parallelizable":true,"critical":false,"description":"Implements logging infrastructure, log formatting, and observability patterns."}"###,
    r###"{"key":"integration-verification-agent","phase":"Implementation","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/integration-verification-agent.md","tool_access":"ReadOnly","algorithm":"ReAct","fallback_algorithm":null,"depends_on":["logger-implementer"],"memory_reads":["coding/implementation/wiring-plan","coding/implementation/generated-code"],"memory_writes":["coding/implementation/verification-report","coding/implementation/wiring-status"],"xp_reward":60,"parallelizable":false,"critical":true,"description":"Verifies all wiring obligations from the WiringPlan using tool-based checks (Read). Reports per-obligation pass/fail with evidence."}"###,
    r###"{"key":"dependency-manager","phase":"Refinement","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/dependency-manager.md","tool_access":"Full","algorithm":"ReAct","fallback_algorithm":"SelfDebug","depends_on":["config-implementer","logger-implementer"],"memory_reads":["coding/implementation/config","coding/architecture/dependencies"],"memory_writes":["coding/implementation/dependencies","coding/implementation/package-json"],"xp_reward":40,"parallelizable":false,"critical":false,"description":"Manages package dependencies, version resolution, and module organization."}"###,
    r###"{"key":"implementation-coordinator","phase":"Refinement","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/implementation-coordinator.md","tool_access":"Full","algorithm":"Reflexion","fallback_algorithm":"ReAct","depends_on":["dependency-manager"],"memory_reads":["coding/implementation/generated-code","coding/implementation/services","coding/implementation/api"],"memory_writes":["coding/implementation/coordination-report","coding/implementation/integration-status"],"xp_reward":55,"parallelizable":false,"critical":true,"description":"Coordinates implementation across all agents, manages dependencies, and ensures consistency."}"###,
    r###"{"key":"phase-4-reviewer","phase":"Testing","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/phase-4-reviewer.md","tool_access":"Full","algorithm":"Reflexion","fallback_algorithm":"SelfDebug","depends_on":["implementation-coordinator"],"memory_reads":["coding/implementation/generated-code","coding/implementation/types","coding/implementation/services","coding/implementation/api","coding/implementation/coordination-report"],"memory_writes":["coding/forensic/phase-4-verdict","coding/forensic/phase-4-evidence"],"xp_reward":100,"parallelizable":false,"critical":true,"description":"Sherlock #45: Phase 4 Implementation forensic review. CRITICAL: Gates progression to Phase 5."}"###,
    r###"{"key":"test-generator","phase":"Testing","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/test-generator.md","tool_access":"Full","algorithm":"ToT","fallback_algorithm":"SelfDebug","depends_on":["phase-4-reviewer"],"memory_reads":["coding/implementation/services","coding/understanding/requirements"],"memory_writes":["coding/testing/generated-tests","coding/testing/test-files"],"xp_reward":55,"parallelizable":false,"critical":false,"description":"Generates comprehensive test suites including unit, integration, and e2e tests."}"###,
    r###"{"key":"test-runner","phase":"Testing","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/test-runner.md","tool_access":"Full","algorithm":"ReAct","fallback_algorithm":"SelfDebug","depends_on":["test-generator"],"memory_reads":["coding/testing/generated-tests","coding/implementation/services"],"memory_writes":["coding/testing/results","coding/testing/failures"],"xp_reward":50,"parallelizable":false,"critical":true,"description":"Orchestrates and executes all test suites, managing test lifecycle and reporting results."}"###,
    r###"{"key":"integration-tester","phase":"Testing","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/integration-tester.md","tool_access":"Full","algorithm":"SelfDebug","fallback_algorithm":"ReAct","depends_on":["test-runner"],"memory_reads":["coding/testing/results","coding/implementation/api"],"memory_writes":["coding/testing/integration-tests","coding/testing/integration-results"],"xp_reward":55,"parallelizable":true,"critical":false,"description":"Creates and executes integration tests verifying component interactions and system behavior."}"###,
    r###"{"key":"regression-tester","phase":"Testing","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/regression-tester.md","tool_access":"Full","algorithm":"Reflexion","fallback_algorithm":"SelfDebug","depends_on":["test-runner"],"memory_reads":["coding/testing/results","coding/understanding/context"],"memory_writes":["coding/testing/regression-tests","coding/testing/breaking-changes"],"xp_reward":50,"parallelizable":true,"critical":false,"description":"Performs regression testing to detect unintended changes and compares against baselines."}"###,
    r###"{"key":"security-tester","phase":"Testing","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/security-tester.md","tool_access":"Full","algorithm":"ReAct","fallback_algorithm":"Reflexion","depends_on":["integration-tester"],"memory_reads":["coding/testing/integration-results","coding/implementation/api"],"memory_writes":["coding/testing/security-tests","coding/testing/vulnerabilities"],"xp_reward":60,"parallelizable":true,"critical":true,"description":"Performs security testing including vulnerability scanning and compliance verification."}"###,
    r###"{"key":"coverage-analyzer","phase":"Testing","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/coverage-analyzer.md","tool_access":"Full","algorithm":"PoT","fallback_algorithm":"ReAct","depends_on":["regression-tester","security-tester"],"memory_reads":["coding/testing/results","coding/testing/integration-results"],"memory_writes":["coding/testing/coverage-report","coding/testing/coverage-gaps"],"xp_reward":50,"parallelizable":false,"critical":false,"description":"Analyzes test coverage metrics, identifies gaps, and generates coverage reports."}"###,
    r###"{"key":"quality-gate","phase":"Refinement","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/quality-gate.md","tool_access":"Full","algorithm":"Reflexion","fallback_algorithm":"PoT","depends_on":["coverage-analyzer"],"memory_reads":["coding/testing/coverage-report","coding/testing/results"],"memory_writes":["coding/testing/quality-verdict","coding/testing/l-score"],"xp_reward":65,"parallelizable":false,"critical":true,"description":"Validates code against quality gates, computes L-Scores, and determines phase completion."}"###,
    r###"{"key":"test-fixer","phase":"Testing","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/test-fixer.md","tool_access":"Full","algorithm":"SelfDebug","fallback_algorithm":"Reflexion","depends_on":["quality-gate"],"memory_reads":["coding/testing/results","coding/testing/failures","coding/testing/quality-verdict"],"memory_writes":["coding/testing/fix-attempts","coding/testing/final-status"],"xp_reward":65,"parallelizable":false,"critical":false,"description":"Self-correction loop: reads test failures, fixes code, re-tests until pass (max 3 retries). Escalates unfixable failures."}"###,
    r###"{"key":"phase-5-reviewer","phase":"Testing","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/phase-5-reviewer.md","tool_access":"Full","algorithm":"Reflexion","fallback_algorithm":"SelfDebug","depends_on":["test-fixer"],"memory_reads":["coding/testing/generated-tests","coding/testing/results","coding/testing/coverage-report","coding/testing/quality-verdict"],"memory_writes":["coding/forensic/phase-5-verdict","coding/forensic/phase-5-evidence"],"xp_reward":100,"parallelizable":false,"critical":true,"description":"Sherlock #46: Phase 5 Testing forensic review. CRITICAL: Gates progression to Phase 6."}"###,
    r###"{"key":"performance-optimizer","phase":"Refinement","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/performance-optimizer.md","tool_access":"Full","algorithm":"PoT","fallback_algorithm":"Reflexion","depends_on":["phase-5-reviewer"],"memory_reads":["coding/implementation/services","coding/testing/results"],"memory_writes":["coding/optimization/performance","coding/optimization/benchmarks"],"xp_reward":60,"parallelizable":false,"critical":false,"description":"Identifies and optimizes performance bottlenecks, memory usage, and runtime efficiency."}"###,
    r###"{"key":"performance-architect","phase":"Design","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/performance-architect.md","tool_access":"ReadOnly","algorithm":"ToT","fallback_algorithm":"ReAct","depends_on":["performance-optimizer"],"memory_reads":["coding/optimization/performance","coding/architecture/design"],"memory_writes":["coding/optimization/architecture-improvements","coding/optimization/scalability"],"xp_reward":55,"parallelizable":true,"critical":false,"description":"Designs performance architecture, optimization strategies, and scalability patterns."}"###,
    r###"{"key":"code-quality-improver","phase":"Refinement","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/code-quality-improver.md","tool_access":"Full","algorithm":"Reflexion","fallback_algorithm":"ReAct","depends_on":["performance-optimizer"],"memory_reads":["coding/implementation/services","coding/testing/quality-verdict"],"memory_writes":["coding/optimization/quality-improvements","coding/optimization/refactoring"],"xp_reward":50,"parallelizable":true,"critical":false,"description":"Improves code quality through refactoring, pattern application, and maintainability enhancements."}"###,
    r###"{"key":"security-architect","phase":"Design","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/security-architect.md","tool_access":"ReadOnly","algorithm":"ReAct","fallback_algorithm":"Reflexion","depends_on":["performance-architect","code-quality-improver"],"memory_reads":["coding/testing/vulnerabilities","coding/implementation/api"],"memory_writes":["coding/optimization/security-improvements","coding/optimization/security-audit"],"xp_reward":60,"parallelizable":false,"critical":true,"description":"Designs security architecture, authentication flows, and threat mitigation strategies."}"###,
    r###"{"key":"final-refactorer","phase":"Refinement","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/final-refactorer.md","tool_access":"Full","algorithm":"Reflexion","fallback_algorithm":"SelfDebug","depends_on":["security-architect"],"memory_reads":["coding/optimization/quality-improvements","coding/optimization/security-audit"],"memory_writes":["coding/optimization/final-code","coding/optimization/polish-report"],"xp_reward":55,"parallelizable":false,"critical":false,"description":"Performs final code polish, consistency checks, and prepares code for delivery."}"###,
    r###"{"key":"sign-off-approver","phase":"Refinement","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/sign-off-approver.md","tool_access":"Full","algorithm":"Reflexion","fallback_algorithm":"ReAct","depends_on":["phase-6-reviewer"],"memory_reads":["coding/optimization/final-code","coding/testing/coverage-report","coding/testing/quality-verdict"],"memory_writes":["coding/delivery/sign-off","coding/delivery/approval-status"],"xp_reward":75,"parallelizable":false,"critical":true,"description":"Final sign-off authority for code delivery, verifying all requirements met. CRITICAL: Must pass for pipeline completion."}"###,
    r###"{"key":"phase-6-reviewer","phase":"Refinement","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/phase-6-reviewer.md","tool_access":"Full","algorithm":"Reflexion","fallback_algorithm":"ToT","depends_on":["final-refactorer"],"memory_reads":["coding/optimization/performance","coding/optimization/quality-improvements","coding/optimization/security-audit","coding/optimization/final-code"],"memory_writes":["coding/forensic/phase-6-verdict","coding/forensic/phase-6-evidence"],"xp_reward":100,"parallelizable":false,"critical":true,"description":"Sherlock #47: Phase 6 Optimization forensic review. CRITICAL: Gates progression to Phase 7."}"###,
    r###"{"key":"recovery-agent","phase":"Refinement","model":"sonnet","prompt_source_path":".archon/agents/coding-pipeline/recovery-agent.md","tool_access":"Full","algorithm":"Reflexion","fallback_algorithm":"LATS","depends_on":["sign-off-approver"],"memory_reads":["coding/delivery/sign-off","coding/forensic/phase-1-verdict","coding/forensic/phase-2-verdict","coding/forensic/phase-3-verdict","coding/forensic/phase-4-verdict","coding/forensic/phase-5-verdict","coding/forensic/phase-6-verdict","coding/pipeline/feedback-status","coding/pipeline/status"],"memory_writes":["coding/forensic/phase-7-verdict","coding/forensic/final-report","coding/forensic/recovery-plan","coding/forensic/feedback-gate-result"],"xp_reward":150,"parallelizable":false,"critical":true,"description":"Sherlock #48: Phase 7 Delivery forensic review, recovery orchestration, and MANDATORY feedback gate enforcement. CRITICAL: Final pipeline gate - verifies learning loop closure."}"###,
];

#[test]
fn coding_definitions_match_pre_split_baseline() {
    let keys = AGENTS.iter().map(|agent| agent.key).collect::<Vec<_>>();
    assert_eq!(keys.as_slice(), EXPECTED_CODING_KEYS);

    let serialized = AGENTS
        .iter()
        .map(|agent| serde_json::to_string(agent).expect("serialize coding agent"))
        .collect::<Vec<_>>();
    assert_eq!(serialized.as_slice(), EXPECTED_CODING_JSON);
}
