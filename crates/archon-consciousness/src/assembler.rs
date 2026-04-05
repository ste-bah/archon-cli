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
    /// Dynamic/ephemeral content such as current date and session info.
    pub dynamic: Option<String>,
}

// ---------------------------------------------------------------------------
// Assembler
// ---------------------------------------------------------------------------

/// Assembles a list of [`PromptSection`]s from an [`AssemblyInput`],
/// applying budget limits and section ordering.
///
/// Section ordering:
/// 1. Identity
/// 2. Personality (truncated to 200 tokens)
/// 3. Rules (truncated to budget)
/// 4. Memories (truncated to budget)
/// 5. User prompt
/// 6. Project instructions (CLAUDE.md)
/// 7. Environment
/// 8. Inner voice (ephemeral)
/// 9. Dynamic (date + session) — marked ephemeral
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
        if let Some(ref text) = input.identity {
            if !text.is_empty() {
                sections.push(PromptSection {
                    content: text.clone(),
                    cache_control: None,
                });
            }
        }

        // 2. Personality — truncated to ~200 tokens
        if let Some(ref text) = input.personality {
            if !text.is_empty() {
                sections.push(PromptSection {
                    content: truncate_to_tokens(text, 200),
                    cache_control: None,
                });
            }
        }

        // 3. Rules — truncated to budget
        if let Some(ref text) = input.rules {
            if !text.is_empty() {
                sections.push(PromptSection {
                    content: truncate_to_tokens(text, self.budget.rules_tokens),
                    cache_control: None,
                });
            }
        }

        // 4. Memories — truncated to budget
        if let Some(ref text) = input.memories {
            if !text.is_empty() {
                sections.push(PromptSection {
                    content: truncate_to_tokens(text, self.budget.memories_tokens),
                    cache_control: None,
                });
            }
        }

        // 5. User prompt
        if let Some(ref text) = input.user_prompt {
            if !text.is_empty() {
                sections.push(PromptSection {
                    content: text.clone(),
                    cache_control: None,
                });
            }
        }

        // 6. Project instructions (CLAUDE.md)
        if let Some(ref text) = input.project_instructions {
            if !text.is_empty() {
                sections.push(PromptSection {
                    content: text.clone(),
                    cache_control: None,
                });
            }
        }

        // 7. Environment
        if let Some(ref text) = input.environment {
            if !text.is_empty() {
                sections.push(PromptSection {
                    content: text.clone(),
                    cache_control: None,
                });
            }
        }

        // 8. Inner voice (ephemeral — changes every turn)
        if let Some(ref text) = input.inner_voice {
            if !text.is_empty() {
                sections.push(PromptSection {
                    content: text.clone(),
                    cache_control: Some("ephemeral".to_string()),
                });
            }
        }

        // 9. Dynamic (ephemeral — not cacheable)
        if let Some(ref text) = input.dynamic {
            if !text.is_empty() {
                sections.push(PromptSection {
                    content: text.clone(),
                    cache_control: Some("ephemeral".to_string()),
                });
            }
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
            project_instructions: Some("See CLAUDE.md".into()),
            environment: Some("Linux x86_64".into()),
            inner_voice: Some("<inner_voice>state</inner_voice>".into()),
            dynamic: Some("Date: 2026-04-02".into()),
        };

        let sections = assembler().assemble(&input);
        assert_eq!(sections.len(), 9);
    }

    #[test]
    fn correct_ordering() {
        let input = AssemblyInput {
            identity: Some("identity".into()),
            personality: Some("personality".into()),
            rules: Some("rules".into()),
            memories: Some("memories".into()),
            user_prompt: Some("user".into()),
            project_instructions: Some("project".into()),
            environment: Some("env".into()),
            inner_voice: Some("voice".into()),
            dynamic: Some("dynamic".into()),
        };

        let sections = assembler().assemble(&input);
        assert_eq!(sections[0].content, "identity");
        assert_eq!(sections[1].content, "personality");
        assert_eq!(sections[2].content, "rules");
        assert_eq!(sections[3].content, "memories");
        assert_eq!(sections[4].content, "user");
        assert_eq!(sections[5].content, "project");
        assert_eq!(sections[6].content, "env");
        assert_eq!(sections[7].content, "voice");
        assert_eq!(sections[8].content, "dynamic");
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
    fn dynamic_section_is_ephemeral() {
        let input = AssemblyInput {
            dynamic: Some("2026-04-02 session-abc".into()),
            ..Default::default()
        };

        let sections = assembler().assemble(&input);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].cache_control, Some("ephemeral".to_string()));
    }

    #[test]
    fn non_dynamic_sections_are_cacheable() {
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
