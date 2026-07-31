//! 50-agent coding pipeline definitions.
//!
//! Ports the TypeScript coding pipeline agent configuration to Rust,
//! mapping the original 7 TS phases into 6 Rust phases per PRD REQ-CODE-007:
//!
//! - Phase 1 Understanding (8): core analysis + exploration agents
//! - Phase 2 Design (10): architecture + feasibility + reviewers
//! - Phase 3 WiringPlan (3): integration-architect + phase-3-reviewer
//! - Phase 4 Implementation (11): code generators + implementers
//! - Phase 5 Testing (9): test agents + phase-4/5-reviewers
//! - Phase 6 Refinement (9): optimization + coordination + sign-off

use serde::{Deserialize, Serialize};

mod agent_definitions;

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
// 50 agent definitions
// ---------------------------------------------------------------------------

/// All 50 coding-pipeline agents in execution order.
static AGENT_DEFINITIONS: [CodingAgent; 50] = [
    agent_definitions::CONTRACT_AGENT,
    agent_definitions::REQUIREMENT_EXTRACTOR,
    agent_definitions::REQUIREMENT_PRIORITIZER,
    agent_definitions::SCOPE_DEFINER,
    agent_definitions::CONTEXT_GATHERER,
    agent_definitions::FEASIBILITY_ANALYZER,
    agent_definitions::PATTERN_EXPLORER,
    agent_definitions::TECHNOLOGY_SCOUT,
    agent_definitions::RESEARCH_PLANNER,
    agent_definitions::CODEBASE_ANALYZER,
    agent_definitions::PHASE_1_REVIEWER,
    agent_definitions::PHASE_2_REVIEWER,
    agent_definitions::SYSTEM_DESIGNER,
    agent_definitions::COMPONENT_DESIGNER,
    agent_definitions::INTERFACE_DESIGNER,
    agent_definitions::DATA_ARCHITECT,
    agent_definitions::INTEGRATION_ARCHITECT,
    agent_definitions::WIRING_OBLIGATION_AGENT,
    agent_definitions::PHASE_3_REVIEWER,
    agent_definitions::CODE_GENERATOR,
    agent_definitions::TYPE_IMPLEMENTER,
    agent_definitions::UNIT_IMPLEMENTER,
    agent_definitions::SERVICE_IMPLEMENTER,
    agent_definitions::DATA_LAYER_IMPLEMENTER,
    agent_definitions::API_IMPLEMENTER,
    agent_definitions::FRONTEND_IMPLEMENTER,
    agent_definitions::ERROR_HANDLER_IMPLEMENTER,
    agent_definitions::CONFIG_IMPLEMENTER,
    agent_definitions::LOGGER_IMPLEMENTER,
    agent_definitions::INTEGRATION_VERIFICATION_AGENT,
    agent_definitions::DEPENDENCY_MANAGER,
    agent_definitions::IMPLEMENTATION_COORDINATOR,
    agent_definitions::PHASE_4_REVIEWER,
    agent_definitions::TEST_GENERATOR,
    agent_definitions::TEST_RUNNER,
    agent_definitions::INTEGRATION_TESTER,
    agent_definitions::REGRESSION_TESTER,
    agent_definitions::SECURITY_TESTER,
    agent_definitions::COVERAGE_ANALYZER,
    agent_definitions::QUALITY_GATE,
    agent_definitions::TEST_FIXER,
    agent_definitions::PHASE_5_REVIEWER,
    agent_definitions::PERFORMANCE_OPTIMIZER,
    agent_definitions::PERFORMANCE_ARCHITECT,
    agent_definitions::CODE_QUALITY_IMPROVER,
    agent_definitions::SECURITY_ARCHITECT,
    agent_definitions::FINAL_REFACTORER,
    agent_definitions::SIGN_OFF_APPROVER,
    agent_definitions::PHASE_6_REVIEWER,
    agent_definitions::RECOVERY_AGENT,
];

pub static AGENTS: &[CodingAgent] = &AGENT_DEFINITIONS;

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
