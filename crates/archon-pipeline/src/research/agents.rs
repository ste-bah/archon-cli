//! 47-agent research pipeline definitions.
//!
//! Ports the TypeScript PhD pipeline agent configuration to Rust.
//! 47 agents across 8 phases:
//!
//! - Phase 1 Foundation (6): step-back analysis, decomposition, planning, architecture
//! - Phase 2 Discovery (4): literature mapping, source classification, citations
//! - Phase 3 Architecture (4): theoretical framework, contradictions, gaps, risks
//! - Phase 4 Synthesis (5): evidence synthesis, patterns, themes, theory building
//! - Phase 5 Design (9): methodology, hypotheses, models, instruments, validity
//! - Phase 6 Writing (6): dissertation chapter writing (introduction through abstract)
//! - Phase 7 Validation (12): systematic review, ethics, quality assurance
//! - Phase 8 Final Assembly (1): compose the final paper from validated chapters

use serde::{Deserialize, Serialize};

mod agent_definitions;
mod phase_definitions;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Tool access capabilities for research agents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResearchToolAccess {
    WebSearch,
    WebFetch,
    Read,
    Glob,
    Grep,
    Write,
}

/// Base tool set for all research agents (no Write).
const BASE_TOOLS: &[ResearchToolAccess] = &[
    ResearchToolAccess::WebSearch,
    ResearchToolAccess::WebFetch,
    ResearchToolAccess::Read,
    ResearchToolAccess::Glob,
    ResearchToolAccess::Grep,
];

/// Extended tool set for writing and final-assembly agents (includes Write).
const WRITER_TOOLS: &[ResearchToolAccess] = &[
    ResearchToolAccess::WebSearch,
    ResearchToolAccess::WebFetch,
    ResearchToolAccess::Read,
    ResearchToolAccess::Glob,
    ResearchToolAccess::Grep,
    ResearchToolAccess::Write,
];

/// A single research pipeline agent definition.
#[derive(Clone, Debug, Serialize)]
pub struct ResearchAgent {
    pub key: &'static str,
    pub display_name: &'static str,
    pub phase: u8,
    pub file: &'static str,
    #[serde(serialize_with = "ser_static_str_slice")]
    pub memory_keys: &'static [&'static str],
    #[serde(serialize_with = "ser_static_str_slice")]
    pub output_artifacts: &'static [&'static str],
    pub prompt_source_path: &'static str,
    #[serde(serialize_with = "ser_tool_access_slice")]
    pub tool_access: &'static [ResearchToolAccess],
}

/// A research pipeline phase definition.
#[derive(Clone, Debug, Serialize)]
pub struct ResearchPhase {
    pub id: u8,
    pub name: &'static str,
    pub description: &'static str,
    #[serde(serialize_with = "ser_static_str_slice")]
    pub agent_keys: &'static [&'static str],
}

// ---------------------------------------------------------------------------
// Serde helpers
// ---------------------------------------------------------------------------

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

fn ser_tool_access_slice<S: serde::Serializer>(
    v: &&'static [ResearchToolAccess],
    s: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = s.serialize_seq(Some(v.len()))?;
    for item in *v {
        seq.serialize_element(item)?;
    }
    seq.end()
}

/// Owned mirror of [`ResearchAgent`] used exclusively for deserialization.
#[derive(Deserialize)]
struct OwnedResearchAgent {
    key: String,
    display_name: String,
    phase: u8,
    file: String,
    memory_keys: Vec<String>,
    output_artifacts: Vec<String>,
    prompt_source_path: String,
    tool_access: Vec<ResearchToolAccess>,
}

