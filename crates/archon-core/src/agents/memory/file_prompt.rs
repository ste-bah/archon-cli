use std::path::{Path, PathBuf};

use crate::agents::definition::AgentMemoryScope;

/// Maximum lines in MEMORY.md entrypoint before truncation.
pub const MAX_ENTRYPOINT_LINES: usize = 200;

/// Maximum bytes in MEMORY.md entrypoint before truncation.
/// Matches Claude Code's memdir.ts line 38: MAX_ENTRYPOINT_BYTES = 25_000.
pub const MAX_ENTRYPOINT_BYTES: usize = 25_000;

/// Resolve the memory directory for an agent based on scope.
///
/// Paths:
/// - User:    `~/.archon/agent-memory/<agent_type>/`
/// - Project: `<cwd>/.archon/agent-memory/<agent_type>/`
/// - Local:   `<cwd>/.archon/agent-memory-local/<agent_type>/`
///
/// Agent type colons are sanitized to dashes (Windows-safe, plugin-namespaced types).
pub fn get_agent_memory_dir(agent_type: &str, scope: &AgentMemoryScope, cwd: &Path) -> PathBuf {
    let safe_name = agent_type.replace(':', "-");
    match scope {
        AgentMemoryScope::User => dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".archon/agent-memory")
            .join(&safe_name),
        AgentMemoryScope::Project => cwd.join(".archon/agent-memory").join(&safe_name),
        AgentMemoryScope::Local => cwd.join(".archon/agent-memory-local").join(&safe_name),
    }
}

/// Ensure memory directory exists (lazy, fire-and-forget).
pub async fn ensure_memory_dir_exists(path: &Path) {
    if let Err(e) = tokio::fs::create_dir_all(path).await
        && e.kind() != std::io::ErrorKind::AlreadyExists
    {
        tracing::warn!(path = %path.display(), error = %e, "Failed to create memory dir");
    }
}

/// Truncate MEMORY.md content to line AND byte caps.
///
/// Line-truncates first, then byte-truncates at last newline before cap.
/// Appends warning if either cap fired.
pub fn truncate_entrypoint_content(raw: &str, max_lines: usize, max_bytes: usize) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let lines: Vec<&str> = trimmed.lines().collect();
    let line_truncated = lines.len() > max_lines;

    // Line-truncate first
    let mut result = if line_truncated {
        lines[..max_lines].join("\n")
    } else {
        trimmed.to_string()
    };

    // Then byte-truncate at last newline before cap
    let byte_truncated = result.len() > max_bytes;
    if byte_truncated {
        if let Some(cut_at) = result[..max_bytes].rfind('\n') {
            result.truncate(cut_at);
        } else {
            result.truncate(max_bytes);
        }
    }

    if !line_truncated && !byte_truncated {
        return result;
    }

    // Append warning naming which cap fired
    let reason = match (line_truncated, byte_truncated) {
        (true, true) => format!("{} lines and {} bytes", lines.len(), trimmed.len()),
        (true, false) => format!("{} lines (limit: {})", lines.len(), max_lines),
        (false, true) => format!("{} bytes (limit: {})", trimmed.len(), max_bytes),
        _ => unreachable!(),
    };
    format!(
        "{}\n\n> WARNING: MEMORY.md is {}. Only part loaded. Keep entries to one line under ~200 chars.",
        result, reason
    )
}

