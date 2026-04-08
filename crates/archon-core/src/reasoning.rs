use std::fs;
use std::path::Path;

/// Load ARCHON.md from the given directory, or empty string if not found.
/// Falls back to CLAUDE.md for backward compatibility.
pub fn load_archon_md(project_dir: &Path) -> String {
    let new_path = project_dir.join("ARCHON.md");
    if new_path.is_file() {
        return fs::read_to_string(&new_path).unwrap_or_default();
    }
    let old_path = project_dir.join("CLAUDE.md");
    if old_path.is_file() {
        tracing::warn!(
            "Loading from deprecated path {}. Rename to {} to suppress this warning.",
            old_path.display(),
            new_path.display()
        );
        return fs::read_to_string(&old_path).unwrap_or_default();
    }
    String::new()
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
    fn load_archon_md_missing_returns_empty() {
        let content = load_archon_md(Path::new("/nonexistent/path"));
        assert!(content.is_empty());
    }
}
