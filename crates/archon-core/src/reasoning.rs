use std::fs;
use std::path::Path;

/// Load CLAUDE.md from the given directory, or empty string if not found.
pub fn load_claude_md(project_dir: &Path) -> String {
    let path = project_dir.join("CLAUDE.md");
    match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(_) => String::new(),
    }
}

/// Build the environment section for the system prompt.
pub fn build_environment_section(working_dir: &Path, git_branch: Option<&str>) -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "unknown".into());

    let mut section = format!(
        "# Environment\n\
         - Platform: {os} ({arch})\n\
         - Shell: {shell}\n\
         - Working directory: {}\n",
        working_dir.display()
    );

    if let Some(branch) = git_branch {
        section.push_str(&format!("- Git branch: {branch}\n"));
    }

    section
}

/// Detect the current git branch from the working directory.
pub fn detect_git_branch(dir: &Path) -> Option<String> {
    let head_path = dir.join(".git/HEAD");
    let content = fs::read_to_string(head_path).ok()?;
    let trimmed = content.trim();

    if let Some(branch) = trimmed.strip_prefix("ref: refs/heads/") {
        Some(branch.to_string())
    } else {
        // Detached HEAD -- return short hash
        Some(trimmed[..8.min(trimmed.len())].to_string())
    }
}

/// Build the complete system prompt blocks.
pub fn build_system_prompt(
    identity_blocks: Vec<serde_json::Value>,
    claude_md: &str,
    environment: &str,
) -> Vec<serde_json::Value> {
    let mut blocks = identity_blocks;

    if !claude_md.is_empty() {
        blocks.push(serde_json::json!({
            "type": "text",
            "text": claude_md,
        }));
    }

    if !environment.is_empty() {
        blocks.push(serde_json::json!({
            "type": "text",
            "text": environment,
        }));
    }

    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_environment_section_includes_os() {
        let section = build_environment_section(Path::new("/tmp"), Some("main"));
        assert!(section.contains("Platform:"));
        assert!(section.contains("/tmp"));
        assert!(section.contains("main"));
    }

    #[test]
    fn load_claude_md_missing_returns_empty() {
        let content = load_claude_md(Path::new("/nonexistent/path"));
        assert!(content.is_empty());
    }

    #[test]
    fn build_system_prompt_assembles_blocks() {
        let identity = vec![serde_json::json!({"type": "text", "text": "identity"})];
        let blocks = build_system_prompt(identity, "claude md content", "env info");
        assert_eq!(blocks.len(), 3);
    }

    #[test]
    fn build_system_prompt_skips_empty() {
        let blocks = build_system_prompt(Vec::new(), "", "");
        assert!(blocks.is_empty());
    }
}