impl<'de> Deserialize<'de> for ResearchAgent {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let owned = OwnedResearchAgent::deserialize(deserializer)?;
        Ok(ResearchAgent {
            key: Box::leak(owned.key.into_boxed_str()),
            display_name: Box::leak(owned.display_name.into_boxed_str()),
            phase: owned.phase,
            file: Box::leak(owned.file.into_boxed_str()),
            memory_keys: Box::leak(
                owned
                    .memory_keys
                    .into_iter()
                    .map(|s| &*Box::leak(s.into_boxed_str()))
                    .collect::<Vec<&'static str>>()
                    .into_boxed_slice(),
            ),
            output_artifacts: Box::leak(
                owned
                    .output_artifacts
                    .into_iter()
                    .map(|s| &*Box::leak(s.into_boxed_str()))
                    .collect::<Vec<&'static str>>()
                    .into_boxed_slice(),
            ),
            prompt_source_path: Box::leak(owned.prompt_source_path.into_boxed_str()),
            tool_access: Box::leak(owned.tool_access.into_boxed_slice()),
        })
    }
}

/// Owned mirror of [`ResearchPhase`] used exclusively for deserialization.
#[derive(Deserialize)]
struct OwnedResearchPhase {
    id: u8,
    name: String,
    description: String,
    agent_keys: Vec<String>,
}

impl<'de> Deserialize<'de> for ResearchPhase {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let owned = OwnedResearchPhase::deserialize(deserializer)?;
        Ok(ResearchPhase {
            id: owned.id,
            name: Box::leak(owned.name.into_boxed_str()),
            description: Box::leak(owned.description.into_boxed_str()),
            agent_keys: Box::leak(
                owned
                    .agent_keys
                    .into_iter()
                    .map(|s| &*Box::leak(s.into_boxed_str()))
                    .collect::<Vec<&'static str>>()
                    .into_boxed_slice(),
            ),
        })
    }
}

// ---------------------------------------------------------------------------
// 47 agent definitions
// ---------------------------------------------------------------------------

/// All 47 research-pipeline agents in execution order.
static RESEARCH_AGENT_DEFINITIONS: [ResearchAgent; 47] = [
    agent_definitions::foundation::STEP_BACK_ANALYZER,
    agent_definitions::foundation::SELF_ASK_DECOMPOSER,
    agent_definitions::foundation::AMBIGUITY_CLARIFIER,
    agent_definitions::foundation::RESEARCH_PLANNER,
    agent_definitions::foundation::CONSTRUCT_DEFINER,
    agent_definitions::foundation::DISSERTATION_ARCHITECT,
    agent_definitions::discovery::LITERATURE_MAPPER,
    agent_definitions::discovery::SOURCE_TIER_CLASSIFIER,
    agent_definitions::discovery::CITATION_EXTRACTOR,
    agent_definitions::discovery::CONTEXT_TIER_MANAGER,
    agent_definitions::architecture::THEORETICAL_FRAMEWORK_ANALYST,
    agent_definitions::architecture::CONTRADICTION_ANALYZER,
    agent_definitions::architecture::GAP_HUNTER,
    agent_definitions::architecture::RISK_ANALYST,
    agent_definitions::synthesis::EVIDENCE_SYNTHESIZER,
    agent_definitions::synthesis::PATTERN_ANALYST,
    agent_definitions::synthesis::THEMATIC_SYNTHESIZER,
    agent_definitions::synthesis::THEORY_BUILDER,
    agent_definitions::synthesis::OPPORTUNITY_IDENTIFIER,
    agent_definitions::design::METHOD_DESIGNER,
    agent_definitions::design::HYPOTHESIS_GENERATOR,
    agent_definitions::design::MODEL_ARCHITECT,
    agent_definitions::design::ANALYSIS_PLANNER,
    agent_definitions::design::SAMPLING_STRATEGIST,
    agent_definitions::design::INSTRUMENT_DEVELOPER,
    agent_definitions::design::VALIDITY_GUARDIAN,
    agent_definitions::design::METHODOLOGY_SCANNER,
    agent_definitions::design::METHODOLOGY_WRITER,
    agent_definitions::writing::INTRODUCTION_WRITER,
    agent_definitions::writing::LITERATURE_REVIEW_WRITER,
    agent_definitions::writing::RESULTS_WRITER,
    agent_definitions::writing::DISCUSSION_WRITER,
    agent_definitions::writing::CONCLUSION_WRITER,
    agent_definitions::writing::ABSTRACT_WRITER,
    agent_definitions::validation::SYSTEMATIC_REVIEWER,
    agent_definitions::validation::ETHICS_REVIEWER,
    agent_definitions::validation::ADVERSARIAL_REVIEWER,
    agent_definitions::validation::CONFIDENCE_QUANTIFIER,
    agent_definitions::validation::CITATION_VALIDATOR,
    agent_definitions::validation::REPRODUCIBILITY_CHECKER,
    agent_definitions::validation::APA_CITATION_SPECIALIST,
    agent_definitions::validation::CITATION_RECONCILER,
    agent_definitions::validation::CONSISTENCY_VALIDATOR,
    agent_definitions::validation::QUALITY_ASSESSOR,
    agent_definitions::validation::BIAS_DETECTOR,
    agent_definitions::validation::FILE_LENGTH_MANAGER,
    agent_definitions::final_assembly::CHAPTER_SYNTHESIZER,
];

