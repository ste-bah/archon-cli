//! TASK-P0-B.3 Plan-file I/O helpers.
//!
//! `.archon/plan.md` is the per-session plan file. When Plan Mode is
//! active, tool calls that fail `is_tool_allowed_in_mode(Plan)` are
//! intercepted and appended to this file for user review. The `/plan`
//! slash command reads + displays it; `/plan open` spawns `$EDITOR`.
//!
//! # Path resolution
//!
//! Prefer project-root `.archon/plan.md`. If the working directory has
//! no `.archon/` (test / fresh repo), fall back to `$HOME/.archon/plan.md`.
//!
//! # Crate placement (bin/library split)
//!
//! This module lives in `archon-core` (library) rather than the bin
//! crate so both the dispatch path
//! (`crates/archon-core/src/dispatch.rs`) AND the bin-crate
//! `PlanHandler` (`src/command/plan.rs`) can import it without a
//! cyclic dependency. A thin shim at `src/command/plan_file.rs`
//! re-exports the public surface so `crate::command::plan_file::*`
//! continues to resolve from handler code (see Resolution (ii) in the
//! P0-B.3 spec).

use std::io::Write;
use std::path::PathBuf;

/// Resolve the plan file path. Prefers `<working_dir>/.archon/plan.md`;
/// if `.archon/` does not exist in the working dir, uses
/// `$HOME/.archon/plan.md` as a fallback.
pub fn plan_path(working_dir: &std::path::Path) -> PathBuf {
    let project = working_dir.join(".archon").join("plan.md");
    if project.parent().is_some_and(|p| p.exists()) {
        project
    } else if let Some(home) = dirs::home_dir() {
        home.join(".archon").join("plan.md")
    } else {
        // Last-resort: return the project path; caller handles missing-dir.
        project
    }
}

/// Read the plan file contents. Returns `Ok(None)` if the file does not
/// exist yet, `Ok(Some(content))` on success, `Err` on IO failure.
pub fn read_plan_file(path: &std::path::Path) -> std::io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Append a structured entry about an intercepted tool call to the plan
/// file. Creates parent dirs and the file if absent. Entry format:
///
/// ```text
/// ## <ISO timestamp> — <tool_name> (intercepted in Plan Mode)
///
/// ```json
/// <input JSON pretty-printed>
/// ```
/// ```
pub fn append_plan_entry(
    path: &std::path::Path,
    tool_name: &str,
    input: &serde_json::Value,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let ts = chrono::Utc::now().to_rfc3339();
    let input_pretty = serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string());
    writeln!(
        file,
        "\n## {ts} — {tool_name} (intercepted in Plan Mode)\n\n```json\n{input_pretty}\n```\n"
    )?;
    Ok(())
}

/// Open the plan file in `$EDITOR` (or platform default). Returns the
/// resolved plan path on success so the caller can surface it to the
/// user. Blocks until the editor process exits.
pub fn open_plan_in_editor(path: &std::path::Path) -> std::io::Result<()> {
    // Ensure the file exists before handing it to the editor so the user
    // always opens into a real file, not a blank buffer.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        std::fs::write(path, "# Archon plan\n\n")?;
    }
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                "notepad".to_string()
            } else {
                "vi".to_string()
            }
        });
    let status = std::process::Command::new(&editor).arg(path).status()?;
    if !status.success() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("editor '{editor}' exited with status {status}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_plan_file_returns_none_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("plan.md");
        assert!(read_plan_file(&path).unwrap().is_none());
    }

    #[test]
    fn read_plan_file_returns_content_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("plan.md");
        std::fs::write(&path, "hello").unwrap();
        assert_eq!(read_plan_file(&path).unwrap(), Some("hello".to_string()));
    }

    #[test]
    fn append_plan_entry_creates_file_and_parent_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sub").join(".archon").join("plan.md");
        append_plan_entry(&path, "Write", &serde_json::json!({"path":"/tmp/x"})).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Write"));
        assert!(content.contains("intercepted in Plan Mode"));
        assert!(content.contains("/tmp/x"));
    }

    #[test]
    fn append_plan_entry_appends_not_overwrites() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("plan.md");
        append_plan_entry(&path, "Write", &serde_json::json!({"a":1})).unwrap();
        append_plan_entry(&path, "Bash", &serde_json::json!({"cmd":"ls"})).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Write"));
        assert!(content.contains("Bash"));
    }

    #[test]
    fn plan_path_uses_working_dir_when_archon_dir_exists() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".archon")).unwrap();
        let p = plan_path(tmp.path());
        assert_eq!(p, tmp.path().join(".archon").join("plan.md"));
    }

    #[test]
    fn plan_path_falls_back_when_archon_dir_absent() {
        let tmp = tempfile::tempdir().unwrap();
        // No .archon/ dir in tmp — expect fallback to $HOME/.archon/plan.md.
        let p = plan_path(tmp.path());
        if let Some(home) = dirs::home_dir() {
            assert_eq!(p, home.join(".archon").join("plan.md"));
        }
    }
}
