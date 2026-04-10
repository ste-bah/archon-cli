use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A single section of the assembled system prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptSection {
    /// The text content of this section.
    pub content: String,
    /// Optional cache-control hint (e.g. "ephemeral" for non-cacheable sections).
    pub cache_control: Option<String>,
}

/// Budget configuration controlling how many tokens specific sections may use.
#[derive(Debug, Clone)]
pub struct BudgetConfig {
    /// Maximum tokens for the rules section.
    pub rules_tokens: usize,
    /// Maximum tokens for the memories section.
    pub memories_tokens: usize,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            rules_tokens: 2048,
            memories_tokens: 4096,
        }
    }
}

/// Input sections for system prompt assembly. All fields are optional;
/// missing sections are simply omitted from the output.
#[derive(Debug, Clone, Default)]
pub struct AssemblyInput {
    pub identity: Option<String>,
    pub personality: Option<String>,
    pub rules: Option<String>,
    pub memories: Option<String>,
    pub user_prompt: Option<String>,
    pub project_instructions: Option<String>,
    pub environment: Option<String>,
    /// Inner voice / consciousness state block.
    pub inner_voice: Option<String>,
    /// Personality briefing from cross-session persistence (first turn only).
    pub personality_briefing: Option<String>,
    /// Memory garden briefing (first turn only).
    pub memory_briefing: Option<String>,
    /// Dynamic/ephemeral content such as current date and session info.
    pub dynamic: Option<String>,
}

// ---------------------------------------------------------------------------
// Assembler
// ---------------------------------------------------------------------------

/// Assembles a list of [`PromptSection`]s from an [`AssemblyInput`],
/// applying budget limits and section ordering.
///
/// Section ordering (G6a — cache marker placement):
///  1. Identity (stable)
///  2. Personality (truncated to 200 tokens, stable)
///  3. Rules (truncated to budget, stable)
///  4. Memories (truncated to budget, stable-ish)
///  5. Project instructions / ARCHON.md (stable)
///  6. Environment (stable) — carries the single
///     `cache_control: Some("ephemeral")` marker. Anthropic treats this as a
///     cache checkpoint: everything up to and INCLUDING this block is cached.
///     Placing the marker here guarantees the cached prefix covers all the
///     stable sections and nothing turn-variable.
///  7. Personality briefing (turn-variable, first turn only) — no marker
///  8. Memory briefing (turn-variable, first turn only) — no marker
///  9. User prompt (turn-variable) — no marker
/// 10. Inner voice (turn-variable) — no marker (was ephemeral pre-G6a)
/// 11. Dynamic (date + session, turn-variable) — no marker (was ephemeral pre-G6a)
pub struct SystemPromptAssembler {
    budget: BudgetConfig,
}

impl SystemPromptAssembler {
    pub fn new(budget: BudgetConfig) -> Self {
        Self { budget }
    }

