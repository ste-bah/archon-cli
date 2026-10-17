//! Game-theory pipeline module.
//!
//! Provides the curated 84-agent game-theory arsenal, Tier 1 parallel
//! classification, 9-axis fingerprint, routing, specialist execution,
//! final-stage report synthesis, and Cozo persistence for all artefacts.

pub mod agents;
pub mod errors;
pub mod facade;
pub mod final_stage;
pub mod fingerprint;
pub mod prompt_builder;
pub mod quality;
pub mod registry;
pub mod routing;
pub mod schema;
pub mod sections;
pub mod spec;
pub mod specimens;

/// Load and validate a game-theory specialist prompt. Shared by preflight and
/// specialist runtime loading so both enforce the same parsed body.
pub fn load_agent_prompt_body(
    agent: &agents::GameTheoryAgent,
    root: &std::path::Path,
) -> Result<String, GameTheoryError> {
    let declared_path = agent.prompt_source_path;
    let path = root.join(declared_path);
    if !path.is_file() {
        return Err(GameTheoryError::MissingAgentFile {
            path: format!(
                "GameTheory pipeline agent '{}' prompt '{}' is unavailable; run `archon agent sync`",
                agent.key, declared_path
            ),
        });
    }
    let content = std::fs::read_to_string(&path).map_err(|error| GameTheoryError::Io {
        message: format!(
            "GameTheory pipeline agent '{}' prompt '{}' is unavailable: {error}; run `archon agent sync`",
            agent.key, declared_path
        ),
    })?;
    let (_frontmatter, body) = crate::agent_loader::parse_frontmatter(&content).map_err(|error| {
        GameTheoryError::Validation {
            message: format!(
                "GameTheory pipeline agent '{}' prompt '{}' is invalid: {error}; run `archon agent sync`",
                agent.key, declared_path
            ),
        }
    })?;
    if body.trim().is_empty() {
        return Err(GameTheoryError::Validation {
            message: format!(
                "GameTheory pipeline agent '{}' prompt '{}' has an empty body; run `archon agent sync`",
                agent.key, declared_path
            ),
        });
    }
    Ok(body)
}

pub use agents::{GameTheoryAgent, GameTheoryTier, GameTheoryToolAccess};
pub use errors::GameTheoryError;
pub use facade::{
    FullPipelineResult, GameTheoryMemoryContext, GameTheoryRunOptions, InProgressRun,
    MemoryRecallAudit, ReplaySpecialistResult, ResumeRunResult, classify, classify_with_learning,
    list_in_progress_runs, replay_routing_from_stored_fingerprint, replay_single_specialist,
    replay_single_specialist_with_learning, resume_run_from_checkpoint, run_full_pipeline,
    run_full_pipeline_with_learning_options, run_full_pipeline_with_memory,
    run_full_pipeline_with_options,
};
pub use fingerprint::{AxisVerdict, GameTheoryFingerprint};
pub use registry::{GAMETHEORY_AGENTS, GAMETHEORY_TIERS};
pub use routing::{
    GameTheorySpec, RoutingDecision, evaluate_routing, load_spec, resolve_spec_path,
};