pub static RESEARCH_AGENTS: &[ResearchAgent] = &RESEARCH_AGENT_DEFINITIONS;

// ---------------------------------------------------------------------------
// 8 phase definitions
// ---------------------------------------------------------------------------

/// All 8 research-pipeline phases in order.
static RESEARCH_PHASE_DEFINITIONS: [ResearchPhase; 8] = phase_definitions::RESEARCH_PHASE_DEFINITIONS;

pub static RESEARCH_PHASES: &[ResearchPhase] = &RESEARCH_PHASE_DEFINITIONS;

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Returns a reference to all 47 research agents.
pub fn get_all_agents() -> &'static [ResearchAgent] {
    RESEARCH_AGENTS
}

/// Returns agents belonging to the given phase number (1-8).
pub fn get_agents_by_phase(phase: u8) -> Vec<&'static ResearchAgent> {
    RESEARCH_AGENTS
        .iter()
        .filter(|a| a.phase == phase)
        .collect()
}

/// Looks up a research agent by its unique key.
pub fn get_agent_by_key(key: &str) -> Option<&'static ResearchAgent> {
    RESEARCH_AGENTS.iter().find(|a| a.key == key)
}

/// Returns the 0-based index of the agent with the given key.
pub fn get_agent_index(key: &str) -> Option<usize> {
    RESEARCH_AGENTS.iter().position(|a| a.key == key)
}

/// Looks up a research phase by its ID (1-8).
pub fn get_phase_by_id(id: u8) -> Option<&'static ResearchPhase> {
    RESEARCH_PHASES.iter().find(|p| p.id == id)
}

/// Validates that phase agent_keys match agent definitions and counts are consistent.
pub fn validate_configuration() -> Result<(), String> {
    let agent_keys: std::collections::HashSet<&str> =
        RESEARCH_AGENTS.iter().map(|a| a.key).collect();

    for phase in RESEARCH_PHASES.iter() {
        for agent_key in phase.agent_keys.iter() {
            if !agent_keys.contains(agent_key) {
                return Err(format!(
                    "Phase {} ({}) references unknown agent \"{}\"",
                    phase.id, phase.name, agent_key
                ));
            }
        }
    }

    let phase_agent_count: usize = RESEARCH_PHASES.iter().map(|p| p.agent_keys.len()).sum();
    if phase_agent_count != RESEARCH_AGENTS.len() {
        return Err(format!(
            "Phase agent count ({}) does not match total agents ({})",
            phase_agent_count,
            RESEARCH_AGENTS.len()
        ));
    }

    // Verify each agent's phase matches the phase that lists it
    for phase in RESEARCH_PHASES.iter() {
        for agent_key in phase.agent_keys.iter() {
            if let Some(agent) = get_agent_by_key(agent_key)
                && agent.phase != phase.id
            {
                return Err(format!(
                    "Agent \"{}\" has phase {} but is listed in phase {}",
                    agent_key, agent.phase, phase.id
                ));
            }
        }
    }

    Ok(())
}
