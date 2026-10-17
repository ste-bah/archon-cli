//! Archon Pipeline — shared pipeline orchestration engine for coding and research pipelines.

pub mod agent_loader;
pub mod artefacts;
pub mod audit;
pub mod capture;
pub mod coding;
pub mod compression;
pub mod gametheory;
pub mod kb;
pub mod layered_context;
pub mod leann_searcher;
pub mod learning;
pub mod ledgers;
pub mod llm_adapter;
pub mod manifest;
pub mod memory;
pub mod prompt_cap;
pub mod rehydration;
pub mod research;

/// Complete agent key sets declared by the built-in pipelines.
///
/// Keep this derived from the definitions so callers can preflight a run
/// without maintaining a second list of required agents.
pub fn required_builtin_agent_keys(kind: runner::PipelineType) -> Vec<&'static str> {
    match kind {
        runner::PipelineType::Research => research::agents::get_all_agents()
            .iter()
            .map(|agent| agent.key)
            .collect(),
        runner::PipelineType::Coding => coding::agents::AGENTS
            .iter()
            .map(|agent| agent.key)
            .collect(),
        runner::PipelineType::GameTheory => gametheory::GAMETHEORY_AGENTS
            .iter()
            .map(|agent| agent.key)
            .collect(),
        _ => Vec::new(),
    }
}

/// Exact bundled prompt files required by a built-in pipeline. Keys alone are
/// insufficient because separate pipelines may use the same key for different agents.
pub fn required_builtin_agent_assets(
    kind: runner::PipelineType,
) -> Vec<(&'static str, &'static str)> {
    match kind {
        runner::PipelineType::Research => research::agents::get_all_agents()
            .iter()
            .map(|agent| (agent.key, agent.prompt_source_path))
            .collect(),
        runner::PipelineType::Coding => coding::agents::AGENTS
            .iter()
            .map(|agent| (agent.key, agent.prompt_source_path))
            .collect(),
        runner::PipelineType::GameTheory => gametheory::GAMETHEORY_AGENTS
            .iter()
            .map(|agent| (agent.key, agent.prompt_source_path))
            .collect(),
        _ => Vec::new(),
    }
}

/// Validate one built-in prompt with the loader used by that pipeline at
/// runtime. Preflight callers use this to guarantee runtime cannot later
/// reject the same prompt for parsing or an empty body.
pub fn validate_builtin_agent_asset(
    kind: runner::PipelineType,
    key: &str,
    root: &std::path::Path,
) -> anyhow::Result<()> {
    match kind {
        runner::PipelineType::Research => {
            let agent = research::agents::get_all_agents()
                .into_iter()
                .find(|agent| agent.key == key)
                .ok_or_else(|| anyhow::anyhow!("unknown research pipeline agent '{key}'"))?;
            research::prompt_builder::load_agent_prompt_body(agent, root)?;
        }
        runner::PipelineType::Coding => {
            let agent = coding::agents::AGENTS
                .iter()
                .find(|agent| agent.key == key)
                .ok_or_else(|| anyhow::anyhow!("unknown coding pipeline agent '{key}'"))?;
            coding::load_agent_prompt_body(agent, root)?;
        }
        runner::PipelineType::GameTheory => {
            let agent = gametheory::GAMETHEORY_AGENTS
                .iter()
                .find(|agent| agent.key == key)
                .ok_or_else(|| anyhow::anyhow!("unknown GameTheory pipeline agent '{key}'"))?;
            gametheory::load_agent_prompt_body(agent, root)?;
        }
        _ => return Ok(()),
    }
    Ok(())
}
pub mod retry;
pub mod runner;
pub mod session;
pub mod subagent_adapter;

#[cfg(test)]
mod test_support;

// --- Phase 4: Declarative pipeline execution ---
pub mod error;
pub mod id;
pub mod run;
pub mod spec;

pub use id::PipelineId;
pub use run::{PipelineRun, PipelineState, StepRun, StepRunState};
pub use spec::{BackoffKind, OnFailurePolicy, PipelineFormat, PipelineSpec, RetrySpec, StepSpec};
pub mod parser;

pub mod backoff;
pub mod dag;
pub mod vars;

pub mod store;

pub mod condition;
pub mod engine;
pub mod executor;
pub mod rollback;

pub use backoff::delay as backoff_delay;
pub use condition::ConditionEvaluator;
pub use dag::{Dag, DagBuilder};
pub use engine::{DefaultPipelineEngine, PipelineEngine};
pub use error::PipelineError;
pub use executor::PipelineExecutor;
pub use parser::PipelineParser;
pub use rollback::{FailureAction, RollbackEngine};
pub use store::{AuditEvent, PipelineStateStore};
pub use vars::{VarRef, extract_refs, substitute, validate_refs};
