use super::super::{Algorithm, CodingAgent, Phase, ToolAccess};

pub(in super::super) const DEPENDENCY_MANAGER: CodingAgent = CodingAgent {
    key: "dependency-manager",
    phase: Phase::Refinement,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/dependency-manager.md",
    tool_access: ToolAccess::Full,
    algorithm: Algorithm::ReAct,
    fallback_algorithm: Some(Algorithm::SelfDebug),
    depends_on: &["config-implementer", "logger-implementer"],
    memory_reads: &[
        "coding/implementation/config",
        "coding/architecture/dependencies",
    ],
    memory_writes: &[
        "coding/implementation/dependencies",
        "coding/implementation/package-json",
    ],
    xp_reward: 40,
    parallelizable: false,
    critical: false,
    description: "Manages package dependencies, version resolution, and module organization.",
};

pub(in super::super) const IMPLEMENTATION_COORDINATOR: CodingAgent = CodingAgent {
    key: "implementation-coordinator",
    phase: Phase::Refinement,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/implementation-coordinator.md",
    tool_access: ToolAccess::Full,
    algorithm: Algorithm::Reflexion,
    fallback_algorithm: Some(Algorithm::ReAct),
    depends_on: &["dependency-manager"],
    memory_reads: &[
        "coding/implementation/generated-code",
        "coding/implementation/services",
        "coding/implementation/api",
    ],
    memory_writes: &[
        "coding/implementation/coordination-report",
        "coding/implementation/integration-status",
    ],
    xp_reward: 55,
    parallelizable: false,
    critical: true,
    description: "Coordinates implementation across all agents, manages dependencies, and ensures consistency.",
};

pub(in super::super) const QUALITY_GATE: CodingAgent = CodingAgent {
    key: "quality-gate",
    phase: Phase::Refinement,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/quality-gate.md",
    tool_access: ToolAccess::Full,
    algorithm: Algorithm::Reflexion,
    fallback_algorithm: Some(Algorithm::PoT),
    depends_on: &["coverage-analyzer"],
    memory_reads: &["coding/testing/coverage-report", "coding/testing/results"],
    memory_writes: &["coding/testing/quality-verdict", "coding/testing/l-score"],
    xp_reward: 65,
    parallelizable: false,
    critical: true,
    description: "Validates code against quality gates, computes L-Scores, and determines phase completion.",
};

pub(in super::super) const PERFORMANCE_OPTIMIZER: CodingAgent = CodingAgent {
    key: "performance-optimizer",
    phase: Phase::Refinement,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/performance-optimizer.md",
    tool_access: ToolAccess::Full,
    algorithm: Algorithm::PoT,
    fallback_algorithm: Some(Algorithm::Reflexion),
    depends_on: &["phase-5-reviewer"],
    memory_reads: &["coding/implementation/services", "coding/testing/results"],
    memory_writes: &[
        "coding/optimization/performance",
        "coding/optimization/benchmarks",
    ],
    xp_reward: 60,
    parallelizable: false,
    critical: false,
    description: "Identifies and optimizes performance bottlenecks, memory usage, and runtime efficiency.",
};

pub(in super::super) const CODE_QUALITY_IMPROVER: CodingAgent = CodingAgent {
    key: "code-quality-improver",
    phase: Phase::Refinement,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/code-quality-improver.md",
    tool_access: ToolAccess::Full,
    algorithm: Algorithm::Reflexion,
    fallback_algorithm: Some(Algorithm::ReAct),
    depends_on: &["performance-optimizer"],
    memory_reads: &[
        "coding/implementation/services",
        "coding/testing/quality-verdict",
    ],
    memory_writes: &[
        "coding/optimization/quality-improvements",
        "coding/optimization/refactoring",
    ],
    xp_reward: 50,
    parallelizable: true,
    critical: false,
    description: "Improves code quality through refactoring, pattern application, and maintainability enhancements.",
};

pub(in super::super) const FINAL_REFACTORER: CodingAgent = CodingAgent {
    key: "final-refactorer",
    phase: Phase::Refinement,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/final-refactorer.md",
    tool_access: ToolAccess::Full,
    algorithm: Algorithm::Reflexion,
    fallback_algorithm: Some(Algorithm::SelfDebug),
    depends_on: &["security-architect"],
    memory_reads: &[
        "coding/optimization/quality-improvements",
        "coding/optimization/security-audit",
    ],
    memory_writes: &[
        "coding/optimization/final-code",
        "coding/optimization/polish-report",
    ],
    xp_reward: 55,
    parallelizable: false,
    critical: false,
    description: "Performs final code polish, consistency checks, and prepares code for delivery.",
};

pub(in super::super) const SIGN_OFF_APPROVER: CodingAgent = CodingAgent {
    key: "sign-off-approver",
    phase: Phase::Refinement,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/sign-off-approver.md",
    tool_access: ToolAccess::Full,
    algorithm: Algorithm::Reflexion,
    fallback_algorithm: Some(Algorithm::ReAct),
    depends_on: &["phase-6-reviewer"],
    memory_reads: &[
        "coding/optimization/final-code",
        "coding/testing/coverage-report",
        "coding/testing/quality-verdict",
    ],
    memory_writes: &[
        "coding/delivery/sign-off",
        "coding/delivery/approval-status",
    ],
    xp_reward: 75,
    parallelizable: false,
    critical: true,
    description: "Final sign-off authority for code delivery, verifying all requirements met. CRITICAL: Must pass for pipeline completion.",
};

pub(in super::super) const PHASE_6_REVIEWER: CodingAgent = CodingAgent {
    key: "phase-6-reviewer",
    phase: Phase::Refinement,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/phase-6-reviewer.md",
    tool_access: ToolAccess::Full,
    algorithm: Algorithm::Reflexion,
    fallback_algorithm: Some(Algorithm::ToT),
    depends_on: &["final-refactorer"],
    memory_reads: &[
        "coding/optimization/performance",
        "coding/optimization/quality-improvements",
        "coding/optimization/security-audit",
        "coding/optimization/final-code",
    ],
    memory_writes: &[
        "coding/forensic/phase-6-verdict",
        "coding/forensic/phase-6-evidence",
    ],
    xp_reward: 100,
    parallelizable: false,
    critical: true,
    description: "Sherlock #47: Phase 6 Optimization forensic review. CRITICAL: Gates progression to Phase 7.",
};

pub(in super::super) const RECOVERY_AGENT: CodingAgent = CodingAgent {
    key: "recovery-agent",
    phase: Phase::Refinement,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/recovery-agent.md",
    tool_access: ToolAccess::Full,
    algorithm: Algorithm::Reflexion,
    fallback_algorithm: Some(Algorithm::LATS),
    depends_on: &["sign-off-approver"],
    memory_reads: &[
        "coding/delivery/sign-off",
        "coding/forensic/phase-1-verdict",
        "coding/forensic/phase-2-verdict",
        "coding/forensic/phase-3-verdict",
        "coding/forensic/phase-4-verdict",
        "coding/forensic/phase-5-verdict",
        "coding/forensic/phase-6-verdict",
        "coding/pipeline/feedback-status",
        "coding/pipeline/status",
    ],
    memory_writes: &[
        "coding/forensic/phase-7-verdict",
        "coding/forensic/final-report",
        "coding/forensic/recovery-plan",
        "coding/forensic/feedback-gate-result",
    ],
    xp_reward: 150,
    parallelizable: false,
    critical: true,
    description: "Sherlock #48: Phase 7 Delivery forensic review, recovery orchestration, and MANDATORY feedback gate enforcement. CRITICAL: Final pipeline gate - verifies learning loop closure.",
};
