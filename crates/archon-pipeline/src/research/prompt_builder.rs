//! 5-part prompt builder for research pipeline agents.
//!
//! Assembles agent prompts from five sections: agent instructions, workflow
//! context, prior context (with safe truncation), output expectations, and
//! task completion summary. Phase 6 agents additionally receive style
//! injection via [`StyleInjector`].

use super::agents::{RESEARCH_AGENTS, get_phase_by_id};
use super::style::StyleInjector;

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
pub struct ResearchPromptBuilder;

impl ResearchPromptBuilder {
    pub fn new() -> Self {
        Self
    }

    /// Build the full prompt for the given research agent.
    ///
    /// # Arguments
    /// * `agent` – the research agent definition
    /// * `agent_index` – 0-based position in the pipeline
    /// * `total_agents` – total agent count (typically 46)
    /// * `task` – the research query / task description
    /// * `prior_context` – recalled memory content from previous agents
    /// * `style_prompt` – optional style override for Phase 6 agents
    pub fn build(
        &self,
        agent: &super::agents::ResearchAgent,
        agent_index: usize,
        total_agents: usize,
        task: &str,
        prior_context: &str,
        style_prompt: Option<&str>,
    ) -> String {
        let mut parts: Vec<String> = Vec::with_capacity(5);

        // Part 1: Agent Instructions
        parts.push(self.build_agent_instructions(agent));

        // Part 2: Workflow Context
        parts.push(self.build_workflow_context(agent, agent_index, total_agents));

        // Part 3: Prior Context (omitted if empty)
        if !prior_context.is_empty() {
            parts.push(self.build_prior_context(prior_context));
        }

        // Part 4: Output Expectations
        parts.push(self.build_output_expectations(agent));

        // Part 5: Task Completion Summary
        parts.push(self.build_task_completion(task));

        let mut prompt = parts.join("\n\n");

        // Phase 6: inject style guidelines
        if agent.phase == 6 {
            if let Some(style) = style_prompt {
                let injector = StyleInjector::new();
                prompt = injector.build_styled_prompt(&prompt, style);
            }
        }

        prompt
    }

    // -----------------------------------------------------------------------
    // Part builders
    // -----------------------------------------------------------------------

