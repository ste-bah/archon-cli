//! Research pipeline (46 agents).

pub mod agents;
pub mod chapters;
pub mod facade;
pub mod final_stage;
pub mod prompt_builder;
pub mod quality;
pub mod style;
pub mod verification;

pub use agents::{
    ResearchAgent, ResearchPhase, ResearchToolAccess, RESEARCH_AGENTS, RESEARCH_PHASES,
};
