//! Load user-defined output styles from `~/.claude/output-styles/`.
//!
//! Each `.md` file in the directory is parsed as one `OutputStyleConfig`:
//!
//! ```text
//! # Style Name
//! Description: Human-readable description
//! The rest of the file is the prompt injected into the system prompt.
//! ```
//!
//! Files that do not start with `# ` are skipped with a warning.
//! Non-`.md` files are silently ignored.

use std::path::Path;

use crate::output_style::{OutputStyleConfig, OutputStyleSource};

/// Load all `.md` files from `dir` as user-defined output styles.
///
/// Returns an empty `Vec` when `dir` does not exist or cannot be read —
/// never panics.
pub fn load_styles_from_dir(dir: &Path) -> Vec<OutputStyleConfig> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut styles = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        match parse_style_file(&path) {
            Some(style) => styles.push(style),
            None => {
                tracing::warn!(
                    path = %path.display(),
                    "skipping output style file: failed to parse"
                );
            }
        }
    }

    styles
}

/// Parse one `.md` file into an `OutputStyleConfig`.
///
/// Returns `None` if the file cannot be read or the first line does not
/// start with `# `.
fn parse_style_file(path: &Path) -> Option<OutputStyleConfig> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut lines = content.lines();

    // First line: `# Style Name`
    let first = lines.next().unwrap_or("").trim();
    if !first.starts_with("# ") {
        tracing::warn!(
            path = %path.display(),
            "output style file must start with '# Name'"
        );
        return None;
    }
    let name = first.trim_start_matches("# ").trim().to_owned();
    if name.is_empty() {
        return None;
    }

    // Second line: optional `Description: ...`
    let second = lines.next().unwrap_or("").trim();
    let description = if second.starts_with("Description:") {
        second.trim_start_matches("Description:").trim().to_owned()
    } else {
        String::new()
    };

    // Remaining lines form the prompt body.
    // If the second line was NOT a Description: line, include it in the body.
    let body = if second.starts_with("Description:") {
        lines.collect::<Vec<_>>().join("\n")
    } else {
        // second line is part of the body
        let rest: Vec<&str> = std::iter::once(second).chain(lines).collect();
        rest.join("\n")
    };

    let prompt = {
        let trimmed = body.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    };

    Some(OutputStyleConfig {
        name,
        description,
        prompt,
        source: OutputStyleSource::Config,
        keep_coding_instructions: None,
        force_for_plugin: None,
    })
}
