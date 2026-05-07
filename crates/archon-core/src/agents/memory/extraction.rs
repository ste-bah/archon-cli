use std::path::Path;

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
                        && let Some(input) = block.get("input")
                    {
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
                    && name.ends_with(".md")
                {
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
