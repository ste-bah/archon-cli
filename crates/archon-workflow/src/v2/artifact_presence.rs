//! Host-stamped existence for declared deliverables.
//!
//! Agents cannot be trusted to answer "does this project artifact exist?" —
//! not because they are careless, but because their filesystem lies to them.
//! A stage agent's working directory is an isolated worktree, which is a git
//! checkout, and a git checkout cannot contain gitignored or project-root
//! files. Observed live: the same deliverable globbed twice by one agent —
//! found via its absolute project path, "No files matched" via the relative
//! pattern — and three consecutive runs bricked on reducers concluding a
//! report was absent while it sat in the project root the whole time.
//!
//! So the host answers the question instead. At driver construction, every
//! `deliverable_contracts` entry in the task universe gains an
//! `artifact_status` object — resolved path, whether it exists, its size, and
//! which root resolved it — computed against the project artifact root first
//! and the repository root second, the same order the verification prompts
//! already teach. Prompts then tell agents this field is authoritative and
//! that absence may never be inferred from a checkout listing.
//!
//! Stamped once, at run start. That is the moment discovery and noop-proof
//! read it, which is where absence-lies did their damage; later tiers
//! re-verify against the live roots they are already given.

use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// Add `artifact_status` to every deliverable contract in the universe value.
///
/// A contract whose `artifact_path` still carries an unexpanded `${...}`
/// template is left unstamped: resolving it would stat a literal `${VAR}`
/// path and report a confident false absence.
pub fn stamp_artifact_presence(
    universe: &mut Value,
    project_artifact_root: Option<&str>,
    target_repository_root: Option<&str>,
) {
    let Some(tasks) = universe.get_mut("tasks").and_then(Value::as_array_mut) else {
        return;
    };
    for task in tasks {
        let Some(contracts) = task
            .get_mut("deliverable_contracts")
            .and_then(Value::as_array_mut)
        else {
            continue;
        };
        for contract in contracts {
            let Some(object) = contract.as_object_mut() else {
                continue;
            };
            let Some(raw) = object
                .get("artifact_path")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|path| !path.is_empty() && !path.contains("${"))
            else {
                continue;
            };
            let status = presence_of(raw, project_artifact_root, target_repository_root);
            object.insert("artifact_status".to_string(), status);
        }
    }
}

/// Resolve one declared path and report what is actually on disk.
///
/// Project root first, repository root second: a deliverable that exists in
/// both places resolves to the project copy, because that is where project
/// artifacts live and the repository copy is the anomaly.
fn presence_of(
    raw: &str,
    project_artifact_root: Option<&str>,
    target_repository_root: Option<&str>,
) -> Value {
    let candidates: Vec<(&'static str, PathBuf)> = if Path::new(raw).is_absolute() {
        vec![("absolute", PathBuf::from(raw))]
    } else {
        let mut list = Vec::new();
        if let Some(root) = project_artifact_root {
            list.push(("project", Path::new(root).join(raw)));
        }
        if let Some(root) = target_repository_root {
            list.push(("repository", Path::new(root).join(raw)));
        }
        list
    };

    for (root, path) in &candidates {
        if let Ok(meta) = std::fs::metadata(path) {
            let mut status = Map::new();
            status.insert(
                "resolved_path".to_string(),
                Value::String(path.display().to_string()),
            );
            status.insert("exists".to_string(), Value::Bool(true));
            status.insert("bytes".to_string(), Value::from(meta.len()));
            status.insert("root".to_string(), Value::String((*root).to_string()));
            return Value::Object(status);
        }
    }

    let mut status = Map::new();
    if let Some((root, path)) = candidates.first() {
        status.insert(
            "resolved_path".to_string(),
            Value::String(path.display().to_string()),
        );
        status.insert("root".to_string(), Value::String((*root).to_string()));
    }
    status.insert("exists".to_string(), Value::Bool(false));
    Value::Object(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn universe_with(path: &str) -> Value {
        serde_json::json!({
            "tasks": [{
                "canonical_task_id": "TASK-X-001",
                "deliverable_contracts": [
                    { "kind": "report", "artifact_path": path }
                ]
            }]
        })
    }

    fn status_of(universe: &Value) -> &Value {
        &universe["tasks"][0]["deliverable_contracts"][0]["artifact_status"]
    }

    /// The live case: the artifact exists in the project root, and nowhere in
    /// any repository checkout. The stamp must say so.
    #[test]
    fn finds_a_project_artifact_the_repo_cannot_contain() {
        let project = tempfile::tempdir().expect("project");
        let repo = tempfile::tempdir().expect("repo");
        std::fs::create_dir_all(project.path().join("docs/reports")).expect("mkdir");
        std::fs::write(project.path().join("docs/reports/audit.md"), "# audit\n").expect("write");

        let mut universe = universe_with("docs/reports/audit.md");
        stamp_artifact_presence(
            &mut universe,
            Some(&project.path().display().to_string()),
            Some(&repo.path().display().to_string()),
        );
        let status = status_of(&universe);
        assert_eq!(status["exists"], Value::Bool(true));
        assert_eq!(status["root"], Value::String("project".into()));
        assert_eq!(status["bytes"], Value::from(8u64));
    }

    #[test]
    fn falls_back_to_the_repository_root() {
        let project = tempfile::tempdir().expect("project");
        let repo = tempfile::tempdir().expect("repo");
        std::fs::create_dir_all(repo.path().join("src")).expect("mkdir");
        std::fs::write(repo.path().join("src/lib.rs"), "// code\n").expect("write");

        let mut universe = universe_with("src/lib.rs");
        stamp_artifact_presence(
            &mut universe,
            Some(&project.path().display().to_string()),
            Some(&repo.path().display().to_string()),
        );
        let status = status_of(&universe);
        assert_eq!(status["exists"], Value::Bool(true));
        assert_eq!(status["root"], Value::String("repository".into()));
    }

    /// Absence is stamped as absence — with the path that was checked, so an
    /// agent can cite where the host looked rather than where it guessed.
    #[test]
    fn absence_is_explicit_and_names_the_checked_path() {
        let project = tempfile::tempdir().expect("project");
        let mut universe = universe_with("docs/missing.md");
        stamp_artifact_presence(
            &mut universe,
            Some(&project.path().display().to_string()),
            None,
        );
        let status = status_of(&universe);
        assert_eq!(status["exists"], Value::Bool(false));
        assert!(
            status["resolved_path"]
                .as_str()
                .is_some_and(|p| p.ends_with("docs/missing.md"))
        );
    }

    /// An unexpanded template must not be statted: `${VAR}` as a literal path
    /// would report a confident false absence.
    #[test]
    fn templated_paths_are_left_unstamped() {
        let mut universe = universe_with("${PROJECT_ROOT}/data/out.json");
        stamp_artifact_presence(&mut universe, Some("/tmp"), None);
        assert!(
            universe["tasks"][0]["deliverable_contracts"][0]
                .get("artifact_status")
                .is_none()
        );
    }
}
