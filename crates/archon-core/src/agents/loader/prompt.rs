use tracing::warn;

pub fn extract_description(agent_md: &str) -> String {
    let lines: Vec<&str> = agent_md.lines().collect();

    // Look for ## INTENT section
    let intent_start = lines.iter().position(|l| {
        let trimmed = l.trim().to_uppercase();
        trimmed == "## INTENT" || trimmed.starts_with("## INTENT")
    });

    if let Some(start) = intent_start {
        let content_lines = &lines[start + 1..];
        let mut paragraph = String::new();

        for line in content_lines {
            // Stop at next heading
            if line.trim_start().starts_with("##") {
                break;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                if !paragraph.is_empty() {
                    break; // End of first paragraph
                }
                continue;
            }
            if !paragraph.is_empty() {
                paragraph.push(' ');
            }
            paragraph.push_str(trimmed);
        }

        if !paragraph.is_empty() {
            return paragraph;
        }
    }

    // Fallback: first line stripped of '#'
    agent_md
        .lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().trim_start_matches('#').trim().to_string())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// System prompt assembly + truncation
// ---------------------------------------------------------------------------

/// Maximum agent prompt budget in tokens. 1 token ~= 4 chars (conservative).
const MAX_AGENT_PROMPT_TOKENS: usize = 8192;
const CHARS_PER_TOKEN: usize = 4;

pub(super) fn assemble_system_prompt(
    agent_md: &str,
    behavior_md: &str,
    context_md: &str,
) -> String {
    truncate_agent_prompt(
        agent_md.trim(),
        behavior_md.trim(),
        context_md.trim(),
        MAX_AGENT_PROMPT_TOKENS,
    )
}

/// Find the largest byte index <= `idx` that is a valid char boundary.
fn floor_char_boundary(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    let mut i = idx;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Assemble and truncate agent prompt with priority cascade:
/// 1. context_md trimmed first (least critical)
/// 2. behavior_md trimmed second
/// 3. agent_md NEVER trimmed (core identity)
pub fn truncate_agent_prompt(
    agent_md: &str,
    behavior_md: &str,
    context_md: &str,
    max_tokens: usize,
) -> String {
    let max_chars = max_tokens * CHARS_PER_TOKEN;
    // Account for "\n\n" separators (up to 4 chars for 2 separators)
    let separator_chars = 4;
    let total = agent_md.len() + behavior_md.len() + context_md.len() + separator_chars;

    if total <= max_chars {
        let mut parts = Vec::new();
        if !agent_md.is_empty() {
            parts.push(agent_md);
        }
        if !behavior_md.is_empty() {
            parts.push(behavior_md);
        }
        if !context_md.is_empty() {
            parts.push(context_md);
        }
        return parts.join("\n\n");
    }

    // Budget remaining after agent_md (never trimmed) + separators
    let remaining = max_chars.saturating_sub(agent_md.len() + separator_chars);

    // Try trimming context_md first
    let context_budget = remaining.saturating_sub(behavior_md.len());
    let trimmed_context = if context_md.len() > context_budget {
        warn!(
            trimmed_section = "context.md",
            original_len = context_md.len(),
            budget = context_budget,
            "truncating agent prompt"
        );
        if context_budget > 20 {
            let end = floor_char_boundary(context_md, context_budget.saturating_sub(16));
            format!("{}... [truncated]", &context_md[..end])
        } else {
            String::new()
        }
    } else {
        context_md.to_string()
    };

    // If still over, trim behavior_md
    let behavior_budget = remaining.saturating_sub(trimmed_context.len());
    let trimmed_behavior = if behavior_md.len() > behavior_budget {
        warn!(
            trimmed_section = "behavior.md",
            original_len = behavior_md.len(),
            budget = behavior_budget,
            "truncating agent prompt"
        );
        if behavior_budget > 20 {
            let end = floor_char_boundary(behavior_md, behavior_budget.saturating_sub(16));
            format!("{}... [truncated]", &behavior_md[..end])
        } else {
            String::new()
        }
    } else {
        behavior_md.to_string()
    };

    let mut parts = Vec::new();
    if !agent_md.is_empty() {
        parts.push(agent_md.to_string());
    }
    if !trimmed_behavior.is_empty() {
        parts.push(trimmed_behavior);
    }
    if !trimmed_context.is_empty() {
        parts.push(trimmed_context);
    }
    parts.join("\n\n")
}

// ---------------------------------------------------------------------------
// Tool extraction
// ---------------------------------------------------------------------------

/// Extract tools from `## Primary Tools` section in tools.md.
/// Returns None if no tools found (meaning all tools allowed).
pub fn extract_tools(tools_md: &str) -> Option<Vec<String>> {
    let lines: Vec<&str> = tools_md.lines().collect();

    let section_start = lines.iter().position(|l| {
        let trimmed = l.trim().to_uppercase();
        trimmed.starts_with("## PRIMARY TOOLS")
    });

    let start = match section_start {
        Some(idx) => idx + 1,
        None => return None,
    };

    let mut tools = Vec::new();
    for line in &lines[start..] {
        let trimmed = line.trim();
        // Stop at next heading
        if trimmed.starts_with("##") {
            break;
        }
        // Parse bullet items: "- **Glob**: description" or "- Glob"
        if let Some(rest) = trimmed.strip_prefix("- ") {
            let name = rest
                .trim_start_matches("**")
                .split(['*', ':', ' '])
                .next()
                .unwrap_or("")
                .trim();
            if !name.is_empty() {
                tools.push(name.to_string());
            }
        }
    }

    if tools.is_empty() { None } else { Some(tools) }
}

/// Extract tool usage guidance from tools.md — everything outside the
/// `## Primary Tools` section. This captures workflow instructions, usage
/// patterns, and constraints that agents should follow when using tools.
pub fn extract_tool_guidance(tools_md: &str) -> String {
    if tools_md.trim().is_empty() {
        return String::new();
    }

    let lines: Vec<&str> = tools_md.lines().collect();

    // Find the Primary Tools section bounds
    let section_start = lines
        .iter()
        .position(|l| l.trim().to_uppercase().starts_with("## PRIMARY TOOLS"));

    let (section_start_idx, section_end_idx) = if let Some(start) = section_start {
        // Find end of section (next ## heading or EOF)
        let end = lines[start + 1..]
            .iter()
            .position(|l| l.trim().starts_with("##"))
            .map(|i| i + start + 1)
            .unwrap_or(lines.len());
        (start, end)
    } else {
        // No Primary Tools section — everything is guidance
        (lines.len(), lines.len())
    };

    // Collect lines outside the Primary Tools section
    let mut guidance_lines: Vec<&str> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if i < section_start_idx || i >= section_end_idx {
            guidance_lines.push(line);
        }
    }

    guidance_lines.join("\n").trim().to_string()
}
