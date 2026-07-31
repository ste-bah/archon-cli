use super::super::{Algorithm, CodingAgent, Phase, ToolAccess};

pub(super) const CODE_GENERATOR: CodingAgent = CodingAgent {
        key: "code-generator",
        phase: Phase::Implementation,
        model: "sonnet",
        prompt_source_path: ".archon/agents/coding-pipeline/code-generator.md",
        tool_access: ToolAccess::Full,
        algorithm: Algorithm::SelfDebug,
        fallback_algorithm: Some(Algorithm::ReAct),
        depends_on: &["phase-3-reviewer"],
        memory_reads: &[
            "coding/architecture/design",
            "coding/architecture/interfaces",
        ],
        memory_writes: &[
            "coding/implementation/generated-code",
            "coding/implementation/core-files",
        ],
        xp_reward: 70,
        parallelizable: false,
        critical: true,
        description: "Generates clean, production-ready code following architecture specifications.",
    };

pub(super) const TYPE_IMPLEMENTER: CodingAgent = CodingAgent {
        key: "type-implementer",
        phase: Phase::Implementation,
        model: "sonnet",
        prompt_source_path: ".archon/agents/coding-pipeline/type-implementer.md",
        tool_access: ToolAccess::Full,
        algorithm: Algorithm::SelfDebug,
        fallback_algorithm: Some(Algorithm::ReAct),
        depends_on: &["code-generator"],
        memory_reads: &[
            "coding/architecture/interfaces",
            "coding/implementation/generated-code",
        ],
        memory_writes: &[
            "coding/implementation/types",
            "coding/implementation/type-files",
        ],
        xp_reward: 55,
        parallelizable: true,
        critical: false,
        description: "Implements TypeScript type definitions, interfaces, generics, and type utilities.",
    };

pub(super) const UNIT_IMPLEMENTER: CodingAgent = CodingAgent {
        key: "unit-implementer",
        phase: Phase::Implementation,
        model: "sonnet",
        prompt_source_path: ".archon/agents/coding-pipeline/unit-implementer.md",
        tool_access: ToolAccess::Full,
        algorithm: Algorithm::SelfDebug,
        fallback_algorithm: Some(Algorithm::Reflexion),
        depends_on: &["type-implementer"],
        memory_reads: &[
            "coding/implementation/types",
            "coding/architecture/components",
        ],
        memory_writes: &[
            "coding/implementation/units",
            "coding/implementation/entities",
        ],
        xp_reward: 55,
        parallelizable: true,
        critical: false,
        description: "Implements domain entities, value objects, and core business logic units.",
    };

pub(super) const SERVICE_IMPLEMENTER: CodingAgent = CodingAgent {
        key: "service-implementer",
        phase: Phase::Implementation,
        model: "sonnet",
        prompt_source_path: ".archon/agents/coding-pipeline/service-implementer.md",
        tool_access: ToolAccess::Full,
        algorithm: Algorithm::LATS,
        fallback_algorithm: Some(Algorithm::SelfDebug),
        depends_on: &["unit-implementer"],
        memory_reads: &["coding/implementation/units", "coding/architecture/design"],
        memory_writes: &[
            "coding/implementation/services",
            "coding/implementation/business-logic",
        ],
        xp_reward: 60,
        parallelizable: false,
        critical: false,
        description: "Implements domain services, business logic, and application use cases.",
    };

pub(super) const DATA_LAYER_IMPLEMENTER: CodingAgent = CodingAgent {
        key: "data-layer-implementer",
        phase: Phase::Implementation,
        model: "sonnet",
        prompt_source_path: ".archon/agents/coding-pipeline/data-layer-implementer.md",
        tool_access: ToolAccess::Full,
        algorithm: Algorithm::SelfDebug,
        fallback_algorithm: Some(Algorithm::ReAct),
        depends_on: &["unit-implementer"],
        memory_reads: &[
            "coding/implementation/units",
            "coding/architecture/data-models",
        ],
        memory_writes: &[
            "coding/implementation/data-layer",
            "coding/implementation/repositories",
        ],
        xp_reward: 55,
        parallelizable: true,
        critical: false,
        description: "Implements repositories, database access, and data persistence layer.",
    };

pub(super) const API_IMPLEMENTER: CodingAgent = CodingAgent {
        key: "api-implementer",
        phase: Phase::Implementation,
        model: "sonnet",
        prompt_source_path: ".archon/agents/coding-pipeline/api-implementer.md",
        tool_access: ToolAccess::Full,
        algorithm: Algorithm::ReAct,
        fallback_algorithm: Some(Algorithm::SelfDebug),
        depends_on: &["service-implementer", "data-layer-implementer"],
        memory_reads: &[
            "coding/implementation/services",
            "coding/architecture/interfaces",
        ],
        memory_writes: &[
            "coding/implementation/api",
            "coding/implementation/endpoints",
        ],
        xp_reward: 60,
        parallelizable: false,
        critical: false,
        description: "Implements REST/GraphQL API endpoints, controllers, and request validation.",
    };

