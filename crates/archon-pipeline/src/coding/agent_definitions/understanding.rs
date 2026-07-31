use super::super::{Algorithm, CodingAgent, Phase, ToolAccess};

pub(super) const CONTRACT_AGENT: CodingAgent = CodingAgent {
        key: "contract-agent",
        phase: Phase::Understanding,
        model: "sonnet",
        prompt_source_path: ".archon/agents/coding-pipeline/contract-agent.md",
        tool_access: ToolAccess::ReadOnly,
        algorithm: Algorithm::ToT,
        fallback_algorithm: Some(Algorithm::Reflexion),
        depends_on: &[],
        memory_reads: &["coding/input/task", "coding/context/project"],
        memory_writes: &[
            "coding/understanding/task-analysis",
            "coding/understanding/parsed-intent",
        ],
        xp_reward: 50,
        parallelizable: false,
        critical: true,
        description: "Parses and structures coding requests into actionable components. CRITICAL agent - pipeline entry point.",
    };

pub(super) const REQUIREMENT_EXTRACTOR: CodingAgent = CodingAgent {
        key: "requirement-extractor",
        phase: Phase::Understanding,
        model: "sonnet",
        prompt_source_path: ".archon/agents/coding-pipeline/requirement-extractor.md",
        tool_access: ToolAccess::ReadOnly,
        algorithm: Algorithm::ToT,
        fallback_algorithm: Some(Algorithm::Reflexion),
        depends_on: &["contract-agent"],
        memory_reads: &["coding/understanding/task-analysis"],
        memory_writes: &[
            "coding/understanding/requirements",
            "coding/understanding/functional-requirements",
        ],
        xp_reward: 45,
        parallelizable: true,
        critical: false,
        description: "Extracts functional and non-functional requirements from parsed task analysis.",
    };

pub(super) const REQUIREMENT_PRIORITIZER: CodingAgent = CodingAgent {
        key: "requirement-prioritizer",
        phase: Phase::Understanding,
        model: "sonnet",
        prompt_source_path: ".archon/agents/coding-pipeline/requirement-prioritizer.md",
        tool_access: ToolAccess::ReadOnly,
        algorithm: Algorithm::PoT,
        fallback_algorithm: Some(Algorithm::ReAct),
        depends_on: &["requirement-extractor"],
        memory_reads: &["coding/understanding/requirements"],
        memory_writes: &["coding/understanding/prioritized-requirements"],
        xp_reward: 40,
        parallelizable: false,
        critical: false,
        description: "Applies MoSCoW prioritization to requirements, enabling focused delivery.",
    };

pub(super) const SCOPE_DEFINER: CodingAgent = CodingAgent {
        key: "scope-definer",
        phase: Phase::Understanding,
        model: "sonnet",
        prompt_source_path: ".archon/agents/coding-pipeline/scope-definer.md",
        tool_access: ToolAccess::ReadOnly,
        algorithm: Algorithm::ToT,
        fallback_algorithm: Some(Algorithm::ReAct),
        depends_on: &["requirement-prioritizer"],
        memory_reads: &["coding/understanding/prioritized-requirements"],
        memory_writes: &[
            "coding/understanding/scope",
            "coding/understanding/boundaries",
        ],
        xp_reward: 45,
        parallelizable: false,
        critical: false,
        description: "Defines clear boundaries, deliverables, and milestones for the coding task.",
    };

pub(super) const CONTEXT_GATHERER: CodingAgent = CodingAgent {
        key: "context-gatherer",
        phase: Phase::Understanding,
        model: "sonnet",
        prompt_source_path: ".archon/agents/coding-pipeline/context-gatherer.md",
        tool_access: ToolAccess::ReadOnly,
        algorithm: Algorithm::ReAct,
        fallback_algorithm: Some(Algorithm::Reflexion),
        depends_on: &["contract-agent"],
        memory_reads: &[
            "coding/understanding/task-analysis",
            "coding/context/project",
        ],
        memory_writes: &[
            "coding/understanding/context",
            "coding/understanding/existing-code",
        ],
        xp_reward: 45,
        parallelizable: true,
        critical: false,
        description: "Gathers codebase context via LEANN semantic search. Produces EvidencePack JSON with file:line evidence for every claim.",
    };

pub(super) const PATTERN_EXPLORER: CodingAgent = CodingAgent {
        key: "pattern-explorer",
        phase: Phase::Understanding,
        model: "sonnet",
        prompt_source_path: ".archon/agents/coding-pipeline/pattern-explorer.md",
        tool_access: ToolAccess::ReadOnly,
        algorithm: Algorithm::LATS,
        fallback_algorithm: Some(Algorithm::ToT),
        depends_on: &["phase-1-reviewer"],
        memory_reads: &[
            "coding/understanding/requirements",
            "coding/understanding/constraints",
        ],
        memory_writes: &[
            "coding/exploration/patterns",
            "coding/exploration/best-practices",
        ],
        xp_reward: 45,
        parallelizable: false,
        critical: false,
        description: "Explores and documents existing code patterns that can guide implementation decisions.",
    };

pub(super) const TECHNOLOGY_SCOUT: CodingAgent = CodingAgent {
        key: "technology-scout",
        phase: Phase::Understanding,
        model: "sonnet",
        prompt_source_path: ".archon/agents/coding-pipeline/technology-scout.md",
        tool_access: ToolAccess::ReadOnly,
        algorithm: Algorithm::ReAct,
        fallback_algorithm: Some(Algorithm::Reflexion),
        depends_on: &["pattern-explorer"],
        memory_reads: &[
            "coding/exploration/patterns",
            "coding/understanding/requirements",
        ],
        memory_writes: &[
            "coding/exploration/technologies",
            "coding/exploration/recommendations",
        ],
        xp_reward: 40,
        parallelizable: true,
        critical: false,
        description: "Evaluates technology options and external solutions that could address implementation needs.",
    };

pub(super) const CODEBASE_ANALYZER: CodingAgent = CodingAgent {
        key: "codebase-analyzer",
        phase: Phase::Understanding,
        model: "sonnet",
        prompt_source_path: ".archon/agents/coding-pipeline/codebase-analyzer.md",
        tool_access: ToolAccess::ReadOnly,
        algorithm: Algorithm::ReAct,
        fallback_algorithm: Some(Algorithm::Reflexion),
        depends_on: &["technology-scout", "research-planner"],
        memory_reads: &[
            "coding/exploration/technologies",
            "coding/understanding/context",
        ],
        memory_writes: &[
            "coding/exploration/codebase-analysis",
            "coding/exploration/integration-points",
        ],
        xp_reward: 50,
        parallelizable: false,
        critical: false,
        description: "Performs deep analysis of relevant code sections to understand implementation context.",
    };
