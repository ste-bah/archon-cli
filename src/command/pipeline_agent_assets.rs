//! Compile-time bundled files for built-in pipeline agents.
#[path = "pipeline_agent_assets_coding.rs"]
mod coding;
#[path = "pipeline_agent_assets_gametheory.rs"]
mod gametheory;
#[path = "pipeline_agent_assets_research.rs"]
mod research;

pub(crate) fn all() -> impl Iterator<Item = (&'static str, &'static str)> {
    coding::ASSETS
        .iter()
        .chain(research::ASSETS)
        .chain(gametheory::ASSETS)
        .copied()
}
