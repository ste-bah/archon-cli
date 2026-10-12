//! Input-scoped audit and immutable per-observation project baseline.
use super::*;
use std::io::Read;

pub(crate) fn excluded(path: &Path, excludes: &[PathBuf]) -> bool {
    acceptance_input_excluded(path, excludes, None)
}

/// One project-relative predicate for inputs copied, audited, or digested by
/// acceptance checks. `task_root` is project-relative; pass an empty path when
/// copying the task root itself.
pub(crate) fn acceptance_input_excluded(
    path: &Path,
    excludes: &[PathBuf],
    task_root: Option<&Path>,
) -> bool {
    const BUILT_IN_ROOTS: &[&str] = &[
        "credentials",
        "credentials.toml",
        "config.toml",
        "config.json",
        ".env",
        ".archon/config.toml",
        "target",
        ".git",
    ];
    path.starts_with(Path::new(".archon").join(crate::task_set_lineage::PIN_STORE_NAMESPACE))
        || excludes.iter().any(|e| path.starts_with(e))
        || BUILT_IN_ROOTS.iter().any(|root| path.starts_with(root))
        || path.starts_with(".archon/workflows")
        || acceptance_harness_run_state(path)
        || task_root.is_some_and(|root| {
            path.strip_prefix(root)
                .ok()
                .is_some_and(task_root_pipeline_output)
        })
        || path.starts_with(".git")
}

fn task_root_pipeline_output(path: &Path) -> bool {
    path == Path::new(crate::task_set_contract::ACCEPTANCE_CONTRACT_FILE)
        || path == Path::new(crate::task_set_contract::ACCEPTANCE_LOCK_FILE)
        || path == Path::new(crate::task_set_contract::TASK_SKELETON_FILE)
        || path == Path::new(crate::task_set_contract::TASK_SKELETON_LOCK_FILE)
        || path == Path::new(".decompose.log")
        || (path.components().count() == 1
            && path.file_name().is_some_and(|name| {
                let name = name.to_string_lossy();
                name.starts_with("TASK-") && name.ends_with(".md")
            }))
}

/// Project-relative paths created or appended by the host while running checks.
/// Keep this shared by scratch export and the host's hermetic digest.
pub(crate) fn acceptance_harness_run_state(path: &Path) -> bool {
    const DIRECTORIES: &[(&str, &str)] = &[
        // src/command/workflow_gate.rs:append_shadow_records
        (
            ".archon/logs",
            "src/command/workflow_gate.rs:append_shadow_records",
        ),
        // src/command/topology_lint/fidelity_store.rs:store_path
        (
            ".archon/lint-cache",
            "src/command/topology_lint/fidelity_store.rs:store_path",
        ),
        // src/session/cognitive_store.rs:open
        (".archon/cognitive", "src/session/cognitive_store.rs:open"),
        // crates/archon-core/src/spill.rs:spill_root
        (
            ".archon/spill",
            "crates/archon-core/src/spill.rs:spill_root",
        ),
        // crates/archon-topology/src/trace/paths.rs:TopologyPaths::for_project
        (
            ".archon/topology",
            "crates/archon-topology/src/trace/paths.rs:TopologyPaths::for_project",
        ),
        // src/command/topology_fold/schema.rs:topology_db_path
        (
            ".archon/topology.db",
            "src/command/topology_fold/schema.rs:topology_db_path",
        ),
        // src/command/workflow_freeze_budget.rs:FREEZE_CACHE_DIR
        (
            ".archon/freeze-cache",
            "src/command/workflow_freeze_budget.rs:FREEZE_CACHE_DIR",
        ),
        // crates/archon-workflow/src/store.rs:WorkflowStore::new
        (
            ".archon/workflows",
            "crates/archon-workflow/src/store.rs:WorkflowStore::new",
        ),
        // crates/archon-workflow/src/task_set_lineage.rs:PIN_STORE_NAMESPACE
        (
            ".archon/task-set-pins",
            "crates/archon-workflow/src/task_set_lineage.rs:PIN_STORE_NAMESPACE",
        ),
    ];
    if DIRECTORIES
        .iter()
        .any(|(root, _writer)| path.starts_with(root))
    {
        return true;
    }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    // src/command/topology_fold/workflow_learning.rs:open_db uses Cozo;
    // src/session/leann_startup.rs:open uses the same Cozo sidecars.
    ["learning-state.db", "leann.db", "topology.db"]
        .iter()
        .any(|database| {
            name == *database
                || name.strip_prefix(database).is_some_and(|suffix| {
                    matches!(suffix, "-wal" | "-journal" | ".archon-cozo-write.lock")
                        || suffix == ".archon-cozo-write.lock.progress"
                })
        })
        && path.parent() == Some(Path::new(".archon"))
}