    /// Assemble sections from the given input.
    pub fn assemble(&self, input: &AssemblyInput) -> Vec<PromptSection> {
        let mut sections = Vec::new();

        // 1. Identity
        if let Some(ref text) = input.identity
            && !text.is_empty()
        {
            sections.push(PromptSection {
                content: text.clone(),
                cache_control: None,
            });
        }

        // 2. Personality — truncated to ~200 tokens
        if let Some(ref text) = input.personality
            && !text.is_empty()
        {
            sections.push(PromptSection {
                content: truncate_to_tokens(text, 200),
                cache_control: None,
            });
        }

        // 3. Rules — truncated to budget
        if let Some(ref text) = input.rules
            && !text.is_empty()
        {
            sections.push(PromptSection {
                content: truncate_to_tokens(text, self.budget.rules_tokens),
                cache_control: None,
            });
        }

        // 4. Memories — truncated to budget
        if let Some(ref text) = input.memories
            && !text.is_empty()
        {
            sections.push(PromptSection {
                content: truncate_to_tokens(text, self.budget.memories_tokens),
                cache_control: None,
            });
        }

        // 5. Project instructions (ARCHON.md) — stable, no marker.
        if let Some(ref text) = input.project_instructions
            && !text.is_empty()
        {
            sections.push(PromptSection {
                content: text.clone(),
                cache_control: None,
            });
        }

        // 6. Environment (stable) — carries the SINGLE ephemeral cache marker.
        //    Anthropic caches everything up to and including this block, so
        //    every stable section above is cached on subsequent turns. This is
        //    the G6a fix: the marker used to sit on inner_voice/dynamic, which
        //    polluted the cached prefix with turn-variable content.
        if let Some(ref text) = input.environment
            && !text.is_empty()
        {
            sections.push(PromptSection {
                content: text.clone(),
                cache_control: Some("ephemeral".to_string()),
            });
        }

        // 7. Personality briefing (first turn only) — turn-variable, no marker.
        if let Some(ref text) = input.personality_briefing
            && !text.is_empty()
        {
            sections.push(PromptSection {
                content: text.clone(),
                cache_control: None,
            });
        }

        // 8. Memory briefing (first turn only) — turn-variable, no marker.
        if let Some(ref text) = input.memory_briefing
            && !text.is_empty()
        {
            sections.push(PromptSection {
                content: text.clone(),
                cache_control: None,
            });
        }

        // 9. User prompt — turn-variable, no marker.
        if let Some(ref text) = input.user_prompt
            && !text.is_empty()
        {
            sections.push(PromptSection {
                content: text.clone(),
                cache_control: None,
            });
        }

        // 10. Inner voice — turn-variable, no marker (was ephemeral pre-G6a).
        if let Some(ref text) = input.inner_voice
            && !text.is_empty()
        {
            sections.push(PromptSection {
                content: text.clone(),
                cache_control: None,
            });
        }

        // 11. Dynamic (date + session) — turn-variable, no marker (was ephemeral pre-G6a).
        if let Some(ref text) = input.dynamic
            && !text.is_empty()
        {
            sections.push(PromptSection {
                content: text.clone(),
                cache_control: None,
            });
        }

        sections
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Approximate token truncation. Uses a simple heuristic of ~4 chars per token
/// (a common approximation for English text). Truncates at a word boundary
/// when possible.
fn truncate_to_tokens(text: &str, max_tokens: usize) -> String {
    let max_chars = max_tokens * 4;
    if text.len() <= max_chars {
        return text.to_string();
    }

    // Find a word boundary near the limit.
    let truncated = &text[..max_chars];
    if let Some(last_space) = truncated.rfind(' ') {
        format!("{}…", &text[..last_space])
    } else {
        format!("{}…", truncated)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn assembler() -> SystemPromptAssembler {
        SystemPromptAssembler::new(BudgetConfig::default())
    }

    #[test]
    fn all_sections_present() {
        let input = AssemblyInput {
            identity: Some("I am Archon.".into()),
            personality: Some("Friendly and helpful.".into()),
            rules: Some("Rule 1. Rule 2.".into()),
            memories: Some("User prefers Rust.".into()),
            user_prompt: Some("Write a function.".into()),
            project_instructions: Some("See ARCHON.md".into()),
            environment: Some("Linux x86_64".into()),
            inner_voice: Some("<inner_voice>state</inner_voice>".into()),
            personality_briefing: Some("<personality_briefing>data</personality_briefing>".into()),
            memory_briefing: Some("<memory_briefing>data</memory_briefing>".into()),
            dynamic: Some("Date: 2026-04-02".into()),
        };

        let sections = assembler().assemble(&input);
        assert_eq!(sections.len(), 11);
    }

    #[test]
    fn correct_ordering() {
        // G6a reorder (see docs/audit-gap-fixes-plan.md §G6a):
        // project_instructions and environment were moved BEFORE the turn-variable
        // sections (personality_briefing, memory_briefing, user_prompt) so that the
        // single Anthropic `cache_control: ephemeral` marker can sit on `environment`
        // (the last stable section) and cache everything up to and including it.
        //
        // New order:
        //   0 identity, 1 personality, 2 rules, 3 memories,
        //   4 project_instructions, 5 environment (cache marker),
        //   6 personality_briefing, 7 memory_briefing, 8 user_prompt,
        //   9 inner_voice, 10 dynamic
        let input = AssemblyInput {
            identity: Some("identity".into()),
            personality: Some("personality".into()),
            rules: Some("rules".into()),
            memories: Some("memories".into()),
            user_prompt: Some("user".into()),
            project_instructions: Some("project".into()),
            environment: Some("env".into()),
            inner_voice: Some("voice".into()),
            personality_briefing: Some("pbriefing".into()),
            memory_briefing: Some("mbriefing".into()),
            dynamic: Some("dynamic".into()),
        };

        let sections = assembler().assemble(&input);
        assert_eq!(sections[0].content, "identity");
        assert_eq!(sections[1].content, "personality");
        assert_eq!(sections[2].content, "rules");
        assert_eq!(sections[3].content, "memories");
        assert_eq!(sections[4].content, "project");
        assert_eq!(sections[5].content, "env");
        assert_eq!(sections[6].content, "pbriefing");
        assert_eq!(sections[7].content, "mbriefing");
        assert_eq!(sections[8].content, "user");
        assert_eq!(sections[9].content, "voice");
        assert_eq!(sections[10].content, "dynamic");
    }

    #[test]
    fn cache_marker_on_environment_only() {
        // G6a: The single Anthropic `cache_control: ephemeral` marker must sit on
        // the `environment` section, which is the last stable section in the
        // prefix. Turn-variable sections (inner_voice, dynamic, project_instructions
        // — the last is stable but not the designated marker) must carry
        // `cache_control: None`, and the marker must appear exactly once.
        let input = AssemblyInput {
            identity: Some("identity".into()),
            personality: Some("personality".into()),
            rules: Some("rules".into()),
            memories: Some("memories".into()),
            user_prompt: Some("user".into()),
            project_instructions: Some("project-instructions".into()),
            environment: Some("environment".into()),
            inner_voice: Some("inner-voice".into()),
            personality_briefing: Some("pbriefing".into()),
            memory_briefing: Some("mbriefing".into()),
            dynamic: Some("dynamic".into()),
        };

        let sections = assembler().assemble(&input);

        // Exactly one ephemeral marker.
        let marker_count = sections
            .iter()
            .filter(|s| s.cache_control.as_deref() == Some("ephemeral"))
            .count();
        assert_eq!(
            marker_count, 1,
            "expected exactly one ephemeral cache marker, found {marker_count}"
        );

        // The marker sits on the environment section.
        let env_section = sections
            .iter()
            .find(|s| s.content == "environment")
            .expect("environment section missing");
        assert_eq!(
            env_section.cache_control,
            Some("ephemeral".to_string()),
            "environment section must carry the ephemeral cache marker"
        );

        // Turn-variable and other stable sections must NOT carry the marker.
        let inner_voice_section = sections
            .iter()
            .find(|s| s.content == "inner-voice")
            .expect("inner_voice section missing");
        assert_eq!(inner_voice_section.cache_control, None);

        let dynamic_section = sections
            .iter()
            .find(|s| s.content == "dynamic")
            .expect("dynamic section missing");
        assert_eq!(dynamic_section.cache_control, None);

        let project_section = sections
            .iter()
            .find(|s| s.content == "project-instructions")
            .expect("project_instructions section missing");
        assert_eq!(project_section.cache_control, None);
    }

    #[test]
    fn prefix_stable_across_turn_variable_changes() {
        // G6a prefix stability: two assemblies with identical stable fields
        // (identity, personality, rules, memories, project_instructions,
        // environment) but DIFFERENT turn-variable fields must produce an
        // identical prefix up to and including the ephemeral marker. This is
        // what lets Anthropic's prompt cache actually hit across turns.
        let stable_identity = "identity-stable";
        let stable_personality = "personality-stable";
        let stable_rules = "rules-stable";
        let stable_memories = "memories-stable";
        let stable_project = "project-stable";
        let stable_env = "env-stable";

        let input_a = AssemblyInput {
            identity: Some(stable_identity.into()),
            personality: Some(stable_personality.into()),
            rules: Some(stable_rules.into()),
            memories: Some(stable_memories.into()),
            project_instructions: Some(stable_project.into()),
            environment: Some(stable_env.into()),
            user_prompt: Some("turn-A-user".into()),
            personality_briefing: Some("turn-A-pbrief".into()),
            memory_briefing: Some("turn-A-mbrief".into()),
            inner_voice: Some("turn-A-voice".into()),
            dynamic: Some("turn-A-dynamic".into()),
        };

        let input_b = AssemblyInput {
            identity: Some(stable_identity.into()),
            personality: Some(stable_personality.into()),
            rules: Some(stable_rules.into()),
            memories: Some(stable_memories.into()),
            project_instructions: Some(stable_project.into()),
            environment: Some(stable_env.into()),
            user_prompt: Some("turn-B-user-totally-different".into()),
            personality_briefing: Some("turn-B-pbrief-different".into()),
            memory_briefing: Some("turn-B-mbrief-different".into()),
            inner_voice: Some("turn-B-voice-different".into()),
            dynamic: Some("turn-B-dynamic-different".into()),
        };

        let asm = assembler();
        let sections_a = asm.assemble(&input_a);
        let sections_b = asm.assemble(&input_b);

        let marker_idx_a = sections_a
            .iter()
            .position(|s| s.cache_control.as_deref() == Some("ephemeral"))
            .expect("no ephemeral marker in output A");
        let marker_idx_b = sections_b
            .iter()
            .position(|s| s.cache_control.as_deref() == Some("ephemeral"))
            .expect("no ephemeral marker in output B");

        assert_eq!(
            marker_idx_a, marker_idx_b,
            "ephemeral marker index drifted between turns: A={marker_idx_a} B={marker_idx_b}"
        );

        // Prefix (everything up to and including the marker) must be byte-for-byte
        // identical: same count, same content, same cache_control per index.
        let prefix_a = &sections_a[0..=marker_idx_a];
        let prefix_b = &sections_b[0..=marker_idx_b];
        assert_eq!(
            prefix_a.len(),
            prefix_b.len(),
            "prefix length differs between turns"
        );
        for (i, (a, b)) in prefix_a.iter().zip(prefix_b.iter()).enumerate() {
            assert_eq!(
                a.content, b.content,
                "prefix section {i} content differs: A={:?} B={:?}",
                a.content, b.content
            );
            assert_eq!(
                a.cache_control, b.cache_control,
                "prefix section {i} cache_control differs: A={:?} B={:?}",
                a.cache_control, b.cache_control
            );
        }
    }

    #[test]
    fn missing_sections_produce_no_gaps() {
        let input = AssemblyInput {
            identity: Some("id".into()),
            rules: Some("rules".into()),
            environment: Some("env".into()),
            ..Default::default()
        };

        let sections = assembler().assemble(&input);
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].content, "id");
        assert_eq!(sections[1].content, "rules");
        assert_eq!(sections[2].content, "env");
    }

    #[test]
    fn empty_string_sections_are_skipped() {
        let input = AssemblyInput {
            identity: Some(String::new()),
            personality: Some("present".into()),
            ..Default::default()
        };

        let sections = assembler().assemble(&input);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].content, "present");
    }

    #[test]
    fn personality_truncated_to_200_tokens() {
        // 200 tokens ~= 800 chars. Create a string longer than that.
        let long_text = "word ".repeat(300); // 1500 chars
        let input = AssemblyInput {
            personality: Some(long_text.clone()),
            ..Default::default()
        };

        let sections = assembler().assemble(&input);
        assert_eq!(sections.len(), 1);
        // Should be truncated: shorter than original and ends with ellipsis.
        assert!(sections[0].content.len() < long_text.len());
        assert!(sections[0].content.ends_with('…'));
    }

    #[test]
    fn rules_truncated_to_budget() {
        let budget = BudgetConfig {
            rules_tokens: 10, // ~40 chars
            ..Default::default()
        };
        let asm = SystemPromptAssembler::new(budget);

        let long_rules = "a ".repeat(100); // 200 chars
        let input = AssemblyInput {
            rules: Some(long_rules.clone()),
            ..Default::default()
        };

        let sections = asm.assemble(&input);
        assert_eq!(sections.len(), 1);
        assert!(sections[0].content.len() < long_rules.len());
    }

    #[test]
    fn memories_truncated_to_budget() {
        let budget = BudgetConfig {
            memories_tokens: 5, // ~20 chars
            ..Default::default()
        };
        let asm = SystemPromptAssembler::new(budget);

        let long_mem = "memory ".repeat(50);
        let input = AssemblyInput {
            memories: Some(long_mem.clone()),
            ..Default::default()
        };

        let sections = asm.assemble(&input);
        assert_eq!(sections.len(), 1);
        assert!(sections[0].content.len() < long_mem.len());
    }

    #[test]
    fn dynamic_section_is_no_longer_ephemeral() {
        // G6a: Previously the `dynamic` section carried the ephemeral cache
        // marker. Under the new layout the single marker lives on `environment`
        // (the last stable section), and `dynamic` — being turn-variable — must
        // no longer carry any cache_control, so it cannot pollute the cached
        // prefix.
        let input = AssemblyInput {
            dynamic: Some("2026-04-02 session-abc".into()),
            ..Default::default()
        };

        let sections = assembler().assemble(&input);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].cache_control, None);
    }

    #[test]
    fn identity_and_rules_have_no_cache_marker() {
        // G6a: The ephemeral marker lives exclusively on `environment`. All
        // other stable sections (identity, personality, rules, memories,
        // project_instructions) must carry `cache_control: None` — they are
        // still cacheable via the single checkpoint placed on environment, but
        // they are not themselves the marker.
        let input = AssemblyInput {
            identity: Some("id".into()),
            rules: Some("rules".into()),
            ..Default::default()
        };

        let sections = assembler().assemble(&input);
        for section in &sections {
            assert_eq!(section.cache_control, None);
        }
    }

    #[test]
    fn short_text_not_truncated() {
        let short = "Hello world";
        let result = truncate_to_tokens(short, 100);
        assert_eq!(result, short);
    }
}
