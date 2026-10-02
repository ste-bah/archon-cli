//! Which PRD a task set without a frozen contract answers to (REM-13).
//!
//! Only what the host RECORDED for the task set decides it, never a file
//! name: the PRD an unfrozen candidate contract names (`prd.path`), else the
//! PRD a fixed decomposition of this very task root recorded (its
//! decomposition record's `identity.prd_identity`, in the run store this
//! run lives in). Records that disagree, a recorded PRD that is gone, or no record
//! at all leave the PRD unresolved, and the round says which -- the host
//! never guesses a PRD to author a contract against.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::command::workflow_decompose_state::FIXED_STATE_PATH as DECOMPOSITION_STATE;

fn canonical(path: &Path) -> Option<PathBuf> {
    path.canonicalize().map(archon_shell::paths::plain).ok()
}

/// Every PRD a fixed decomposition of `task_root` recorded, as recorded.
fn decomposition_prds(runs: &Path, task_root: &Path) -> Result<BTreeSet<String>, String> {
    let wanted = canonical(task_root);
    let mut prds = BTreeSet::new();
    let entries = match std::fs::read_dir(runs) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(prds),
        Err(error) => return Err(format!("{} cannot be listed: {error}", runs.display())),
    };
    for entry in entries.flatten() {
        let path = entry.path().join(DECOMPOSITION_STATE);
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let state: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
            format!("{} is not a decomposition record: {error}", path.display())
        })?;
        let identity = &state["identity"];
        let (Some(root), Some(prd)) = (
            identity["task_root_identity"].as_str(),
            identity["prd_identity"].as_str(),
        ) else {
            return Err(format!(
                "{} records no task root and PRD identity",
                path.display()
            ));
        };
        if wanted.is_some() && canonical(Path::new(root)) == wanted {
            prds.insert(prd.to_string());
        }
    }
    Ok(prds)
}

/// The PRD the task set at `task_root` answers to (see the module doc).
/// `candidate` is the PRD path an unfrozen candidate contract names,
/// resolved; `None` when there is no candidate.
pub(super) fn recorded(
    runs: &Path,
    task_root: &Path,
    candidate: Option<PathBuf>,
) -> Result<PathBuf, String> {
    if let Some(path) = candidate {
        return path.is_file().then_some(path.clone()).ok_or_else(|| {
            format!(
                "the unfrozen candidate contract names PRD {}, which does not exist",
                path.display()
            )
        });
    }
    let prds = decomposition_prds(runs, task_root)?;
    let mut prds = prds.into_iter();
    match (prds.next(), prds.next()) {
        (Some(prd), None) => {
            let path = PathBuf::from(&prd);
            path.is_file().then_some(path).ok_or_else(|| {
                format!(
                    "the decomposition of this task set recorded PRD {prd}, which does not exist"
                )
            })
        }
        (Some(first), Some(second)) => Err(format!(
            "decompositions of this task set recorded different PRDs ({first}, {second}), so which one it answers to is not recorded"
        )),
        (None, _) => Err(format!(
            "no PRD is recorded for {}: no candidate contract names one and no decomposition of it recorded one",
            task_root.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decomposition(runs: &Path, run: &str, task_root: &Path, prd: &Path) {
        let dir = runs.join(run).join("decomposition");
        std::fs::create_dir_all(&dir).unwrap();
        let state = serde_json::json!({"identity": {
            "task_root_identity": task_root.display().to_string(),
            "prd_identity": prd.display().to_string(),
        }});
        std::fs::write(dir.join("state.json"), state.to_string()).unwrap();
    }

    #[test]
    fn only_a_recorded_prd_is_used_never_a_name_convention() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path();
        let runs = project.join("runs");
        let root = project.join("tasks/PRD-LAKE-001");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(project.join("prds")).unwrap();
        let prd = project.join("prds/PRD-LAKE-001.md");
        std::fs::write(&prd, "# prd\n").unwrap();
        // A file named after the task set is no record.
        assert!(
            recorded(&runs, &root, None)
                .unwrap_err()
                .contains("no PRD is recorded")
        );
        // A decomposition of ANOTHER task root records nothing for this one.
        decomposition(&runs, "run-other", &project.join("tasks/other"), &prd);
        assert!(recorded(&runs, &root, None).is_err());
        decomposition(&runs, "run-1", &root, &prd);
        assert_eq!(recorded(&runs, &root, None).unwrap(), prd);
        // Two decompositions that disagree are never resolved by a guess.
        let other = project.join("prds/OTHER.md");
        std::fs::write(&other, "# other\n").unwrap();
        decomposition(&runs, "run-2", &root, &other);
        assert!(
            recorded(&runs, &root, None)
                .unwrap_err()
                .contains("different PRDs")
        );
        // A candidate's own PRD comes first; a missing one is an error.
        assert_eq!(recorded(&runs, &root, Some(other.clone())).unwrap(), other);
        assert!(recorded(&runs, &root, Some(project.join("gone.md"))).is_err());
    }
}
