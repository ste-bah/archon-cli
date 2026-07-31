use super::super::{Algorithm, CodingAgent, Phase, ToolAccess};

pub(in super::super) const FEASIBILITY_ANALYZER: CodingAgent = CodingAgent {
    key: "feasibility-analyzer",
    phase: Phase::Design,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/feasibility-analyzer.md",
    tool_access: ToolAccess::ReadOnly,
    algorithm: Algorithm::PoT,
    fallback_algorithm: Some(Algorithm::ReAct),
    depends_on: &["scope-definer", "context-gatherer"],
    memory_reads: &["coding/understanding/scope", "coding/understanding/context"],
    memory_writes: &[
        "coding/understanding/feasibility",
        "coding/understanding/constraints",
    ],
    xp_reward: 50,
    parallelizable: false,
    critical: true,
    description: "Assesses technical, resource, and timeline feasibility of proposed implementation.",
};

pub(in super::super) const RESEARCH_PLANNER: CodingAgent = CodingAgent {
    key: "research-planner",
    phase: Phase::Design,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/research-planner.md",
    tool_access: ToolAccess::ReadOnly,
    algorithm: Algorithm::ToT,
    fallback_algorithm: Some(Algorithm::ReAct),
    depends_on: &["pattern-explorer"],
    memory_reads: &["coding/exploration/patterns", "coding/understanding/scope"],
    memory_writes: &[
        "coding/exploration/research-plan",
        "coding/exploration/unknowns",
    ],
    xp_reward: 35,
    parallelizable: true,
    critical: false,
    description: "Creates structured research plans to investigate implementation approaches and unknowns.",
};

pub(in super::super) const PHASE_1_REVIEWER: CodingAgent = CodingAgent {
    key: "phase-1-reviewer",
    phase: Phase::Design,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/phase-1-reviewer.md",
    tool_access: ToolAccess::ReadOnly,
    algorithm: Algorithm::Reflexion,
    fallback_algorithm: Some(Algorithm::ToT),
    depends_on: &["feasibility-analyzer"],
    memory_reads: &[
        "coding/understanding/task-analysis",
        "coding/understanding/requirements",
        "coding/understanding/scope",
        "coding/understanding/context",
        "coding/understanding/feasibility",
    ],
    memory_writes: &[
        "coding/forensic/phase-1-verdict",
        "coding/forensic/phase-1-evidence",
    ],
    xp_reward: 100,
    parallelizable: false,
    critical: true,
    description: "Sherlock #42: Phase 1 Understanding forensic review. CRITICAL: Gates progression to Phase 2.",
};

pub(in super::super) const PHASE_2_REVIEWER: CodingAgent = CodingAgent {
    key: "phase-2-reviewer",
    phase: Phase::Design,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/phase-2-reviewer.md",
    tool_access: ToolAccess::ReadOnly,
    algorithm: Algorithm::Reflexion,
    fallback_algorithm: Some(Algorithm::ToT),
    depends_on: &["codebase-analyzer"],
    memory_reads: &[
        "coding/exploration/patterns",
        "coding/exploration/technologies",
        "coding/exploration/research-plan",
        "coding/exploration/codebase-analysis",
    ],
    memory_writes: &[
        "coding/forensic/phase-2-verdict",
        "coding/forensic/phase-2-evidence",
    ],
    xp_reward: 100,
    parallelizable: false,
    critical: true,
    description: "Sherlock #43: Phase 2 Exploration forensic review. CRITICAL: Gates progression to Phase 3.",
};

pub(in super::super) const SYSTEM_DESIGNER: CodingAgent = CodingAgent {
    key: "system-designer",
    phase: Phase::Design,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/system-designer.md",
    tool_access: ToolAccess::ReadOnly,
    algorithm: Algorithm::ToT,
    fallback_algorithm: Some(Algorithm::LATS),
    depends_on: &["phase-2-reviewer"],
    memory_reads: &[
        "coding/exploration/codebase-analysis",
        "coding/understanding/requirements",
    ],
    memory_writes: &[
        "coding/architecture/design",
        "coding/architecture/structure",
    ],
    xp_reward: 60,
    parallelizable: false,
    critical: true,
    description: "Designs high-level system architecture, module boundaries, and component relationships.",
};