    fn build_agent_instructions(&self, agent: &super::agents::ResearchAgent) -> String {
        // Try to load from file, parsing frontmatter to get just the body
        let path = agent.prompt_source_path;
        if let Ok(content) = std::fs::read_to_string(path) {
            if !content.trim().is_empty() {
                if let Ok((_frontmatter, body)) = crate::agent_loader::parse_frontmatter(&content) {
                    if !body.trim().is_empty() {
                        return body;
                    }
                }
                // Fallback: if frontmatter parsing fails, use raw content
                return content;
            }
        }
        format!(
            "You are the {} agent for the PhD research pipeline.",
            agent.display_name,
        )
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

    fn build_prior_context(&self, prior_context: &str) -> String {
        let escaped = escape_backticks(prior_context);
        let truncated = safe_truncate(&escaped, 10_000);
        format!(
            "## Prior Context\n\
             The following context has been recalled from previous agent outputs:\n\n\
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
mod tests {
    use super::*;
    use crate::research::agents::RESEARCH_AGENTS;

    fn builder() -> ResearchPromptBuilder {
        ResearchPromptBuilder::new()
    }

    // 1. Prompt has all 5 sections for a Phase 4 agent
    #[test]
    fn prompt_has_all_5_sections_phase4() {
        let b = builder();
        // evidence-synthesizer is index 15, Phase 4
        let agent = &RESEARCH_AGENTS[15];
        assert_eq!(agent.key, "evidence-synthesizer");

        let prompt = b.build(
            agent,
            15,
            46,
            "AI in healthcare",
            "some prior context",
            None,
        );

        assert!(
            prompt.contains("## Workflow Context"),
            "missing workflow context"
        );
        assert!(prompt.contains("## Prior Context"), "missing prior context");
        assert!(
            prompt.contains("## Output Expectations"),
            "missing output expectations"
        );
        assert!(
            prompt.contains("## Task Completion"),
            "missing task completion"
        );
        // Part 1 is the agent instructions (either file content or fallback)
        assert!(
            prompt.contains("Evidence Synthesizer") || prompt.contains("evidence-synthesizer"),
            "missing agent instructions reference"
        );
    }

    // 2. Workflow context shows correct position
    #[test]
    fn workflow_context_correct_position() {
        let b = builder();
        let agent = &RESEARCH_AGENTS[15];
        let prompt = b.build(agent, 15, 46, "test query", "", None);

        assert!(
            prompt.contains("Agent #16 of 46"),
            "should show 1-based index: Agent #16 of 46"
        );
        assert!(
            prompt.contains("Phase: 4 - Synthesis"),
            "should show phase name"
        );
    }

    // 3. Prior context truncated at 10000 chars with marker
    #[test]
    fn prior_context_truncation() {
        let b = builder();
        let agent = &RESEARCH_AGENTS[0];
        let long_context: String = "x".repeat(15_000);
        let prompt = b.build(agent, 0, 46, "test", &long_context, None);

        assert!(
            prompt.contains("... [truncated]"),
            "should have truncation marker"
        );
        // The prior context section should not contain all 15000 chars
        let prior_section = prompt.split("## Prior Context").nth(1).unwrap_or("");
        // Truncated to 10000 + marker
        assert!(
            prior_section.chars().count() < 12_000,
            "truncated section should be well under 15000 chars"
        );
    }

    // 4. Empty prior context omits the section
    #[test]
    fn empty_prior_context_omitted() {
        let b = builder();
        let agent = &RESEARCH_AGENTS[0];
        let prompt = b.build(agent, 0, 46, "test", "", None);

        assert!(
            !prompt.contains("## Prior Context"),
            "empty prior context should be omitted"
        );
    }

    // 5. Backtick escaping in prior context
    #[test]
    fn backtick_escaping() {
        let b = builder();
        let agent = &RESEARCH_AGENTS[0];
        let context_with_backticks = "Here is code: ```python\nprint('hello')\n```";
        let prompt = b.build(agent, 0, 46, "test", context_with_backticks, None);

        assert!(
            !prompt.contains("```python"),
            "triple backticks should be escaped"
        );
        assert!(
            prompt.contains("\\`\\`\\`python"),
            "should contain escaped backticks"
        );
    }

    // 6. Output expectations include memory_keys[0]
    #[test]
    fn output_expectations_memory_key() {
        let b = builder();
        let agent = &RESEARCH_AGENTS[0]; // step-back-analyzer
        let prompt = b.build(agent, 0, 46, "test", "", None);

        assert!(
            prompt.contains("Store output at: research/foundation/framing"),
            "should reference memory_keys[0]"
        );
        assert!(
            prompt.contains("Expected artifacts: high-level-framing.md, abstraction-analysis.md"),
            "should list output artifacts"
        );
    }

    // 7. Phase 6 agent gets style injection
    #[test]
    fn phase6_style_injection() {
        let b = builder();
        // introduction-writer is index 29, Phase 6
        let agent = &RESEARCH_AGENTS[29];
        assert_eq!(agent.key, "introduction-writer");
        assert_eq!(agent.phase, 6);

        let prompt = b.build(
            agent,
            29,
            46,
            "test",
            "",
            Some("Use American English spelling conventions"),
        );

        assert!(
            prompt.contains("## STYLE GUIDELINES"),
            "Phase 6 should have style guidelines"
        );
        assert!(
            prompt.contains("American English"),
            "should include style content"
        );
    }

    // 8. Missing agent instruction file uses fallback
    #[test]
    fn missing_instruction_file_fallback() {
        let b = builder();
        let agent = &RESEARCH_AGENTS[0];
        // The prompt_source_path likely doesn't exist in the test environment
        // so it should fall back to the default message
        let prompt = b.build(agent, 0, 46, "test", "", None);

        // Either the file was found OR we got the fallback
        assert!(
            prompt.contains("Step-Back Analyzer") || prompt.contains("step-back-analyzer"),
            "should contain agent reference either from file or fallback"
        );
    }

    // 9. safe_truncate preserves char boundaries (multi-byte)
    #[test]
    fn safe_truncate_char_boundary() {
        // Each emoji is multiple bytes but one char
        let text = "🎉🎊🎈🎆🎇"; // 5 chars
        let result = safe_truncate(text, 3);
        assert!(result.starts_with("🎉🎊🎈"));
        assert!(result.contains("... [truncated]"));
    }

    // 10. safe_truncate no-op when under limit
    #[test]
    fn safe_truncate_no_op() {
        let text = "short";
        let result = safe_truncate(text, 100);
        assert_eq!(result, "short");
    }

    // 11. escape_backticks replaces triple backticks
    #[test]
    fn escape_backticks_works() {
        let input = "before ``` middle ``` after";
        let result = escape_backticks(input);
        assert_eq!(result, "before \\`\\`\\` middle \\`\\`\\` after");
    }

    // 12. Phase 6 without style_prompt does NOT get STYLE GUIDELINES
    #[test]
    fn phase6_no_style_prompt_no_injection() {
        let b = builder();
        let agent = &RESEARCH_AGENTS[29]; // introduction-writer, Phase 6
        let prompt = b.build(agent, 29, 46, "test", "", None);
        assert!(
            !prompt.contains("## STYLE GUIDELINES"),
            "no style_prompt means no STYLE GUIDELINES section"
        );
    }

    // 13. Workflow context shows previous and next agents
    #[test]
    fn workflow_context_prev_next() {
        let b = builder();
        // Agent at index 1 (self-ask-decomposer)
        let agent = &RESEARCH_AGENTS[1];
        let prompt = b.build(agent, 1, 46, "test", "", None);

        assert!(
            prompt.contains("Previous agent: step-back-analyzer"),
            "should show previous agent key"
        );
        assert!(
            prompt.contains("Next agent: ambiguity-clarifier"),
            "should show next agent key"
        );
    }

    // 14. First agent shows "none (first agent)"
    #[test]
    fn first_agent_prev_none() {
        let b = builder();
        let agent = &RESEARCH_AGENTS[0];
        let prompt = b.build(agent, 0, 46, "test", "", None);
        assert!(prompt.contains("Previous agent: none (first agent)"));
    }

    // 15. Last agent shows "none (final agent)"
    #[test]
    fn last_agent_next_none() {
        let b = builder();
        let last_idx = RESEARCH_AGENTS.len() - 1;
        let agent = &RESEARCH_AGENTS[last_idx];
        let prompt = b.build(agent, last_idx, 46, "test", "", None);
        assert!(prompt.contains("Next agent: none (final agent)"));
    }
}
