//! 48-agent coding pipeline definitions.
//!
//! Ports the TypeScript coding pipeline agent configuration to Rust,
//! mapping the original 7 TS phases into 6 Rust phases per PRD REQ-CODE-007:
//!
//! - Phase 1 Understanding (8): core analysis + exploration agents
//! - Phase 2 Design (10): architecture + feasibility + reviewers
//! - Phase 3 WiringPlan (2): integration-architect + phase-3-reviewer
//! - Phase 4 Implementation (10): code generators + implementers
//! - Phase 5 Testing (9): test agents + phase-4/5-reviewers
//! - Phase 6 Refinement (9): optimization + coordination + sign-off

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Pipeline phase (6 phases per PRD REQ-CODE-007).
///
/// Phases 1-3 (Understanding, Design, WiringPlan) use ReadOnly tool access.
/// Phases 4-6 (Implementation, Testing, Refinement) use Full tool access.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Phase {
    Understanding = 1,
    Design = 2,
    WiringPlan = 3,
    Implementation = 4,
    Testing = 5,
    Refinement = 6,
}

/// Tool access level for an agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolAccess {
    /// Read, Glob, Grep, WebSearch, WebFetch
    ReadOnly,
    /// Read, Write, Edit, Bash, Glob, Grep, WebSearch, WebFetch
    Full,
}

/// USACF reasoning algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Algorithm {
    LATS,
    ReAct,
    ToT,
    SelfDebug,
    Reflexion,
    PoT,
}

/// Definition of a single coding pipeline agent (static / borrowed form).
///
/// This struct is used for the compile-time `AGENTS` array. All string and
/// slice fields are `&'static` references. Serde `Serialize` is derived
/// automatically; `Deserialize` is implemented manually so that the borrowed
/// fields can round-trip through an intermediate owned representation.
#[derive(Clone, Debug, Serialize)]
pub struct CodingAgent {
    pub key: &'static str,
    pub phase: Phase,
    pub model: &'static str,
    pub prompt_source_path: &'static str,
    pub tool_access: ToolAccess,
    pub algorithm: Algorithm,
    pub fallback_algorithm: Option<Algorithm>,
    #[serde(serialize_with = "ser_static_str_slice")]
    pub depends_on: &'static [&'static str],
    #[serde(serialize_with = "ser_static_str_slice")]
    pub memory_reads: &'static [&'static str],
    #[serde(serialize_with = "ser_static_str_slice")]
    pub memory_writes: &'static [&'static str],
    pub xp_reward: u32,
    pub parallelizable: bool,
    pub critical: bool,
    pub description: &'static str,
}

fn ser_static_str_slice<S: serde::Serializer>(
    v: &&'static [&'static str],
    s: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = s.serialize_seq(Some(v.len()))?;
    for item in *v {
        seq.serialize_element(item)?;
    }
    seq.end()
}

/// Owned mirror of [`CodingAgent`] used exclusively for deserialization.
#[derive(Deserialize)]
struct OwnedCodingAgent {
    key: String,
    phase: Phase,
    model: String,
    prompt_source_path: String,
    tool_access: ToolAccess,
    algorithm: Algorithm,
    fallback_algorithm: Option<Algorithm>,
    depends_on: Vec<String>,
    memory_reads: Vec<String>,
    memory_writes: Vec<String>,
    xp_reward: u32,
    parallelizable: bool,
    critical: bool,
    description: String,
}

