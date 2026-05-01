//! Agent-scoped memory operations using CozoDB tag-based isolation.
//!
//! Agent memories are scoped via the tag convention `agent:{agent_type}`.
//! This avoids namespace collisions between different agents' memories.

use std::path::{Path, PathBuf};

use archon_memory::MemoryTrait;
use archon_memory::types::{MemoryError, MemoryType, SearchFilter};

use super::definition::AgentMemoryScope;

/// Build the scoping tag for an agent type.
pub fn agent_tag(agent_type: &str) -> String {
    format!("agent:{agent_type}")
}

/// Build the scope isolation tag from an `AgentMemoryScope`.
pub fn scope_tag(scope: &AgentMemoryScope) -> String {
    match scope {
        AgentMemoryScope::User => "scope:user".to_string(),
        AgentMemoryScope::Project => "scope:project".to_string(),
        AgentMemoryScope::Local => "scope:local".to_string(),
    }
}

/// Load agent-scoped memories from the memory store.
///
/// For each query in `recall_queries`, searches memories tagged with
/// `agent:{agent_type}` (and optionally a scope tag) and returns the
/// matching content strings.
///
/// If `memory_scope` is `None`, returns empty — AC-101: no persistent memory.
pub fn load_agent_memory(
    agent_type: &str,
    recall_queries: &[String],
    memory: &dyn MemoryTrait,
    memory_scope: Option<&AgentMemoryScope>,
) -> Vec<String> {
    // AC-101: None = no persistent memory
    let scope = match memory_scope {
        Some(s) => s,
        None => return Vec::new(),
    };

    let agent = agent_tag(agent_type);
    let scope = scope_tag(scope);
    let mut results = Vec::new();

    for query in recall_queries {
        let filter = SearchFilter {
            memory_type: None,
            tags: vec![agent.clone(), scope.clone()],
            require_all_tags: true,
            text: Some(query.clone()),
            date_from: None,
            date_to: None,
        };

        match memory.search_memories(&filter) {
            Ok(memories) => {
                for m in memories {
                    results.push(m.content.clone());
                }
            }
            Err(e) => {
                tracing::warn!(agent = agent_type, query = query.as_str(), error = %e,
                    "failed to recall agent memory");
            }
        }
    }

    results
}

/// Save a memory scoped to a specific agent.
///
/// Tags the memory with `agent:{agent_type}` and `scope:{scope}` plus any
/// additional user tags.
///
/// If `memory_scope` is `None`, this is a no-op (AC-101: no persistent memory).
pub fn save_agent_memory(
    agent_type: &str,
    content: &str,
    title: &str,
    extra_tags: &[String],
    memory: &dyn MemoryTrait,
    project_path: &str,
    memory_scope: Option<&AgentMemoryScope>,
) -> Result<String, MemoryError> {
    // AC-101: None = no persistent memory
    let scope = match memory_scope {
        Some(s) => s,
        None => return Ok("skipped-no-scope".to_string()),
    };

    let mut tags = vec![agent_tag(agent_type), scope_tag(scope)];
    tags.extend(extra_tags.iter().cloned());

    memory.store_memory(
        content,
        title,
        MemoryType::Fact,
        0.5,
        &tags,
        "agent",
        project_path,
    )
}

