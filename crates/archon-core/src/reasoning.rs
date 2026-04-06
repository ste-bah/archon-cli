use std::fs;
use std::path::Path;

/// Load CLAUDE.md from the given directory, or empty string if not found.
pub fn load_claude_md(project_dir: &Path) -> String {
    let path = project_dir.join("CLAUDE.md");
    fs::read_to_string(&path).unwrap_or_default()
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
}