pub(crate) fn freeze_owned(path: &Path) -> bool {
    path.starts_with(Path::new(".archon").join(crate::task_set_lineage::PIN_STORE_NAMESPACE))
}

pub(super) fn project_excludes(policy: &ScratchPolicy) -> WorkflowResult<Vec<PathBuf>> {
    let project = archon_shell::paths::canonicalize(&policy.project)
        .map_err(|error| WorkflowError::io(&policy.project, error))?;
    let tasks = archon_shell::paths::canonicalize(&policy.task_root)
        .map_err(|error| WorkflowError::io(&policy.task_root, error))?;
    let task_relative = tasks
        .strip_prefix(&project)
        .map_err(|_| invalid("task root must belong to declared project"))?;
    let mut excludes = policy.project_input_excludes.clone();
    excludes.extend([
        task_relative.join(crate::task_set_contract::ACCEPTANCE_CONTRACT_FILE),
        task_relative.join(crate::task_set_contract::ACCEPTANCE_LOCK_FILE),
    ]);
    Ok(excludes)
}
pub(super) fn copy_project(
    root: &Path,
    relative: &Path,
    dest: &Path,
    excludes: &[PathBuf],
    task_root: &Path,
    remaining: &mut u64,
) -> WorkflowResult<()> {
    control::check()?;
    if acceptance_input_excluded(relative, excludes, Some(task_root)) {
        return Ok(());
    }
    let source = root.join(relative);
    let meta = source
        .symlink_metadata()
        .map_err(|e| WorkflowError::io(&source, e))?;
    if meta.is_dir() {
        std::fs::create_dir_all(dest.join(relative)).map_err(|e| WorkflowError::io(dest, e))?;
        for item in std::fs::read_dir(&source).map_err(|e| WorkflowError::io(&source, e))? {
            let item = item.map_err(|e| WorkflowError::io(&source, e))?;
            copy_project(
                root,
                &relative.join(item.file_name()),
                dest,
                excludes,
                task_root,
                remaining,
            )?;
        }
    } else if meta.is_file() {
        io::copy_tree(&source, &dest.join(relative), remaining, false)?;
    }
    // Sockets/FIFOs/symlinks are not snapshot inputs and are never followed.
    Ok(())
}
pub(super) fn snapshot_project(
    project: &Path,
    snapshot: &Path,
    remaining: &mut u64,
) -> WorkflowResult<()> {
    std::fs::create_dir(snapshot).map_err(|e| WorkflowError::io(snapshot, e))?;
    for item in std::fs::read_dir(project).map_err(|e| WorkflowError::io(project, e))? {
        let item = item.map_err(|e| WorkflowError::io(project, e))?;
        if item.file_name() == "target" || item.file_name() == ".git" {
            continue;
        }
        io::copy_tree(
            &item.path(),
            &snapshot.join(item.file_name()),
            remaining,
            true,
        )?;
    }
    Ok(())
}
pub(super) fn reset_project(roots: &ScratchRoots) -> WorkflowResult<()> {
    let baseline = roots.root().join("project-baseline");
    let before = inventory(&baseline)?;
    let current = inventory(roots.project())?;
    // Remove new/changed paths deepest first, but never replace source files
    // whose bytes already match (Cargo freshness depends on their mtimes).
    let mut remove = current
        .keys()
        .filter(|k| {
            !k.is_empty() && *k != "target" && *k != ".git" && current.get(*k) != before.get(*k)
        })
        .cloned()
        .collect::<Vec<_>>();
    remove.sort_by_key(|k| std::cmp::Reverse(k.len()));
    for name in remove {
        io::remove_owned_tree(&roots.project().join(name))?;
    }
    let mut remaining = u64::MAX;
    for item in std::fs::read_dir(&baseline).map_err(|e| WorkflowError::io(&baseline, e))? {
        let item = item.map_err(|e| WorkflowError::io(&baseline, e))?;
        io::copy_tree(
            &item.path(),
            &roots.project().join(item.file_name()),
            &mut remaining,
            true,
        )?;
    }
    for name in ["tmp", "home"] {
        let path = roots.root().join(name);
        io::remove_owned_tree(&path)?;
        std::fs::create_dir(&path).map_err(|e| WorkflowError::io(&path, e))?;
    }
    Ok(())
}
fn file_digest(path: &Path) -> WorkflowResult<String> {
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut f = opts.open(path).map_err(|e| WorkflowError::io(path, e))?;
    let mut hash = blake3::Hasher::new();
    let mut buf = [0; 65536];
    loop {
        control::check()?;
        let n = f.read(&mut buf).map_err(|e| WorkflowError::io(path, e))?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(hash.finalize().to_hex().to_string())
}
fn audit_path(
    root: &Path,
    rel: &Path,
    excludes: &[PathBuf],
    task_root: Option<&Path>,
    map: &mut BTreeMap<String, String>,
) -> WorkflowResult<()> {
    control::check()?;
    if acceptance_input_excluded(rel, excludes, task_root) {
        return Ok(());
    }
    let path = root.join(rel);
    let meta = match path.symlink_metadata() {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            map.insert(rel.display().to_string(), "missing".into());
            return Ok(());
        }
        Err(e) => return Err(WorkflowError::io(&path, e)),
    };
    if meta.is_file() {
        map.insert(rel.display().to_string(), file_digest(&path)?);
    } else if meta.is_dir() {
        map.insert(rel.display().to_string(), "directory".into());
        for item in std::fs::read_dir(&path).map_err(|e| WorkflowError::io(&path, e))? {
            audit_path(
                root,
                &rel.join(item.map_err(|e| WorkflowError::io(&path, e))?.file_name()),
                excludes,
                task_root,
                map,
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "acceptance_scratch_inputs_tests.rs"]
mod tests;

pub(super) fn live(
    policy: &ScratchPolicy,
    commit: &str,
) -> WorkflowResult<BTreeMap<String, BTreeMap<String, String>>> {
    let project_excludes = project_excludes(policy)?;
    let mut source = BTreeMap::new();
    let files = git(
        &policy.repository,
        &["ls-tree", "-r", "--name-only", "-z", commit],
        &[],
    )?;
    for name in files.split('\0').filter(|n| !n.is_empty()) {
        let path = Path::new(name);
        if !relative(path) {
            return Err(invalid("unsafe recorded source path"));
        }
        // Only named tracked files, never descend into untracked directory replacements.
        let live = policy.repository.join(path);
        let value = match live.symlink_metadata() {
            Ok(m) if m.is_file() => file_digest(&live)?,
            _ => "missing-or-nonregular".into(),
        };
        source.insert(name.into(), value);
    }
    let mut project = BTreeMap::new();
    let project_root = archon_shell::paths::canonicalize(&policy.project)
        .map_err(|error| WorkflowError::io(&policy.project, error))?;
    let task_root = archon_shell::paths::canonicalize(&policy.task_root)
        .map_err(|error| WorkflowError::io(&policy.task_root, error))?;
    let task_relative = task_root
        .strip_prefix(&project_root)
        .map_err(|_| invalid("task root must belong to declared project"))?;
    for input in &policy.project_inputs {
        audit_path(
            &policy.project,
            input,
            &project_excludes,
            Some(task_relative),
            &mut project,
        )?;
    }
    let mut tasks = BTreeMap::new();
    // Audit the task tree in project-relative coordinates so the live
    // tripwire applies the same exclusions as the scratch copy and digest.
    audit_path(
        &policy.project,
        task_relative,
        &project_excludes,
        Some(task_relative),
        &mut tasks,
    )?;
    Ok(BTreeMap::from([
        (policy.repository.display().to_string(), source),
        (policy.project.display().to_string(), project),
        (policy.task_root.display().to_string(), tasks),
    ]))
}
/// Batch G (D): a collision while overlaying a project input names both
/// sources: the project root's copy and, in the combined view, the
/// repository's tracked copy at the recorded commit.
pub(super) fn name_collision(
    error: WorkflowError,
    policy: &ScratchPolicy,
    project: &Path,
    input: &Path,
    commit: &str,
) -> WorkflowError {
    match error {
        WorkflowError::SpecInvalid(message)
            if message.starts_with("nonidentical scratch path collision") =>
        {
            let earlier = if policy.combined {
                format!(
                    "the repository's tracked copy at commit {commit} ({})",
                    policy.repository.display()
                )
            } else {
                "an earlier project input".to_string()
            };
            invalid(format!(
                "{message}: project input {} from the project root ({}) differs from {earlier}; a tracked project input changed outside the host's landings",
                input.display(),
                project.join(input).display()
            ))
        }
        other => other,
    }
}
