//! Coding pipeline (50 agents).

pub mod agents;
pub mod algorithm;
mod compilation_gate;
pub mod contract;
pub mod evidence;
pub mod facade;

/// Load and validate a coding agent's source prompt. Shared by preflight and
/// the facade so empty or malformed prompt files cannot pass preflight.
pub fn load_agent_prompt_body(
    agent: &agents::CodingAgent,
    root: &std::path::Path,
) -> anyhow::Result<String> {
    let declared_path = agent.prompt_source_path;
    let read_path = root.join(declared_path);
    let content = std::fs::read_to_string(&read_path).map_err(|error| {
        anyhow::anyhow!(
            "Coding pipeline agent '{}' prompt '{}' is unavailable: {error}; run `archon agent sync`.",
            agent.key,
            declared_path
        )
    })?;
    let (_frontmatter, body) = crate::agent_loader::parse_frontmatter(&content).map_err(|error| {
        anyhow::anyhow!(
            "Coding pipeline agent '{}' prompt '{}' is invalid: {error}; run `archon agent sync`.",
            agent.key,
            declared_path
        )
    })?;
    if body.trim().is_empty() {
        anyhow::bail!(
            "Coding pipeline agent '{}' prompt '{}' has an empty body; run `archon agent sync`.",
            agent.key,
            declared_path
        );
    }
    Ok(body)
}
pub mod gates;
pub mod hooks;
mod orphan_gate;
#[cfg(test)]
mod orphan_gate_go_import_tests;
#[cfg(test)]
mod orphan_gate_python_import_tests;
#[cfg(test)]
mod orphan_gate_tests;
pub mod quality;
#[cfg(test)]
mod quality_markdown_tests;
pub mod rlm;
pub mod wiring;

pub use agents::{AGENTS, Algorithm, CodingAgent, Phase, ToolAccess};
