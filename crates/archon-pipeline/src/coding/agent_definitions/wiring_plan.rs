use super::super::{Algorithm, CodingAgent, Phase, ToolAccess};

pub(in super::super) const INTEGRATION_ARCHITECT: CodingAgent = CodingAgent {
    key: "integration-architect",
    phase: Phase::WiringPlan,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/integration-architect.md",
    tool_access: ToolAccess::ReadOnly,
    algorithm: Algorithm::ToT,
    fallback_algorithm: Some(Algorithm::Reflexion),
    depends_on: &["interface-designer", "data-architect"],
    memory_reads: &[
        "coding/architecture/interfaces",
        "coding/architecture/data-models",
    ],
    memory_writes: &[
        "coding/architecture/integrations",
        "coding/architecture/dependencies",
    ],
    xp_reward: 55,
    parallelizable: false,
    critical: false,
    description: "Designs integration patterns, external API connections, and system interoperability.",
};

pub(in super::super) const WIRING_OBLIGATION_AGENT: CodingAgent = CodingAgent {
    key: "wiring-obligation-agent",
    phase: Phase::WiringPlan,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/wiring-obligation-agent.md",
    tool_access: ToolAccess::ReadOnly,
    algorithm: Algorithm::ToT,
    fallback_algorithm: Some(Algorithm::ReAct),
    depends_on: &["integration-architect"],
    memory_reads: &[
        "coding/architecture/integrations",
        "coding/architecture/dependencies",
        "coding/contract",
    ],
    memory_writes: &["coding/wiring-plan"],
    xp_reward: 60,
    parallelizable: false,
    critical: true,
    description: "Produces WiringPlan with typed obligations before implementation begins. Gates Phase 4.",
};

pub(in super::super) const PHASE_3_REVIEWER: CodingAgent = CodingAgent {
    key: "phase-3-reviewer",
    phase: Phase::WiringPlan,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/phase-3-reviewer.md",
    tool_access: ToolAccess::ReadOnly,
    algorithm: Algorithm::Reflexion,
    fallback_algorithm: Some(Algorithm::ToT),
    depends_on: &["wiring-obligation-agent"],
    memory_reads: &[
        "coding/architecture/design",
        "coding/architecture/components",
        "coding/architecture/interfaces",
        "coding/architecture/data-models",
        "coding/architecture/integrations",
        "coding/wiring-plan",
    ],
    memory_writes: &[
        "coding/forensic/phase-3-verdict",
        "coding/forensic/phase-3-evidence",
    ],
    xp_reward: 100,
    parallelizable: false,
    critical: true,
    description: "Sherlock #44: Phase 3 Architecture forensic review. CRITICAL: Gates progression to Phase 4.",
};