// ---------------------------------------------------------------------------
// File-based memory directory & prompt system (AGT-027)
// ---------------------------------------------------------------------------

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
        && e.kind() != std::io::ErrorKind::AlreadyExists {
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

// ---------------------------------------------------------------------------
// Background memory extraction (AGT-027)
// ---------------------------------------------------------------------------

/// State held across turns for extraction throttling and cursor tracking.
#[derive(Debug)]
pub struct ExtractionState {
    /// UUID of last message processed by extraction (cursor).
    pub last_memory_message_uuid: Option<String>,
    /// Counter for throttling: only extract every N turns.
    pub turns_since_last_extraction: u32,
    /// Extraction interval (default 1 = every eligible turn).
    pub extraction_interval: u32,
}

impl Default for ExtractionState {
    fn default() -> Self {
        Self {
            last_memory_message_uuid: None,
            turns_since_last_extraction: 0,
            extraction_interval: 1,
        }
    }
}

impl ExtractionState {
    /// Create with custom extraction interval.
    pub fn with_interval(interval: u32) -> Self {
        Self {
            extraction_interval: interval,
            ..Default::default()
        }
    }
}

/// Scan tool_use blocks for Write/Edit calls targeting memory directory.
///
/// Scans messages since `since_uuid` cursor. If `since_uuid` is None, scans all.
pub fn has_memory_writes_since(
    messages: &[serde_json::Value],
    since_uuid: Option<&str>,
    memory_dir: &Path,
) -> bool {
    let mut found_start = since_uuid.is_none();
    let mem_path = memory_dir.to_string_lossy();

    for msg in messages {
        if !found_start {
            if msg.get("uuid").and_then(|v| v.as_str()) == since_uuid {
                found_start = true;
            }
            continue;
        }
        // Scan tool_use blocks in assistant messages
        if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
            for block in content {
                if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                    let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    if (name == "Write" || name == "Edit")
                        && let Some(input) = block.get("input") {
                            let file_path = input
                                .get("file_path")
                                .and_then(|p| p.as_str())
                                .unwrap_or("");
                            if file_path.starts_with(&*mem_path) {
                                return true;
                            }
                        }
                }
            }
        }
    }
    false
}

/// Scan memory directory for existing .md files and return a manifest string.
pub async fn scan_memory_files(dir: &Path) -> String {
    match tokio::fs::read_dir(dir).await {
        Ok(mut entries) => {
            let mut files = Vec::new();
            while let Ok(Some(entry)) = entries.next_entry().await {
                if let Some(name) = entry.file_name().to_str()
                    && name.ends_with(".md") {
                        files.push(name.to_string());
                    }
            }
            if files.is_empty() {
                "(no files)".to_string()
            } else {
                files.join(", ")
            }
        }
        Err(_) => "(no files)".to_string(),
    }
}

/// Check extraction gates and determine whether to run extraction this turn.
///
/// Returns `true` if extraction should proceed, `false` if gated out.
/// Advances throttling counter as a side effect.
pub fn should_extract(
    is_subagent: bool,
    state: &mut ExtractionState,
    messages: &[serde_json::Value],
    memory_dir: &Path,
) -> bool {
    // Gate 1: Main session only — subagents do NOT get extraction
    if is_subagent {
        return false;
    }

    // Gate 2: Throttling — only extract every N eligible turns
    state.turns_since_last_extraction += 1;
    if state.turns_since_last_extraction < state.extraction_interval {
        return false;
    }
    state.turns_since_last_extraction = 0;

    // Gate 3: Mutual exclusion via SCAN — check if agent wrote memory directly
    if has_memory_writes_since(
        messages,
        state.last_memory_message_uuid.as_deref(),
        memory_dir,
    ) {
        tracing::debug!("Skipping extraction — agent wrote to memory directly");
        // Advance cursor past this range
        if let Some(last) = messages.last() {
            state.last_memory_message_uuid =
                last.get("uuid").and_then(|v| v.as_str()).map(String::from);
        }
        return false;
    }

    // Gate 4: No messages to extract from
    if messages.is_empty() {
        return false;
    }

    true
}

// ---------------------------------------------------------------------------
// Original functions
// ---------------------------------------------------------------------------

