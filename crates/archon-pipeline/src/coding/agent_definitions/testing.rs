use super::super::{Algorithm, CodingAgent, Phase, ToolAccess};

pub(in super::super) const PHASE_4_REVIEWER: CodingAgent = CodingAgent {
    key: "phase-4-reviewer",
    phase: Phase::Testing,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/phase-4-reviewer.md",
    tool_access: ToolAccess::Full,
    algorithm: Algorithm::Reflexion,
    fallback_algorithm: Some(Algorithm::SelfDebug),
    depends_on: &["implementation-coordinator"],
    memory_reads: &[
        "coding/implementation/generated-code",
        "coding/implementation/types",
        "coding/implementation/services",
        "coding/implementation/api",
        "coding/implementation/coordination-report",
    ],
    memory_writes: &[
        "coding/forensic/phase-4-verdict",
        "coding/forensic/phase-4-evidence",
    ],
    xp_reward: 100,
    parallelizable: false,
    critical: true,
    description: "Sherlock #45: Phase 4 Implementation forensic review. CRITICAL: Gates progression to Phase 5.",
};

pub(in super::super) const TEST_GENERATOR: CodingAgent = CodingAgent {
    key: "test-generator",
    phase: Phase::Testing,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/test-generator.md",
    tool_access: ToolAccess::Full,
    algorithm: Algorithm::ToT,
    fallback_algorithm: Some(Algorithm::SelfDebug),
    depends_on: &["phase-4-reviewer"],
    memory_reads: &[
        "coding/implementation/services",
        "coding/understanding/requirements",
    ],
    memory_writes: &[
        "coding/testing/generated-tests",
        "coding/testing/test-files",
    ],
    xp_reward: 55,
    parallelizable: false,
    critical: false,
    description: "Generates comprehensive test suites including unit, integration, and e2e tests.",
};

pub(in super::super) const TEST_RUNNER: CodingAgent = CodingAgent {
    key: "test-runner",
    phase: Phase::Testing,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/test-runner.md",
    tool_access: ToolAccess::Full,
    algorithm: Algorithm::ReAct,
    fallback_algorithm: Some(Algorithm::SelfDebug),
    depends_on: &["test-generator"],
    memory_reads: &[
        "coding/testing/generated-tests",
        "coding/implementation/services",
    ],
    memory_writes: &["coding/testing/results", "coding/testing/failures"],
    xp_reward: 50,
    parallelizable: false,
    critical: true,
    description: "Orchestrates and executes all test suites, managing test lifecycle and reporting results.",
};

pub(in super::super) const INTEGRATION_TESTER: CodingAgent = CodingAgent {
    key: "integration-tester",
    phase: Phase::Testing,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/integration-tester.md",
    tool_access: ToolAccess::Full,
    algorithm: Algorithm::SelfDebug,
    fallback_algorithm: Some(Algorithm::ReAct),
    depends_on: &["test-runner"],
    memory_reads: &["coding/testing/results", "coding/implementation/api"],
    memory_writes: &[
        "coding/testing/integration-tests",
        "coding/testing/integration-results",
    ],
    xp_reward: 55,
    parallelizable: true,
    critical: false,
    description: "Creates and executes integration tests verifying component interactions and system behavior.",
};

pub(in super::super) const REGRESSION_TESTER: CodingAgent = CodingAgent {
    key: "regression-tester",
    phase: Phase::Testing,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/regression-tester.md",
    tool_access: ToolAccess::Full,
    algorithm: Algorithm::Reflexion,
    fallback_algorithm: Some(Algorithm::SelfDebug),
    depends_on: &["test-runner"],
    memory_reads: &["coding/testing/results", "coding/understanding/context"],
    memory_writes: &[
        "coding/testing/regression-tests",
        "coding/testing/breaking-changes",
    ],
    xp_reward: 50,
    parallelizable: true,
    critical: false,
    description: "Performs regression testing to detect unintended changes and compares against baselines.",
};

pub(in super::super) const SECURITY_TESTER: CodingAgent = CodingAgent {
    key: "security-tester",
    phase: Phase::Testing,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/security-tester.md",
    tool_access: ToolAccess::Full,
    algorithm: Algorithm::ReAct,
    fallback_algorithm: Some(Algorithm::Reflexion),
    depends_on: &["integration-tester"],
    memory_reads: &[
        "coding/testing/integration-results",
        "coding/implementation/api",
    ],
    memory_writes: &[
        "coding/testing/security-tests",
        "coding/testing/vulnerabilities",
    ],
    xp_reward: 60,
    parallelizable: true,
    critical: true,
    description: "Performs security testing including vulnerability scanning and compliance verification.",
};

pub(in super::super) const COVERAGE_ANALYZER: CodingAgent = CodingAgent {
    key: "coverage-analyzer",
    phase: Phase::Testing,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/coverage-analyzer.md",
    tool_access: ToolAccess::Full,
    algorithm: Algorithm::PoT,
    fallback_algorithm: Some(Algorithm::ReAct),
    depends_on: &["regression-tester", "security-tester"],
    memory_reads: &[
        "coding/testing/results",
        "coding/testing/integration-results",
    ],
    memory_writes: &[
        "coding/testing/coverage-report",
        "coding/testing/coverage-gaps",
    ],
    xp_reward: 50,
    parallelizable: false,
    critical: false,
    description: "Analyzes test coverage metrics, identifies gaps, and generates coverage reports.",
};

pub(in super::super) const TEST_FIXER: CodingAgent = CodingAgent {
    key: "test-fixer",
    phase: Phase::Testing,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/test-fixer.md",
    tool_access: ToolAccess::Full,
    algorithm: Algorithm::SelfDebug,
    fallback_algorithm: Some(Algorithm::Reflexion),
    depends_on: &["quality-gate"],
    memory_reads: &[
        "coding/testing/results",
        "coding/testing/failures",
        "coding/testing/quality-verdict",
    ],
    memory_writes: &["coding/testing/fix-attempts", "coding/testing/final-status"],
    xp_reward: 65,
    parallelizable: false,
    critical: false,
    description: "Self-correction loop: reads test failures, fixes code, re-tests until pass (max 3 retries). Escalates unfixable failures.",
};

pub(in super::super) const PHASE_5_REVIEWER: CodingAgent = CodingAgent {
    key: "phase-5-reviewer",
    phase: Phase::Testing,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/phase-5-reviewer.md",
    tool_access: ToolAccess::Full,
    algorithm: Algorithm::Reflexion,
    fallback_algorithm: Some(Algorithm::SelfDebug),
    depends_on: &["test-fixer"],
    memory_reads: &[
        "coding/testing/generated-tests",
        "coding/testing/results",
        "coding/testing/coverage-report",
        "coding/testing/quality-verdict",
    ],
    memory_writes: &[
        "coding/forensic/phase-5-verdict",
        "coding/forensic/phase-5-evidence",
    ],
    xp_reward: 100,
    parallelizable: false,
    critical: true,
    description: "Sherlock #46: Phase 5 Testing forensic review. CRITICAL: Gates progression to Phase 6.",
};
