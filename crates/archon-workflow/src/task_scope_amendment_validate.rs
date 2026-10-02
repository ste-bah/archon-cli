//! The host's validation of one scope grant: which task, which path, which
//! root, and with whom it is shared. Agent text never decides any of it.

use std::path::Path;

use super::{DeclaredDataRoots, ScopeAmendment, ScopeGrantKind, ScopeGrantRoot};
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
/// inputs), and `shared_with` is recomputed from the universe. Project data
/// is a path under `.archon/<namespace>/`, or -- asked for as project data
/// -- one under a data root the run's records declare (`roots`, Issue-223).
pub(super) fn grant(
    universe: &WorkflowV2TaskUniverse,
    repository_root: &Path,
    policy: Option<&ProjectInputPolicy>,
    roots: Option<&DeclaredDataRoots>,
    grant: &ScopeAmendment,
) -> Result<ScopeAmendment, String> {
    if !universe
        .tasks
        .iter()
        .any(|task| task.canonical_task_id == grant.task_id)
    {
        return Err(format!("{} is no task of the universe", grant.task_id));
    }
    if grant.root == ScopeGrantRoot::External {
        let path = external_path(policy, roots, grant)?;
        return finish(
            universe,
            repository_root,
            grant,
            path,
            ScopeGrantRoot::External,
        );
    }
    let path = clean_path(&grant.path)
        .ok_or_else(|| format!("`{}` is not one clean relative file path", grant.path))?;
    if protected(&path) {
        return Err(format!(
            "`{path}` is engine or run state or the frozen task set, which no grant opens"
        ));
    }
    let same = policy.is_some_and(|policy| {
        repository_root.canonicalize().ok().as_deref() == Some(policy.project.as_path())
    });
    let asked_as_data = grant.root == ScopeGrantRoot::Project && !project_data(&path);
    let declared_data = asked_as_data && roots.is_some_and(|roots| roots.covers_project(&path));
    let root = if project_data(&path) {
        ScopeGrantRoot::Project
    } else if declared_data && (!same || git_ignores(repository_root, &path)) {
        // Stored data under a root the run declares lands like
        // `.archon/<namespace>/` data -- unless the project is the
        // repository and git carries the path, when the patch does.
        ScopeGrantRoot::Project
    } else if asked_as_data && !same && policy.is_some() {
        return Err(format!(
            "`{path}` is asked for as project data, but it is neither under `.archon/<namespace>/` nor under a data root the run's records declare (every link resolved, none escaping it), so no grant opens it"
        ));
    } else if grant.root == ScopeGrantRoot::Repository && git_ignores(repository_root, &path) {
        // The repository patch skips a path git ignores, so the change would
        // be lost: it lands through the project inputs when the project is
        // the repository, and no landing can carry it otherwise.
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
        // Never computed above: an External grant returned early.
        ScopeGrantRoot::External => {}
    }
    finish(universe, repository_root, grant, path, root)
}

/// Issue-226: an External grant's path, or why it may not be made. It must
/// be absolute and its own resolved path, admitted by the run policy's
/// allowlist (`EXTERNAL_ROOTS_KEY`), and under a data root the run's
/// records declare; every refusal names the root and the key.
fn external_path(
    policy: Option<&ProjectInputPolicy>,
    roots: Option<&DeclaredDataRoots>,
    grant: &ScopeAmendment,
) -> Result<String, String> {
    use crate::write_coordinator::project_inputs::EXTERNAL_ROOTS_KEY;
    let path = grant.path.trim();
    let policy = policy.ok_or_else(|| {
        format!("`{path}` is external data, but the run records no policy to admit it")
    })?;
    let (_, destination) = policy.landing(path, true)?;
    if destination != Path::new(path) {
        return Err(format!(
            "`{path}` is not its own resolved path ({}): a link on it is never granted under `{EXTERNAL_ROOTS_KEY}`",
            destination.display()
        ));
    }
    if !roots.is_some_and(|roots| roots.covers_external(&destination)) {
        return Err(format!(
            "`{path}` is inside a directory `{EXTERNAL_ROOTS_KEY}` lists, but under no data root the run's records declare, so no grant opens it"
        ));
    }
    Ok(path.to_string())
}

/// What every grant is judged by last, whatever its root: who declares it,
/// a restore only of a declared file, never anything the grantee forbids.
fn finish(
    universe: &WorkflowV2TaskUniverse,
    repository_root: &Path,
    grant: &ScopeAmendment,
    path: String,
    root: ScopeGrantRoot,
) -> Result<ScopeAmendment, String> {
    let mut declared_by = owners(universe, &path, repository_root);
    if root == ScopeGrantRoot::External {
        declared_by.extend(
            (universe.tasks.iter())
                .filter(|task| task.artifact_requirements.iter().any(|a| a.trim() == path))
                .map(|task| task.canonical_task_id.clone()),
        );
    }
    let declared_here = declared_by.contains(&grant.task_id);
    if root == ScopeGrantRoot::External && !Path::new(&path).is_file() && !declared_here {
        return Err(format!(
            "`{path}` is no file under its external data root and {} does not declare it",
            grant.task_id
        ));
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
            std::slice::from_ref(&path),
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