impl<'de> Deserialize<'de> for CodingAgent {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let owned = OwnedCodingAgent::deserialize(deserializer)?;
        // Leak the owned strings into &'static str so the type signature is
        // satisfied. This is only used for testing round-trips; the canonical
        // data lives in the static AGENTS array.
        Ok(CodingAgent {
            key: Box::leak(owned.key.into_boxed_str()),
            phase: owned.phase,
            model: Box::leak(owned.model.into_boxed_str()),
            prompt_source_path: Box::leak(owned.prompt_source_path.into_boxed_str()),
            tool_access: owned.tool_access,
            algorithm: owned.algorithm,
            fallback_algorithm: owned.fallback_algorithm,
            depends_on: Box::leak(
                owned
                    .depends_on
                    .into_iter()
                    .map(|s| &*Box::leak(s.into_boxed_str()))
                    .collect::<Vec<&'static str>>()
                    .into_boxed_slice(),
            ),
            memory_reads: Box::leak(
                owned
                    .memory_reads
                    .into_iter()
                    .map(|s| &*Box::leak(s.into_boxed_str()))
                    .collect::<Vec<&'static str>>()
                    .into_boxed_slice(),
            ),
            memory_writes: Box::leak(
                owned
                    .memory_writes
                    .into_iter()
                    .map(|s| &*Box::leak(s.into_boxed_str()))
                    .collect::<Vec<&'static str>>()
                    .into_boxed_slice(),
            ),
            xp_reward: owned.xp_reward,
            parallelizable: owned.parallelizable,
            critical: owned.critical,
            description: Box::leak(owned.description.into_boxed_str()),
        })
    }
}

// ---------------------------------------------------------------------------
// 48 agent definitions
// ---------------------------------------------------------------------------