pub(super) const FRONTEND_IMPLEMENTER: CodingAgent = CodingAgent {
        key: "frontend-implementer",
        phase: Phase::Implementation,
        model: "sonnet",
        prompt_source_path: ".archon/agents/coding-pipeline/frontend-implementer.md",
        tool_access: ToolAccess::Full,
        algorithm: Algorithm::SelfDebug,
        fallback_algorithm: Some(Algorithm::ReAct),
        depends_on: &["api-implementer"],
        memory_reads: &[
            "coding/implementation/api",
            "coding/architecture/components",
        ],
        memory_writes: &[
            "coding/implementation/frontend",
            "coding/implementation/ui-components",
        ],
        xp_reward: 55,
        parallelizable: true,
        critical: false,
        description: "Implements UI components, pages, state management, and client-side logic.",
    };

pub(super) const ERROR_HANDLER_IMPLEMENTER: CodingAgent = CodingAgent {
        key: "error-handler-implementer",
        phase: Phase::Implementation,
        model: "sonnet",
        prompt_source_path: ".archon/agents/coding-pipeline/error-handler-implementer.md",
        tool_access: ToolAccess::Full,
        algorithm: Algorithm::ReAct,
        fallback_algorithm: Some(Algorithm::Reflexion),
        depends_on: &["api-implementer"],
        memory_reads: &[
            "coding/implementation/api",
            "coding/implementation/services",
        ],
        memory_writes: &[
            "coding/implementation/error-handling",
            "coding/implementation/exceptions",
        ],
        xp_reward: 50,
        parallelizable: true,
        critical: false,
        description: "Implements error handling strategies, recovery mechanisms, and error reporting.",
    };

pub(super) const CONFIG_IMPLEMENTER: CodingAgent = CodingAgent {
        key: "config-implementer",
        phase: Phase::Implementation,
        model: "sonnet",
        prompt_source_path: ".archon/agents/coding-pipeline/config-implementer.md",
        tool_access: ToolAccess::Full,
        algorithm: Algorithm::ReAct,
        fallback_algorithm: Some(Algorithm::SelfDebug),
        depends_on: &["frontend-implementer"],
        memory_reads: &[
            "coding/implementation/api",
            "coding/architecture/dependencies",
        ],
        memory_writes: &[
            "coding/implementation/config",
            "coding/implementation/settings",
        ],
        xp_reward: 40,
        parallelizable: true,
        critical: false,
        description: "Implements configuration management, environment handling, and feature flags.",
    };

pub(super) const LOGGER_IMPLEMENTER: CodingAgent = CodingAgent {
        key: "logger-implementer",
        phase: Phase::Implementation,
        model: "sonnet",
        prompt_source_path: ".archon/agents/coding-pipeline/logger-implementer.md",
        tool_access: ToolAccess::Full,
        algorithm: Algorithm::ReAct,
        fallback_algorithm: Some(Algorithm::SelfDebug),
        depends_on: &["error-handler-implementer"],
        memory_reads: &[
            "coding/implementation/error-handling",
            "coding/implementation/services",
        ],
        memory_writes: &[
            "coding/implementation/logging",
            "coding/implementation/observability",
        ],
        xp_reward: 45,
        parallelizable: true,
        critical: false,
        description: "Implements logging infrastructure, log formatting, and observability patterns.",
    };

pub(super) const INTEGRATION_VERIFICATION_AGENT: CodingAgent = CodingAgent {
        key: "integration-verification-agent",
        phase: Phase::Implementation,
        model: "sonnet",
        prompt_source_path: ".archon/agents/coding-pipeline/integration-verification-agent.md",
        tool_access: ToolAccess::ReadOnly,
        algorithm: Algorithm::ReAct,
        fallback_algorithm: None,
        depends_on: &["logger-implementer"],
        memory_reads: &[
            "coding/implementation/wiring-plan",
            "coding/implementation/generated-code",
        ],
        memory_writes: &[
            "coding/implementation/verification-report",
            "coding/implementation/wiring-status",
        ],
        xp_reward: 60,
        parallelizable: false,
        critical: true,
        description: "Verifies all wiring obligations from the WiringPlan using tool-based checks (Read). Reports per-obligation pass/fail with evidence.",
    };
