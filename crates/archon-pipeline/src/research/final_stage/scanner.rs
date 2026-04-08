//! Output scanner — reads agent output files matching `{index}-{agent-key}.md`.

use anyhow::Result;
use std::path::Path;

/// Represents a single parsed agent output file.
#[derive(Debug, Clone)]
pub struct AgentOutput {
    /// The agent key extracted from the filename (e.g. "topic-researcher").
    pub agent_key: String,
    /// The numeric index/phase parsed from the filename prefix.
    pub phase: u8,
    /// The full text content read from the file.
    pub content: String,
    /// The original filename (e.g. "01-topic-researcher.md").
    pub file_path: String,
}

/// Scan a directory for agent output markdown files matching pattern `{NN}-{agent-key}.md`.
///
/// Files are returned sorted by filename. Non-`.md` files and files whose names
/// do not match the expected pattern are silently skipped.
pub fn scan_outputs(dir: &Path) -> Result<Vec<AgentOutput>> {
    let mut outputs = Vec::new();
    if !dir.exists() {
        return Ok(outputs);
    }

    let mut entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let filename = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s,
            None => continue,
        };
        // Parse pattern: {index}-{agent-key}
        if let Some(dash_pos) = filename.find('-') {
            let index_str = &filename[..dash_pos];
            let agent_key = &filename[dash_pos + 1..];
            if let Ok(index) = index_str.parse::<u8>() {
                let content = std::fs::read_to_string(&path)?;
                outputs.push(AgentOutput {
                    agent_key: agent_key.to_string(),
                    phase: index,
                    content,
                    file_path: path.file_name().unwrap().to_string_lossy().to_string(),
                });
            }
        }
    }

    Ok(outputs)
}