/// Build the full ~150-line memory prompt matching Claude Code's auto memory system.
///
/// Includes: 4-type taxonomy, what-not-to-save, how-to-save (2-step),
/// when-to-access, before-recommending verification, existing content.
pub fn build_full_memory_prompt(
    memory_dir: &Path,
    scope_guidance: &str,
    existing_memory: &str,
) -> String {
    let dir = memory_dir.display();
    let content = if existing_memory.is_empty() {
        "(No memories yet)".to_string()
    } else {
        existing_memory.to_string()
    };

    format!(
        r#"# auto memory

You have a persistent, file-based memory system at `{dir}`. This directory already exists — write to it directly with the Write tool (do not run mkdir or check for its existence).

{scope_guidance}

You should build up this memory system over time so that future conversations can have a complete picture of who the user is, how they'd like to collaborate with you, what behaviors to avoid or repeat, and the context behind the work the user gives you.

If the user explicitly asks you to remember something, save it immediately as whichever type fits best. If they ask you to forget something, find and remove the relevant entry.

## Types of memory

There are several discrete types of memory that you can store in your memory system:

<types>
<type>
    <name>user</name>
    <description>Contain information about the user's role, goals, responsibilities, and knowledge. Great user memories help you tailor your future behavior to the user's preferences and perspective.</description>
    <when_to_save>When you learn any details about the user's role, preferences, responsibilities, or knowledge</when_to_save>
    <how_to_use>When your work should be informed by the user's profile or perspective.</how_to_use>
</type>
<type>
    <name>feedback</name>
    <description>Guidance the user has given you about how to approach work — both what to avoid and what to keep doing. Record from failure AND success.</description>
    <when_to_save>Any time the user corrects your approach OR confirms a non-obvious approach worked.</when_to_save>
    <how_to_use>Let these memories guide your behavior so that the user does not need to offer the same guidance twice.</how_to_use>
    <body_structure>Lead with the rule itself, then a **Why:** line and a **How to apply:** line.</body_structure>
</type>
<type>
    <name>project</name>
    <description>Information about ongoing work, goals, initiatives, bugs, or incidents within the project that is not otherwise derivable from the code or git history.</description>
    <when_to_save>When you learn who is doing what, why, or by when. Always convert relative dates to absolute dates.</when_to_save>
    <how_to_use>Use these memories to understand the broader context and motivation behind the user's request.</how_to_use>
    <body_structure>Lead with the fact or decision, then a **Why:** line and a **How to apply:** line.</body_structure>
</type>
<type>
    <name>reference</name>
    <description>Stores pointers to where information can be found in external systems.</description>
    <when_to_save>When you learn about resources in external systems and their purpose.</when_to_save>
    <how_to_use>When the user references an external system or information that may be in an external system.</how_to_use>
</type>
</types>

## What NOT to save in memory

- Code patterns, conventions, architecture, file paths, or project structure — these can be derived by reading the current project state.
- Git history, recent changes, or who-changed-what — `git log` / `git blame` are authoritative.
- Debugging solutions or fix recipes — the fix is in the code; the commit message has the context.
- Anything already documented in CLAUDE.md files.
- Ephemeral task details: in-progress work, temporary state, current conversation context.

## How to save memories

Saving a memory is a two-step process:

**Step 1** — write the memory to its own file (e.g., `user_role.md`, `feedback_testing.md`) using this frontmatter format:

```markdown
---
name: {{{{memory name}}}}
description: {{{{one-line description}}}}
type: {{{{user, feedback, project, reference}}}}
---

{{{{memory content}}}}
```

**Step 2** — add a pointer to that file in `MEMORY.md`. `MEMORY.md` is an index, not a memory — each entry should be one line, under ~150 characters: `- [Title](file.md) — one-line hook`.

- `MEMORY.md` lines after 200 will be truncated, so keep the index concise
- Keep the name, description, and type fields in memory files up-to-date with the content
- Do not write duplicate memories. First check if there is an existing memory you can update.

## When to access memories

- When memories seem relevant, or the user references prior-conversation work.
- You MUST access memory when the user explicitly asks you to check, recall, or remember.
- If the user says to *ignore* or *not use* memory: do not apply remembered facts.
- Memory records can become stale. Verify against current state before acting on them.

## Before recommending from memory

A memory that names a specific function, file, or flag may have been renamed, removed, or never merged. Before recommending it:

- If the memory names a file path: check the file exists.
- If the memory names a function or flag: grep for it.
- If the user is about to act on your recommendation, verify first.

## Current MEMORY.md

{content}

Memory directory: `{dir}`
"#,
        dir = dir,
        scope_guidance = scope_guidance,
        content = content,
    )
}

/// Load agent memory and build the memory system prompt.
///
/// Returns `None` if `memory_scope` is `None` (AC-101: no persistent memory).
/// Returns the full prompt string for system prompt injection.
pub fn load_agent_memory_prompt(
    agent_type: &str,
    memory_scope: Option<&AgentMemoryScope>,
    cwd: &Path,
) -> Option<String> {
    let scope = memory_scope?;

    let memory_dir = get_agent_memory_dir(agent_type, scope, cwd);

    // Fire-and-forget dir creation
    let dir_clone = memory_dir.clone();
    tokio::spawn(async move { ensure_memory_dir_exists(&dir_clone).await });

    // Scope-specific guidance
    let scope_guidance = match scope {
        AgentMemoryScope::User => "Keep learnings general since they apply across all projects.",
        AgentMemoryScope::Project => "Tailor your memories to this specific project.",
        AgentMemoryScope::Local => "Tailor to this project and machine. Not checked into VCS.",
    };

    // Read MEMORY.md (sync — must be available for prompt building)
    let memory_md_path = memory_dir.join("MEMORY.md");
    let memory_content = std::fs::read_to_string(&memory_md_path).unwrap_or_default();

    // Truncate to 200 lines AND 25KB (whichever fires first)
    let truncated =
        truncate_entrypoint_content(&memory_content, MAX_ENTRYPOINT_LINES, MAX_ENTRYPOINT_BYTES);

    Some(build_full_memory_prompt(
        &memory_dir,
        scope_guidance,
        &truncated,
    ))
}
