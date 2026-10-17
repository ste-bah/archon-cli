//! 5-part prompt builder for research pipeline agents.
//!
//! Assembles agent prompts from five sections: agent instructions, workflow
//! context, prior context (with safe truncation), output expectations, and
//! task completion summary. Phase 6 and final-assembly agents additionally
//! receive style injection via [`StyleInjector`].

use super::agents::{RESEARCH_AGENTS, get_phase_by_id};
use super::style::StyleInjector;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// Load and validate a research agent's source prompt. Shared by preflight and
/// runtime prompt construction so both reject the same files.
pub fn load_agent_prompt_body(agent: &super::agents::ResearchAgent, root: &Path) -> Result<String> {
    let declared_path = agent.prompt_source_path;
    let read_path = root.join(declared_path);
    let content = std::fs::read_to_string(&read_path).with_context(|| {
        format!(
            "Research pipeline agent '{}' prompt '{}' is unavailable; run `archon agent sync`.",
            agent.key, declared_path
        )
    })?;
    if content.trim().is_empty() {
        bail!(
            "Research pipeline agent '{}' prompt '{}' is empty; run `archon agent sync`.",
            agent.key,
            declared_path
        );
    }
    let (_frontmatter, body) =
        crate::agent_loader::parse_frontmatter(&content).with_context(|| {
            format!(
                "Research pipeline agent '{}' prompt '{}' is invalid; run `archon agent sync`.",
                agent.key, declared_path
            )
        })?;
    if body.trim().is_empty() {
        bail!(
            "Research pipeline agent '{}' prompt '{}' has an empty body; run `archon agent sync`.",
            agent.key,
            declared_path
        );
    }
    Ok(body)
}

// ---------------------------------------------------------------------------
// Safe string helpers
// ---------------------------------------------------------------------------

/// Truncate `text` at a char boundary, appending a truncation marker.
///
/// If `text` is shorter than `max_chars`, it is returned unchanged.
/// Otherwise, the result is `max_chars` characters followed by
/// `"\n... [truncated]"`.
pub fn safe_truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    // Find the byte offset of the `max_chars`-th character.
    let byte_offset = text
        .char_indices()
        .nth(max_chars)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len());
    let mut result = text[..byte_offset].to_string();
    result.push_str("\n... [truncated]");
    result
}

/// Escape triple backticks to prevent prompt injection.
pub fn escape_backticks(text: &str) -> String {
    text.replace("```", "\\`\\`\\`")
}

// ---------------------------------------------------------------------------
// ResearchPromptBuilder
// ---------------------------------------------------------------------------

/// Builds the 5-part prompt for research pipeline agents.
pub struct ResearchPromptBuilder {
    project_root: PathBuf,
}

impl ResearchPromptBuilder {
    pub fn new() -> Self {
        Self {
            project_root: PathBuf::from("."),
        }
    }

    /// Set the project root used to load research agent prompt assets.
    pub fn with_project_root(mut self, project_root: impl Into<PathBuf>) -> Self {
        self.project_root = project_root.into();
        self
    }

    /// Build the full prompt for the given research agent.
    ///
    /// # Arguments
    /// * `agent` – the research agent definition
    /// * `agent_index` – 0-based position in the pipeline
    /// * `total_agents` – total agent count (typically 47)
    /// * `task` – the research query / task description
    /// * `prior_context` – recalled memory content from previous agents
    /// * `style_prompt` – optional style override for writing/final assembly
    pub fn build(
        &self,
        agent: &super::agents::ResearchAgent,
        agent_index: usize,
        total_agents: usize,
        task: &str,
        prior_context: &str,
        style_prompt: Option<&str>,
    ) -> Result<String> {
        let mut parts: Vec<String> = Vec::with_capacity(5);

        // Part 1: Agent Instructions
        parts.push(self.build_agent_instructions(agent, &self.project_root)?);

        // Part 2: Workflow Context
        parts.push(self.build_workflow_context(agent, agent_index, total_agents));

        // Part 3: Prior Context (omitted if empty)
        if !prior_context.is_empty() {
            parts.push(self.build_prior_context(agent, prior_context));
        }

        // Part 4: Output Expectations
        parts.push(self.build_output_expectations(agent));

        // Part 5: Task Completion Summary
        parts.push(self.build_task_completion(task));

        let mut prompt = parts.join("\n\n");

        // Phase 6 and final assembly: inject style guidelines
        if (agent.phase == 6 || agent.phase == 8)
            && let Some(style) = style_prompt
        {
            let injector = StyleInjector::new();
            prompt = injector.build_styled_prompt(&prompt, style);
        }

        Ok(prompt)
    }

