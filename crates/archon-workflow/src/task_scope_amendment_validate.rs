//! The host's validation of one scope grant: which task, which path, which
//! root, and with whom it is shared. Agent text never decides any of it.

use std::path::Path;

use super::{ScopeAmendment, ScopeGrantKind, ScopeGrantRoot};
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::script::residual_paths::{
    is_repo_file, owners, project_data, protected, residual_forbidden,
};
use crate::write_coordinator::project_inputs::ProjectInputPolicy;

/// `raw` as a clean relative path: no root, no `.`/`..` or empty segment, no
/// glob, never a directory form. `None` otherwise.
pub(super) fn clean_path(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches("./");
    if trimmed.is_empty()
        || trimmed.ends_with('/')
        || trimmed.contains(['*', '?', '[', '\\'])
        || Path::new(trimmed).has_root()
    {
        return None;
    }
    let segments: Vec<&str> = trimmed.split('/').collect();
    segments
        .iter()
        .all(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
        .then(|| segments.join("/"))
}

/// The grant as the host will record it, or why it may not be made. The
/// root follows the path (project data always lands through the project
/// inputs), and `shared_with` is recomputed from the universe.
pub(super) fn grant(
    universe: &WorkflowV2TaskUniverse,
    repository_root: &Path,
    policy: Option<&ProjectInputPolicy>,
    grant: &ScopeAmendment,
) -> Result<ScopeAmendment, String> {
    if !universe
        .tasks
        .iter()
        .any(|task| task.canonical_task_id == grant.task_id)
    {
        return Err(format!("{} is no task of the universe", grant.task_id));
    }
    let path = clean_path(&grant.path)
        .ok_or_else(|| format!("`{}` is not one clean relative file path", grant.path))?;
    if protected(&path) {
        return Err(format!(
            "`{path}` is engine or run state or the frozen task set, which no grant opens"
        ));
    }
    let root = if project_data(&path) {
        ScopeGrantRoot::Project
    } else if grant.root == ScopeGrantRoot::Repository && git_ignores(repository_root, &path) {
        // The repository patch skips a path git ignores, so the change would
        // be lost: it lands through the project inputs when the project is
        // the repository, and no landing can carry it otherwise.
        let same = policy.is_some_and(|policy| {
            repository_root.canonicalize().ok().as_deref() == Some(policy.project.as_path())
        });
        if !same {
            return Err(format!(
                "`{path}` is ignored by the repository's git and the repository is not the project root, so neither the patch nor the project-input landing can carry it"
            ));
        }
        ScopeGrantRoot::Project
    } else {
        // Never the caller's choice: a repository path lands through the
        // branch's patch, whatever the grant says.
        ScopeGrantRoot::Repository
    };
    let declared_by = owners(universe, &path, repository_root);
    let declared_here = declared_by.contains(&grant.task_id);
    match root {
        ScopeGrantRoot::Project => {
            let policy = policy.ok_or_else(|| {
                format!(
                    "`{path}` is project data, but the run records no project root to land it in"
                )
            })?;
            let destination = policy
                .placed(&path, true)
                .map_err(|why| format!("`{path}` cannot land as project data: {why}"))?;
            if !destination.is_file() && !declared_here {
                return Err(format!(
                    "`{path}` is no file under the project root and {} does not declare it",
                    grant.task_id
                ));
            }
        }
        ScopeGrantRoot::Repository => {
            if !is_repo_file(repository_root, &path) && !declared_here {
                return Err(format!(
                    "`{path}` is no repository file and {} does not declare it",
                    grant.task_id
                ));
            }
        }
    }
    if grant.kind == ScopeGrantKind::DeclaredRestore && !declared_here {
        return Err(format!(
            "a declared-file restore of `{path}`, which {} does not declare",
            grant.task_id
        ));
    }
    // Batch O review: a grant never lifts anything the grantee forbids --
    // an exact entry included -- since its only callers act on what review
    // text names. A directory, basename or glob the grantee forbids stays
    // forbidden too.
    let forbids = universe
        .tasks
        .iter()
        .find(|task| task.canonical_task_id == grant.task_id)
        .map(|task| {
            archon_write_plan::ForbiddenPaths::from_entries(task.files_forbidden_to_change.iter())
                .matches(&path)
        })
        .unwrap_or(false);
    if forbids
        || residual_forbidden(
            universe,
            std::slice::from_ref(&grant.task_id),
            &[path.clone()],
        )
        .matches(&path)
    {
        return Err(format!("`{path}` is forbidden to {}", grant.task_id));
    }
    let mut shared_with = declared_by;
    shared_with.remove(&grant.task_id);
    Ok(ScopeAmendment {
        task_id: grant.task_id.clone(),
        path,
        kind: grant.kind,
        root,
        shared_with,
        evidence: grant.evidence.clone(),
    })
}

/// Whether the repository's git ignores `path` (and so its patch would skip
/// it). Not a git repository, or git unavailable: not ignored.
fn git_ignores(repository_root: &Path, path: &str) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(repository_root)
        .args(["check-ignore", "-q", "--"])
        .arg(path)
        .output()
        .is_ok_and(|output| output.status.code() == Some(0))
}
