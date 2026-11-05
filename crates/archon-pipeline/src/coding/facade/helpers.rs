use std::collections::HashSet;

use crate::coding::agents::{CodingAgent, ToolAccess};
use crate::coding::quality::phase_threshold;
use crate::runner::{AgentInfo, ToolAccessLevel};
use archon_core::config::AnthropicModelsConfig;

/// Convert a kebab-case key like `"contract-agent"` into a title-case display
/// name like `"Task Analyzer"`.
pub(super) fn display_name_from_key(key: &str) -> String {
    key.split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => {
                    let upper: String = c.to_uppercase().collect();
                    format!("{}{}", upper, chars.collect::<String>())
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Convert a [`CodingAgent`] to an [`AgentInfo`] for the runner.
///
/// `agent.model` is emitted verbatim as a tier alias (e.g. `"sonnet"`,
/// `"opus"`). Resolution to a provider-specific concrete model id happens
/// downstream at the LLM provider boundary via
/// `LlmProvider::resolve_alias(..)`. That is what keeps pipelines
/// provider-neutral: the same alias resolves to `claude-sonnet-4-6` on
/// Anthropic and `gpt-5.5` on Codex.
///
/// `_models` is retained for API stability (existing call sites pass an
/// `AnthropicModelsConfig`) but is unused — resolution moved to the provider.
pub(super) fn agent_to_info(agent: &CodingAgent, _models: &AnthropicModelsConfig) -> AgentInfo {
    AgentInfo {
        key: agent.key.to_string(),
        display_name: display_name_from_key(agent.key),
        model: agent.model.to_string(),
        phase: agent.phase.ordinal(),
        critical: agent.critical,
        parallelizable: agent.parallelizable,
        quality_threshold: phase_threshold(agent.phase.ordinal()),
        tool_access_level: match agent.tool_access {
            ToolAccess::ReadOnly => ToolAccessLevel::ReadOnly,
            ToolAccess::Full => ToolAccessLevel::Full,
        },
    }
}

pub(super) fn dependencies_satisfied(agent: &CodingAgent, completed: &HashSet<&str>) -> bool {
    agent.depends_on.iter().all(|dep| completed.contains(*dep))
}