    // -----------------------------------------------------------------------
    // Part builders
    // -----------------------------------------------------------------------

    fn build_agent_instructions(
        &self,
        agent: &super::agents::ResearchAgent,
        root: &Path,
    ) -> Result<String> {
        load_agent_prompt_body(agent, root)
    }

    fn build_workflow_context(
        &self,
        agent: &super::agents::ResearchAgent,
        agent_index: usize,
        total_agents: usize,
    ) -> String {
        let phase_name = get_phase_by_id(agent.phase)
            .map(|p| p.name)
            .unwrap_or("Unknown");

        let prev_agent = if agent_index > 0 {
            RESEARCH_AGENTS
                .get(agent_index - 1)
                .map(|a| a.key.to_string())
                .unwrap_or_else(|| "none".to_string())
        } else {
            "none (first agent)".to_string()
        };

        let next_agent = if agent_index + 1 < total_agents {
            RESEARCH_AGENTS
                .get(agent_index + 1)
                .map(|a| a.key.to_string())
                .unwrap_or_else(|| "none".to_string())
        } else {
            "none (final agent)".to_string()
        };

        format!(
            "## Workflow Context\n\
             Agent #{} of {}\n\
             Phase: {} - {}\n\
             Previous agent: {}\n\
             Next agent: {}",
            agent_index + 1,
            total_agents,
            agent.phase,
            phase_name,
            prev_agent,
            next_agent,
        )
    }

    fn build_prior_context(
        &self,
        agent: &super::agents::ResearchAgent,
        prior_context: &str,
    ) -> String {
        let max_chars = if agent.key == "chapter-synthesizer" {
            120_000
        } else if agent.phase >= 7 {
            80_000
        } else {
            10_000
        };
        let escaped = escape_backticks(prior_context);
        let truncated = safe_truncate(&escaped, max_chars);
        format!(
            "## Prior Context\n\
             The following context has been injected by the Archon runtime from memory, \
             accepted prior agent outputs, and deterministic pre-scans where applicable. \
             Treat it as available evidence for this agent call; do not claim that memory \
             or filesystem access is unavailable when the needed data appears here.\n\n\
             {}",
            truncated,
        )
    }

    fn build_output_expectations(&self, agent: &super::agents::ResearchAgent) -> String {
        let artifacts = agent.output_artifacts.join(", ");
        let store_key = agent
            .memory_keys
            .first()
            .copied()
            .unwrap_or("research/output");
        format!(
            "## Output Expectations\n\
             Expected artifacts: {}\n\
             Store output at: {}\n\
             Tags: [\"phd-pipeline\", \"project/research\"]",
            artifacts, store_key,
        )
    }

    fn build_task_completion(&self, task: &str) -> String {
        format!(
            "## Task Completion\n\
             Research query: \"{}\"\n\
             Quality threshold: agent output will be assessed for content depth, \
             structural quality, research rigor, completeness, and format quality.\n\
             Success criteria: produce a comprehensive, well-structured output \
             addressing the agent's specific role in the research pipeline.",
            task,
        )
    }
}

impl Default for ResearchPromptBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
#[path = "prompt_builder_tests.rs"]
mod tests;
