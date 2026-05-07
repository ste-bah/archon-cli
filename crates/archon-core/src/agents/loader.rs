//! Agent loading from custom directories, plugin bundles, and flat files.
//!
//! This module keeps the historical `agents::loader::*` surface while
//! implementation details live in focused submodules.

mod discovery;
mod flat_file;
mod meta;
mod prompt;
mod six_file;

pub use discovery::{load_custom_agents, load_plugin_agents};
pub use flat_file::load_flat_file_agents;
pub use meta::parse_agent_hooks;
pub use prompt::{
    extract_description, extract_tool_guidance, extract_tools, truncate_agent_prompt,
};

#[derive(Debug, thiserror::Error)]
pub enum AgentLoadError {
    #[error("failed to read directory {path}: {source}")]
    ReadDir {
        path: String,
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests;
