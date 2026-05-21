//! Research pipeline (47 agents across 8 phases).

pub mod agents;
pub mod artifacts;
pub mod chapters;
pub mod citation_gate;
pub mod facade;
pub mod final_appendix;
pub mod final_artifact;
pub mod final_assembly;
pub mod final_stage;
pub mod final_steps;
pub mod pdf;
pub mod prompt_builder;
pub mod quality;
pub mod rlm;
pub mod style;
pub mod verification;

pub use agents::{
    RESEARCH_AGENTS, RESEARCH_PHASES, ResearchAgent, ResearchPhase, ResearchToolAccess,
};
