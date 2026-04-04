use std::path::Path;
use std::process::Command;

/// Git repository info extracted from the working directory.
#[derive(Debug, Clone)]
pub struct GitInfo {
    pub repo_name: String,
    pub branch: String,
    pub is_dirty: bool,
}

/// Detect git repository info from a working directory.
/// Returns None if not in a git repo or if git is not available.
pub fn detect_git_info(dir: &Path) -> Option<GitInfo> {
    // Check if in a git repo
    let status = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(dir)
        .output()
        .ok()?;

    if !status.status.success() {
        return None;
    }

    let branch = detect_branch(dir).unwrap_or_else(|| "unknown".into());
    let repo_name = detect_repo_name(dir).unwrap_or_else(|| "unknown".into());
    let is_dirty = detect_dirty(dir);

    Some(GitInfo {
        repo_name,
        branch,
        is_dirty,
    })
}

fn detect_branch(dir: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(dir)
        .output()
        .ok()?;

    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if branch.is_empty() {
        // Detached HEAD -- get short hash
        let hash_output = Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(dir)
            .output()
            .ok()?;
        Some(String::from_utf8_lossy(&hash_output.stdout).trim().to_string())
    } else {
        Some(branch)
    }
}

fn detect_repo_name(dir: &Path) -> Option<String> {
    // Try remote origin URL
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(dir)
        .output()
        .ok()?;

    if output.status.success() {
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        // Extract repo name from URL: github.com/user/repo.git -> repo
        if let Some(name) = url.rsplit('/').next() {
            return Some(name.strip_suffix(".git").unwrap_or(name).to_string());
        }
    }

    // Fall back to directory name
    let toplevel = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir)
        .output()
        .ok()?;

    let path = String::from_utf8_lossy(&toplevel.stdout).trim().to_string();
    Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
}

fn detect_dirty(dir: &Path) -> bool {
    Command::new("git")
        .args(["diff", "--quiet", "HEAD"])
        .current_dir(dir)
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(false)
}

/// Format git info for system prompt injection.
pub fn format_git_context(info: &GitInfo) -> String {
    let dirty = if info.is_dirty { " (dirty)" } else { "" };
    format!("- Git: repo={}, branch={}{dirty}", info.repo_name, info.branch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_git_in_repo() {
        // This test runs inside the archon repo, so git should be detected
        let dir = std::env::current_dir().expect("cwd");
        // May or may not be in a git repo depending on test runner
        let _ = detect_git_info(&dir); // Just verify no panic
    }

    #[test]
    fn detect_git_not_a_repo() {
        let info = detect_git_info(Path::new("/tmp"));
        // /tmp is not a git repo
        assert!(info.is_none());
    }

    #[test]
    fn format_git_context_clean() {
        let info = GitInfo {
            repo_name: "archon".into(),
            branch: "main".into(),
            is_dirty: false,
        };
        let formatted = format_git_context(&info);
        assert!(formatted.contains("archon"));
        assert!(formatted.contains("main"));
        assert!(!formatted.contains("dirty"));
    }

    #[test]
    fn format_git_context_dirty() {
        let info = GitInfo {
            repo_name: "archon".into(),
            branch: "dev".into(),
            is_dirty: true,
        };
        let formatted = format_git_context(&info);
        assert!(formatted.contains("dirty"));
    }
}