pub(in super::super) const COMPONENT_DESIGNER: CodingAgent = CodingAgent {
    key: "component-designer",
    phase: Phase::Design,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/component-designer.md",
    tool_access: ToolAccess::ReadOnly,
    algorithm: Algorithm::ToT,
    fallback_algorithm: Some(Algorithm::Reflexion),
    depends_on: &["system-designer"],
    memory_reads: &["coding/architecture/design"],
    memory_writes: &[
        "coding/architecture/components",
        "coding/architecture/modules",
    ],
    xp_reward: 45,
    parallelizable: true,
    critical: false,
    description: "Designs internal component structure, class hierarchies, and implementation details.",
};

pub(in super::super) const INTERFACE_DESIGNER: CodingAgent = CodingAgent {
    key: "interface-designer",
    phase: Phase::Design,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/interface-designer.md",
    tool_access: ToolAccess::ReadOnly,
    algorithm: Algorithm::ToT,
    fallback_algorithm: Some(Algorithm::Reflexion),
    depends_on: &["component-designer"],
    memory_reads: &["coding/architecture/components"],
    memory_writes: &[
        "coding/architecture/interfaces",
        "coding/architecture/contracts",
    ],
    xp_reward: 50,
    parallelizable: true,
    critical: true,
    description: "Designs API contracts, type definitions, and interface specifications.",
};

pub(in super::super) const DATA_ARCHITECT: CodingAgent = CodingAgent {
    key: "data-architect",
    phase: Phase::Design,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/data-architect.md",
    tool_access: ToolAccess::ReadOnly,
    algorithm: Algorithm::ReAct,
    fallback_algorithm: Some(Algorithm::PoT),
    depends_on: &["component-designer"],
    memory_reads: &[
        "coding/architecture/components",
        "coding/architecture/interfaces",
    ],
    memory_writes: &[
        "coding/architecture/data-models",
        "coding/architecture/schemas",
    ],
    xp_reward: 45,
    parallelizable: true,
    critical: false,
    description: "Designs data models, database schemas, and data persistence strategies.",
};

pub(in super::super) const PERFORMANCE_ARCHITECT: CodingAgent = CodingAgent {
    key: "performance-architect",
    phase: Phase::Design,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/performance-architect.md",
    tool_access: ToolAccess::ReadOnly,
    algorithm: Algorithm::ToT,
    fallback_algorithm: Some(Algorithm::ReAct),
    depends_on: &["performance-optimizer"],
    memory_reads: &[
        "coding/optimization/performance",
        "coding/architecture/design",
    ],
    memory_writes: &[
        "coding/optimization/architecture-improvements",
        "coding/optimization/scalability",
    ],
    xp_reward: 55,
    parallelizable: true,
    critical: false,
    description: "Designs performance architecture, optimization strategies, and scalability patterns.",
};

pub(in super::super) const SECURITY_ARCHITECT: CodingAgent = CodingAgent {
    key: "security-architect",
    phase: Phase::Design,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/security-architect.md",
    tool_access: ToolAccess::ReadOnly,
    algorithm: Algorithm::ReAct,
    fallback_algorithm: Some(Algorithm::Reflexion),
    depends_on: &["performance-architect", "code-quality-improver"],
    memory_reads: &[
        "coding/testing/vulnerabilities",
        "coding/implementation/api",
    ],
    memory_writes: &[
        "coding/optimization/security-improvements",
        "coding/optimization/security-audit",
    ],
    xp_reward: 60,
    parallelizable: false,
    critical: true,
    description: "Designs security architecture, authentication flows, and threat mitigation strategies.",
};