/// Increment the invocation_count in an agent's meta.json.
///
/// Returns the new count, or an error message.
pub fn increment_invocation_count(agent_dir: &std::path::Path) -> Result<u64, String> {
    let meta_path = agent_dir.join("meta.json");
    let content = std::fs::read_to_string(&meta_path)
        .map_err(|e| format!("failed to read meta.json: {e}"))?;

    let mut meta: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("failed to parse meta.json: {e}"))?;

    let count = meta
        .get("invocation_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        + 1;

    meta["invocation_count"] = serde_json::json!(count);
    meta["updated_at"] = serde_json::json!(chrono::Utc::now().to_rfc3339());

    let serialized = serde_json::to_string_pretty(&meta)
        .map_err(|e| format!("failed to serialize meta.json: {e}"))?;

    std::fs::write(&meta_path, serialized)
        .map_err(|e| format!("failed to write meta.json: {e}"))?;

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_memory::types::{Memory, MemoryType, RelType};
    use std::sync::Mutex;

    /// Null memory implementation for testing. Returns empty results.
    struct NullMemory;

    impl MemoryTrait for NullMemory {
        fn store_memory(
            &self,
            _content: &str,
            _title: &str,
            _memory_type: MemoryType,
            _importance: f64,
            _tags: &[String],
            _source_type: &str,
            _project_path: &str,
        ) -> Result<String, MemoryError> {
            Ok("null-id".to_string())
        }
        fn get_memory(&self, _id: &str) -> Result<Memory, MemoryError> {
            Err(MemoryError::NotFound("null".into()))
        }
        fn update_memory(
            &self,
            _id: &str,
            _content: Option<&str>,
            _tags: Option<&[String]>,
        ) -> Result<(), MemoryError> {
            Ok(())
        }
        fn update_importance(&self, _id: &str, _importance: f64) -> Result<(), MemoryError> {
            Ok(())
        }
        fn delete_memory(&self, _id: &str) -> Result<(), MemoryError> {
            Ok(())
        }
        fn create_relationship(
            &self,
            _from: &str,
            _to: &str,
            _rel: RelType,
            _ctx: Option<&str>,
            _str: f64,
        ) -> Result<(), MemoryError> {
            Ok(())
        }
        fn recall_memories(&self, _query: &str, _limit: usize) -> Result<Vec<Memory>, MemoryError> {
            Ok(vec![])
        }
        fn search_memories(&self, _filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
            Ok(vec![])
        }
        fn list_recent(&self, _limit: usize) -> Result<Vec<Memory>, MemoryError> {
            Ok(vec![])
        }
        fn memory_count(&self) -> Result<usize, MemoryError> {
            Ok(0)
        }
        fn clear_all(&self) -> Result<usize, MemoryError> {
            Ok(0)
        }
        fn get_related_memories(&self, _id: &str, _depth: u32) -> Result<Vec<Memory>, MemoryError> {
            Ok(vec![])
        }
    }

    /// Mock memory that records store calls for verification.
    struct MockMemory {
        stored: Mutex<Vec<(String, Vec<String>)>>, // (content, tags)
    }

    impl MockMemory {
        fn new() -> Self {
            Self {
                stored: Mutex::new(vec![]),
            }
        }
    }

    impl MemoryTrait for MockMemory {
        fn store_memory(
            &self,
            content: &str,
            _title: &str,
            _memory_type: MemoryType,
            _importance: f64,
            tags: &[String],
            _source_type: &str,
            _project_path: &str,
        ) -> Result<String, MemoryError> {
            self.stored
                .lock()
                .unwrap()
                .push((content.to_string(), tags.to_vec()));
            Ok("mock-id".to_string())
        }
        fn get_memory(&self, _id: &str) -> Result<Memory, MemoryError> {
            Err(MemoryError::NotFound("mock".into()))
        }
        fn update_memory(
            &self,
            _id: &str,
            _content: Option<&str>,
            _tags: Option<&[String]>,
        ) -> Result<(), MemoryError> {
            Ok(())
        }
        fn update_importance(&self, _id: &str, _importance: f64) -> Result<(), MemoryError> {
            Ok(())
        }
        fn delete_memory(&self, _id: &str) -> Result<(), MemoryError> {
            Ok(())
        }
        fn create_relationship(
            &self,
            _from: &str,
            _to: &str,
            _rel: RelType,
            _ctx: Option<&str>,
            _str: f64,
        ) -> Result<(), MemoryError> {
            Ok(())
        }
        fn recall_memories(&self, _query: &str, _limit: usize) -> Result<Vec<Memory>, MemoryError> {
            Ok(vec![])
        }
        fn search_memories(&self, _filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
            Ok(vec![])
        }
        fn list_recent(&self, _limit: usize) -> Result<Vec<Memory>, MemoryError> {
            Ok(vec![])
        }
        fn memory_count(&self) -> Result<usize, MemoryError> {
            Ok(0)
        }
        fn clear_all(&self) -> Result<usize, MemoryError> {
            Ok(0)
        }
        fn get_related_memories(&self, _id: &str, _depth: u32) -> Result<Vec<Memory>, MemoryError> {
            Ok(vec![])
        }
    }

    #[test]
    fn agent_tag_format() {
        assert_eq!(agent_tag("code-reviewer"), "agent:code-reviewer");
        assert_eq!(agent_tag("explore"), "agent:explore");
    }

    #[test]
    fn load_agent_memory_empty_queries_returns_empty() {
        let memory = NullMemory;
        let results = load_agent_memory("test-agent", &[], &memory, Some(&AgentMemoryScope::User));
        assert!(results.is_empty());
    }

    #[test]
    fn load_agent_memory_null_memory_returns_empty() {
        let memory = NullMemory;
        let queries = vec!["test query".to_string()];
        let results = load_agent_memory(
            "test-agent",
            &queries,
            &memory,
            Some(&AgentMemoryScope::User),
        );
        assert!(results.is_empty());
    }

    #[test]
    fn load_agent_memory_none_scope_returns_empty() {
        let memory = NullMemory;
        let queries = vec!["test query".to_string()];
        let results = load_agent_memory("test-agent", &queries, &memory, None);
        assert!(
            results.is_empty(),
            "AC-101: None scope = no persistent memory"
        );
    }

    #[test]
    fn save_agent_memory_includes_agent_and_scope_tags() {
        let memory = MockMemory::new();
        let result = save_agent_memory(
            "code-reviewer",
            "test content",
            "test title",
            &["extra-tag".to_string()],
            &memory,
            "/tmp",
            Some(&AgentMemoryScope::Project),
        );
        assert!(result.is_ok());

        let stored = memory.stored.lock().unwrap();
        assert_eq!(stored.len(), 1);
        assert!(stored[0].1.contains(&"agent:code-reviewer".to_string()));
        assert!(stored[0].1.contains(&"scope:project".to_string()));
        assert!(stored[0].1.contains(&"extra-tag".to_string()));
    }

    #[test]
    fn save_agent_memory_none_scope_is_noop() {
        let memory = MockMemory::new();
        let result = save_agent_memory(
            "code-reviewer",
            "test content",
            "test title",
            &[],
            &memory,
            "/tmp",
            None,
        );
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "skipped-no-scope");

        let stored = memory.stored.lock().unwrap();
        assert!(stored.is_empty(), "AC-101: None scope = no memory stored");
    }

    #[test]
    fn scope_tag_formats() {
        assert_eq!(scope_tag(&AgentMemoryScope::User), "scope:user");
        assert_eq!(scope_tag(&AgentMemoryScope::Project), "scope:project");
        assert_eq!(scope_tag(&AgentMemoryScope::Local), "scope:local");
    }

    #[test]
    fn increment_invocation_count_on_valid_meta() {
        let dir = tempfile::tempdir().unwrap();
        let meta_path = dir.path().join("meta.json");
        std::fs::write(
            &meta_path,
            r#"{"version":"1.0","invocation_count":5,"updated_at":"2026-01-01T00:00:00Z"}"#,
        )
        .unwrap();

        let count = increment_invocation_count(dir.path()).unwrap();
        assert_eq!(count, 6);

        // Verify the file was updated
        let content = std::fs::read_to_string(&meta_path).unwrap();
        let meta: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(meta["invocation_count"], 6);
    }

    #[test]
    fn increment_invocation_count_missing_file_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let result = increment_invocation_count(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn increment_invocation_count_missing_field_starts_at_one() {
        let dir = tempfile::tempdir().unwrap();
        let meta_path = dir.path().join("meta.json");
        std::fs::write(&meta_path, r#"{"version":"1.0"}"#).unwrap();

        let count = increment_invocation_count(dir.path()).unwrap();
        assert_eq!(count, 1);
    }

    // -----------------------------------------------------------------------
    // AGT-027: File-based memory directory & prompt tests
    // -----------------------------------------------------------------------

    #[test]
    fn get_agent_memory_dir_user_scope() {
        let cwd = Path::new("/tmp/project");
        let dir = get_agent_memory_dir("code-reviewer", &AgentMemoryScope::User, cwd);
        let home = dirs::home_dir().unwrap();
        assert_eq!(dir, home.join(".archon/agent-memory/code-reviewer"));
    }

    #[test]
    fn get_agent_memory_dir_project_scope() {
        let cwd = Path::new("/tmp/project");
        let dir = get_agent_memory_dir("code-reviewer", &AgentMemoryScope::Project, cwd);
        assert_eq!(
            dir,
            PathBuf::from("/tmp/project/.archon/agent-memory/code-reviewer")
        );
    }

    #[test]
    fn get_agent_memory_dir_local_scope() {
        let cwd = Path::new("/tmp/project");
        let dir = get_agent_memory_dir("code-reviewer", &AgentMemoryScope::Local, cwd);
        assert_eq!(
            dir,
            PathBuf::from("/tmp/project/.archon/agent-memory-local/code-reviewer")
        );
    }

    #[test]
    fn get_agent_memory_dir_sanitizes_colons() {
        let cwd = Path::new("/tmp/project");
        let dir = get_agent_memory_dir("plugin:my-agent", &AgentMemoryScope::Project, cwd);
        assert_eq!(
            dir,
            PathBuf::from("/tmp/project/.archon/agent-memory/plugin-my-agent")
        );
    }

    #[tokio::test]
    async fn ensure_memory_dir_creates_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("nested/agent-memory/test");
        assert!(!dir.exists());
        ensure_memory_dir_exists(&dir).await;
        assert!(dir.exists());
    }

    #[tokio::test]
    async fn ensure_memory_dir_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("agent-memory/test");
        ensure_memory_dir_exists(&dir).await;
        ensure_memory_dir_exists(&dir).await; // second call should not error
        assert!(dir.exists());
    }

    // --- truncate_entrypoint_content tests ---

    #[test]
    fn truncate_empty_returns_empty() {
        assert_eq!(truncate_entrypoint_content("", 200, 25_000), "");
        assert_eq!(truncate_entrypoint_content("   \n  ", 200, 25_000), "");
    }

    #[test]
    fn truncate_no_truncation_needed() {
        let content = "- [Agent role](role.md) — Senior developer\n- [Prefs](prefs.md) — Uses vim";
        let result = truncate_entrypoint_content(content, 200, 25_000);
        assert_eq!(result, content);
        assert!(!result.contains("WARNING"));
    }

    #[test]
    fn truncate_line_limit_fires() {
        let content = (0..10)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let result = truncate_entrypoint_content(&content, 5, 25_000);
        assert!(result.contains("WARNING"));
        assert!(result.contains("10 lines"));
        // Should have exactly 5 content lines before the warning
        let before_warning = result.split("\n\n> WARNING").next().unwrap();
        assert_eq!(before_warning.lines().count(), 5);
    }

    #[test]
    fn truncate_byte_limit_fires() {
        // Create content that's under line limit but over byte limit
        let line = "x".repeat(100);
        let content = (0..5).map(|_| line.clone()).collect::<Vec<_>>().join("\n");
        let result = truncate_entrypoint_content(&content, 200, 200);
        assert!(result.contains("WARNING"));
        assert!(result.contains("bytes"));
    }

    #[test]
    fn truncate_both_limits_fire() {
        let content = (0..300)
            .map(|i| format!("line {i} with some extra padding text"))
            .collect::<Vec<_>>()
            .join("\n");
        let result = truncate_entrypoint_content(&content, 200, 500);
        assert!(result.contains("WARNING"));
        assert!(result.contains("lines and"));
        assert!(result.contains("bytes"));
    }

    // --- build_full_memory_prompt tests ---

    #[test]
    fn prompt_contains_all_required_sections() {
        let dir = Path::new("/tmp/.archon/agent-memory/test-agent");
        let prompt = build_full_memory_prompt(dir, "Project-scoped memories.", "- [Role](role.md)");

        // Header
        assert!(prompt.contains("# auto memory"), "must have header");
        // Dir path
        assert!(
            prompt.contains("/tmp/.archon/agent-memory/test-agent"),
            "must have dir path"
        );
        // Scope guidance
        assert!(
            prompt.contains("Project-scoped memories."),
            "must have scope guidance"
        );
        // 4 memory types
        assert!(prompt.contains("<name>user</name>"), "must have user type");
        assert!(
            prompt.contains("<name>feedback</name>"),
            "must have feedback type"
        );
        assert!(
            prompt.contains("<name>project</name>"),
            "must have project type"
        );
        assert!(
            prompt.contains("<name>reference</name>"),
            "must have reference type"
        );
        // What not to save
        assert!(prompt.contains("What NOT to save"), "must have exclusions");
        // How to save (2-step)
        assert!(prompt.contains("Step 1"), "must have step 1");
        assert!(prompt.contains("Step 2"), "must have step 2");
        assert!(prompt.contains("MEMORY.md"), "must reference MEMORY.md");
        // Frontmatter format
        assert!(prompt.contains("name:"), "must show frontmatter");
        assert!(prompt.contains("description:"), "must show frontmatter");
        assert!(prompt.contains("type:"), "must show frontmatter");
        // When to access
        assert!(prompt.contains("When to access"), "must have access rules");
        // Before recommending
        assert!(
            prompt.contains("Before recommending"),
            "must have verification rules"
        );
        // Existing content
        assert!(
            prompt.contains("- [Role](role.md)"),
            "must include existing content"
        );
    }

    #[test]
    fn prompt_empty_memory_shows_placeholder() {
        let dir = Path::new("/tmp/test");
        let prompt = build_full_memory_prompt(dir, "test", "");
        assert!(prompt.contains("(No memories yet)"));
    }

    // --- load_agent_memory_prompt tests ---

    #[test]
    fn load_prompt_none_scope_returns_none() {
        let result = load_agent_memory_prompt("test-agent", None, Path::new("/tmp"));
        assert!(result.is_none(), "AC-101: None scope = no memory prompt");
    }

    #[tokio::test]
    async fn load_prompt_with_scope_returns_some() {
        let tmp = tempfile::tempdir().unwrap();
        let result =
            load_agent_memory_prompt("test-agent", Some(&AgentMemoryScope::Project), tmp.path());
        assert!(result.is_some(), "should return prompt for valid scope");
        let prompt = result.unwrap();
        assert!(prompt.contains("# auto memory"));
        assert!(prompt.contains("test-agent"));
    }

    #[tokio::test]
    async fn load_prompt_reads_existing_memory_md() {
        let tmp = tempfile::tempdir().unwrap();
        let memory_dir = tmp.path().join(".archon/agent-memory/test-agent");
        std::fs::create_dir_all(&memory_dir).unwrap();
        std::fs::write(
            memory_dir.join("MEMORY.md"),
            "- [Pref](pref.md) — prefers short responses",
        )
        .unwrap();

        let prompt =
            load_agent_memory_prompt("test-agent", Some(&AgentMemoryScope::Project), tmp.path())
                .unwrap();
        assert!(
            prompt.contains("prefers short responses"),
            "must include existing MEMORY.md content"
        );
    }

    // --- has_memory_writes_since tests ---

    #[test]
    fn has_memory_writes_no_messages() {
        let dir = Path::new("/tmp/.archon/agent-memory/test");
        assert!(!has_memory_writes_since(&[], None, dir));
    }

    #[test]
    fn has_memory_writes_detects_write_to_memory_dir() {
        let dir = Path::new("/tmp/.archon/agent-memory/test");
        let messages = vec![serde_json::json!({
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "name": "Write",
                "input": {
                    "file_path": "/tmp/.archon/agent-memory/test/role.md",
                    "content": "some memory"
                }
            }]
        })];
        assert!(has_memory_writes_since(&messages, None, dir));
    }

    #[test]
    fn has_memory_writes_detects_edit_to_memory_dir() {
        let dir = Path::new("/tmp/.archon/agent-memory/test");
        let messages = vec![serde_json::json!({
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "name": "Edit",
                "input": {
                    "file_path": "/tmp/.archon/agent-memory/test/MEMORY.md",
                    "old_string": "old",
                    "new_string": "new"
                }
            }]
        })];
        assert!(has_memory_writes_since(&messages, None, dir));
    }

    #[test]
    fn has_memory_writes_ignores_write_outside_memory_dir() {
        let dir = Path::new("/tmp/.archon/agent-memory/test");
        let messages = vec![serde_json::json!({
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "name": "Write",
                "input": {
                    "file_path": "/tmp/src/main.rs",
                    "content": "code"
                }
            }]
        })];
        assert!(!has_memory_writes_since(&messages, None, dir));
    }

    #[test]
    fn has_memory_writes_respects_cursor() {
        let dir = Path::new("/tmp/.archon/agent-memory/test");
        let messages = vec![
            serde_json::json!({
                "uuid": "msg-001",
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "name": "Write",
                    "input": { "file_path": "/tmp/.archon/agent-memory/test/old.md", "content": "x" }
                }]
            }),
            serde_json::json!({
                "uuid": "msg-002",
                "role": "user",
                "content": "next turn"
            }),
        ];
        // Write is BEFORE cursor msg-001, so should not be detected after it
        assert!(!has_memory_writes_since(&messages, Some("msg-001"), dir));
    }

    #[test]
    fn has_memory_writes_detects_after_cursor() {
        let dir = Path::new("/tmp/.archon/agent-memory/test");
        let messages = vec![
            serde_json::json!({
                "uuid": "msg-001",
                "role": "user",
                "content": "first"
            }),
            serde_json::json!({
                "uuid": "msg-002",
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "name": "Write",
                    "input": { "file_path": "/tmp/.archon/agent-memory/test/new.md", "content": "x" }
                }]
            }),
        ];
        // Write is AFTER cursor msg-001, so should be detected
        assert!(has_memory_writes_since(&messages, Some("msg-001"), dir));
    }

    // --- should_extract tests ---

    #[test]
    fn should_extract_false_for_subagent() {
        let dir = Path::new("/tmp/test");
        let mut state = ExtractionState::default();
        let msgs = vec![serde_json::json!({"role": "user", "content": "hi"})];
        assert!(!should_extract(true, &mut state, &msgs, dir));
    }

    #[test]
    fn should_extract_true_for_main_session() {
        let dir = Path::new("/tmp/test");
        let mut state = ExtractionState::default();
        let msgs = vec![serde_json::json!({"role": "user", "content": "hi"})];
        assert!(should_extract(false, &mut state, &msgs, dir));
    }

    #[test]
    fn should_extract_throttles_by_interval() {
        let dir = Path::new("/tmp/test");
        let mut state = ExtractionState::with_interval(3);
        let msgs = vec![serde_json::json!({"role": "user", "content": "hi"})];

        // Turns 1, 2: should NOT extract (interval = 3)
        assert!(!should_extract(false, &mut state, &msgs, dir));
        assert!(!should_extract(false, &mut state, &msgs, dir));
        // Turn 3: SHOULD extract
        assert!(should_extract(false, &mut state, &msgs, dir));
        // Turn 4: starts over — should NOT extract
        assert!(!should_extract(false, &mut state, &msgs, dir));
    }

    #[test]
    fn should_extract_false_when_agent_wrote_memory() {
        let dir = Path::new("/tmp/.archon/agent-memory/test");
        let mut state = ExtractionState::default();
        let msgs = vec![serde_json::json!({
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "name": "Write",
                "input": { "file_path": "/tmp/.archon/agent-memory/test/role.md", "content": "x" }
            }]
        })];
        assert!(!should_extract(false, &mut state, &msgs, dir));
    }

    #[test]
    fn should_extract_false_for_empty_messages() {
        let dir = Path::new("/tmp/test");
        let mut state = ExtractionState::default();
        assert!(!should_extract(false, &mut state, &[], dir));
    }

    #[test]
    fn extraction_state_default_interval_is_one() {
        let state = ExtractionState::default();
        assert_eq!(state.extraction_interval, 1);
        assert_eq!(state.turns_since_last_extraction, 0);
        assert!(state.last_memory_message_uuid.is_none());
    }

    // --- scan_memory_files tests ---

    #[tokio::test]
    async fn scan_memory_files_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let result = scan_memory_files(tmp.path()).await;
        assert_eq!(result, "(no files)");
    }

    #[tokio::test]
    async fn scan_memory_files_lists_md_only() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("role.md"), "memory").unwrap();
        std::fs::write(tmp.path().join("notes.md"), "memory").unwrap();
        std::fs::write(tmp.path().join("data.json"), "{}").unwrap();
        let result = scan_memory_files(tmp.path()).await;
        // Should contain both .md files but not .json
        assert!(result.contains("role.md"));
        assert!(result.contains("notes.md"));
        assert!(!result.contains("data.json"));
    }

    #[tokio::test]
    async fn scan_memory_files_nonexistent_dir() {
        let result = scan_memory_files(Path::new("/nonexistent/dir")).await;
        assert_eq!(result, "(no files)");
    }
}