/// All 48 coding-pipeline agents in execution order.
pub static AGENTS: &[CodingAgent] = &[
    // =========================================================================
    // PHASE 1: UNDERSTANDING  (TS understanding agents)
    // =========================================================================

    // #1 contract-agent (replaces task-analyzer per REQ-IMPROVE-001)
    CodingAgent {
        key: "contract-agent",
        phase: Phase::Understanding,
        model: "claude-sonnet-4-20250514",
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
    },
    // #2 requirement-extractor
    CodingAgent {
        key: "requirement-extractor",
        phase: Phase::Understanding,
        model: "claude-sonnet-4-20250514",
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
    },
    // #3 requirement-prioritizer
    CodingAgent {
        key: "requirement-prioritizer",
        phase: Phase::Understanding,
        model: "claude-sonnet-4-20250514",
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
    },
    // #4 scope-definer
    CodingAgent {
        key: "scope-definer",
        phase: Phase::Understanding,
        model: "claude-sonnet-4-20250514",
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
    },
    // #5 context-gatherer
    CodingAgent {
        key: "context-gatherer",
        phase: Phase::Understanding,
        model: "claude-sonnet-4-20250514",
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
    },
    // #6 feasibility-analyzer
    CodingAgent {
        key: "feasibility-analyzer",
        phase: Phase::Design,
        model: "claude-sonnet-4-20250514",
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
    },
    // =========================================================================
    // PHASE 1: UNDERSTANDING  (TS exploration agents — merged)
    // =========================================================================

    // #7 pattern-explorer
    CodingAgent {
        key: "pattern-explorer",
        phase: Phase::Understanding,
        model: "claude-sonnet-4-20250514",
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
    },
    // #8 technology-scout
    CodingAgent {
        key: "technology-scout",
        phase: Phase::Understanding,
        model: "claude-sonnet-4-20250514",
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
    },
    // #9 research-planner
    CodingAgent {
        key: "research-planner",
        phase: Phase::Design,
        model: "claude-sonnet-4-20250514",
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
    },
    // #10 codebase-analyzer
    CodingAgent {
        key: "codebase-analyzer",
        phase: Phase::Understanding,
        model: "claude-sonnet-4-20250514",
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
    },
    // =========================================================================
    // PHASE 1: UNDERSTANDING  (Sherlock reviewers that gate Phase 1 → 2)
    // =========================================================================

    // #42 phase-1-reviewer
    CodingAgent {
        key: "phase-1-reviewer",
        phase: Phase::Design,
        model: "claude-sonnet-4-20250514",
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
    },
    // #43 phase-2-reviewer
    CodingAgent {
        key: "phase-2-reviewer",
        phase: Phase::Design,
        model: "claude-sonnet-4-20250514",
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
    },
    // =========================================================================
    // PHASE 2: DESIGN  (TS architecture agents)
    // =========================================================================

    // #11 system-designer
    CodingAgent {
        key: "system-designer",
        phase: Phase::Design,
        model: "claude-sonnet-4-20250514",
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
    },
    // #12 component-designer
    CodingAgent {
        key: "component-designer",
        phase: Phase::Design,
        model: "claude-sonnet-4-20250514",
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
    },
    // #13 interface-designer
    CodingAgent {
        key: "interface-designer",
        phase: Phase::Design,
        model: "claude-sonnet-4-20250514",
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
    },
    // #14 data-architect
    CodingAgent {
        key: "data-architect",
        phase: Phase::Design,
        model: "claude-sonnet-4-20250514",
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
    },
    // #15 integration-architect
    CodingAgent {
        key: "integration-architect",
        phase: Phase::WiringPlan,
        model: "claude-sonnet-4-20250514",
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
    },
    // #48 wiring-obligation-agent (REQ-IMPROVE-003)
    CodingAgent {
        key: "wiring-obligation-agent",
        phase: Phase::WiringPlan,
        model: "claude-sonnet-4-20250514",
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
    },
    // =========================================================================
    // PHASE 3: WIRING PLAN  (Sherlock reviewer that gates WiringPlan)
    // =========================================================================

    // #44 phase-3-reviewer
    CodingAgent {
        key: "phase-3-reviewer",
        phase: Phase::WiringPlan,
        model: "claude-sonnet-4-20250514",
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
    },
    // =========================================================================
    // PHASE 3: IMPLEMENTATION  (TS implementation agents)
    // =========================================================================

    // #16 code-generator
    CodingAgent {
        key: "code-generator",
        phase: Phase::Implementation,
        model: "claude-sonnet-4-20250514",
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
    },
    // #17 type-implementer
    CodingAgent {
        key: "type-implementer",
        phase: Phase::Implementation,
        model: "claude-sonnet-4-20250514",
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
    },
    // #18 unit-implementer
    CodingAgent {
        key: "unit-implementer",
        phase: Phase::Implementation,
        model: "claude-sonnet-4-20250514",
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
    },
    // #19 service-implementer
    CodingAgent {
        key: "service-implementer",
        phase: Phase::Implementation,
        model: "claude-sonnet-4-20250514",
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
    },
    // #20 data-layer-implementer
    CodingAgent {
        key: "data-layer-implementer",
        phase: Phase::Implementation,
        model: "claude-sonnet-4-20250514",
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
    },
    // #21 api-implementer
    CodingAgent {
        key: "api-implementer",
        phase: Phase::Implementation,
        model: "claude-sonnet-4-20250514",
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
    },
    // #22 frontend-implementer
    CodingAgent {
        key: "frontend-implementer",
        phase: Phase::Implementation,
        model: "claude-sonnet-4-20250514",
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
    },
    // #23 error-handler-implementer
    CodingAgent {
        key: "error-handler-implementer",
        phase: Phase::Implementation,
        model: "claude-sonnet-4-20250514",
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
    },
    // #24 config-implementer
    CodingAgent {
        key: "config-implementer",
        phase: Phase::Implementation,
        model: "claude-sonnet-4-20250514",
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
    },
    // #25 logger-implementer
    CodingAgent {
        key: "logger-implementer",
        phase: Phase::Implementation,
        model: "claude-sonnet-4-20250514",
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
    },
    // Integration Verification Agent — runs after Phase 4 (last agent in Phase 4)
    CodingAgent {
        key: "integration-verification-agent",
        phase: Phase::Implementation,
        model: "claude-sonnet-4-20250514",
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
    },
    // #26 dependency-manager
    CodingAgent {
        key: "dependency-manager",
        phase: Phase::Refinement,
        model: "claude-sonnet-4-20250514",
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
    },
    // #27 implementation-coordinator
    CodingAgent {
        key: "implementation-coordinator",
        phase: Phase::Refinement,
        model: "claude-sonnet-4-20250514",
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
    },
    // =========================================================================
    // PHASE 3: IMPLEMENTATION  (Sherlock reviewer)
    // =========================================================================

    // #45 phase-4-reviewer
    CodingAgent {
        key: "phase-4-reviewer",
        phase: Phase::Testing,
        model: "claude-sonnet-4-20250514",
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
    },
    // =========================================================================
    // PHASE 4: TESTING  (TS testing agents)
    // =========================================================================

    // #28 test-generator
    CodingAgent {
        key: "test-generator",
        phase: Phase::Testing,
        model: "claude-sonnet-4-20250514",
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
    },
    // #29 test-runner
    CodingAgent {
        key: "test-runner",
        phase: Phase::Testing,
        model: "claude-sonnet-4-20250514",
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
    },
    // #30 integration-tester
    CodingAgent {
        key: "integration-tester",
        phase: Phase::Testing,
        model: "claude-sonnet-4-20250514",
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
    },
    // #31 regression-tester
    CodingAgent {
        key: "regression-tester",
        phase: Phase::Testing,
        model: "claude-sonnet-4-20250514",
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
    },
    // #32 security-tester
    CodingAgent {
        key: "security-tester",
        phase: Phase::Testing,
        model: "claude-sonnet-4-20250514",
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
    },
    // #33 coverage-analyzer
    CodingAgent {
        key: "coverage-analyzer",
        phase: Phase::Testing,
        model: "claude-sonnet-4-20250514",
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
    },
    // #34 quality-gate
    CodingAgent {
        key: "quality-gate",
        phase: Phase::Refinement,
        model: "claude-sonnet-4-20250514",
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
    },
    // #35 test-fixer
    CodingAgent {
        key: "test-fixer",
        phase: Phase::Testing,
        model: "claude-sonnet-4-20250514",
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
    },
    // =========================================================================
    // PHASE 4: TESTING  (Sherlock reviewer)
    // =========================================================================

    // #46 phase-5-reviewer
    CodingAgent {
        key: "phase-5-reviewer",
        phase: Phase::Testing,
        model: "claude-sonnet-4-20250514",
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
    },
    // =========================================================================
    // PHASE 5: REFINEMENT  (TS optimization agents)
    // =========================================================================

    // #36 performance-optimizer
    CodingAgent {
        key: "performance-optimizer",
        phase: Phase::Refinement,
        model: "claude-sonnet-4-20250514",
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
    },
    // #37 performance-architect
    CodingAgent {
        key: "performance-architect",
        phase: Phase::Design,
        model: "claude-sonnet-4-20250514",
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
    },
    // #38 code-quality-improver
    CodingAgent {
        key: "code-quality-improver",
        phase: Phase::Refinement,
        model: "claude-sonnet-4-20250514",
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
    },
    // #39 security-architect
    CodingAgent {
        key: "security-architect",
        phase: Phase::Design,
        model: "claude-sonnet-4-20250514",
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
    },
    // #40 final-refactorer
    CodingAgent {
        key: "final-refactorer",
        phase: Phase::Refinement,
        model: "claude-sonnet-4-20250514",
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
    },
    // =========================================================================
    // PHASE 5: REFINEMENT  (TS delivery agents — merged)
    // =========================================================================

    // #41 sign-off-approver
    CodingAgent {
        key: "sign-off-approver",
        phase: Phase::Refinement,
        model: "claude-sonnet-4-20250514",
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
    },
    // =========================================================================
    // PHASE 5: REFINEMENT  (Sherlock reviewers)
    // =========================================================================

    // #47 phase-6-reviewer
    CodingAgent {
        key: "phase-6-reviewer",
        phase: Phase::Refinement,
        model: "claude-sonnet-4-20250514",
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
    },
    // #48 recovery-agent
    CodingAgent {
        key: "recovery-agent",
        phase: Phase::Refinement,
        model: "claude-sonnet-4-20250514",
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
    },
];

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Look up a single agent by its key.
pub fn get_agent_by_key(key: &str) -> Option<&'static CodingAgent> {
    AGENTS.iter().find(|a| a.key == key)
}

/// Return all agents belonging to the given phase (preserving definition order).
pub fn get_agents_by_phase(phase: Phase) -> Vec<&'static CodingAgent> {
    AGENTS.iter().filter(|a| a.phase == phase).collect()
}

/// Total number of agents in the pipeline.
pub fn agent_count() -> usize {
    AGENTS.len()
}
