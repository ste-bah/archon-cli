//! Pull request creation via the `gh` CLI.

use std::process::Command;

/// Build the `gh pr create` command arguments.
///
/// Returns the full argument list for `Command::new("gh")`.
pub fn build_gh_command(title: &str, body: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "gh".to_string(),
        "pr".to_string(),
        "create".to_string(),
        "--title".to_string(),
        title.to_string(),
    ];
    if let Some(b) = body {
        args.push("--body".to_string());
        args.push(b.to_string());
    }
    args
}

/// Create a pull request using the `gh` CLI.
///
/// Returns the PR URL on success or an error message on failure.
/// Requires `gh` to be installed and authenticated.
pub fn create_pr(title: &str, body: Option<&str>) -> Result<String, String> {
    let args = build_gh_command(title, body);

    let output = Command::new(&args[0])
        .args(&args[1..])
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "gh CLI not found. Install it from https://cli.github.com/".to_string()
            } else {
                format!("Failed to run gh: {e}")
            }
        })?;

    if output.status.success() {
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(url)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(format!("gh pr create failed: {stderr}"))
    }
}
