//! Input-scoped audit and immutable per-observation project baseline.
use super::*;
use std::io::Read;

pub(crate) fn excluded(path: &Path, excludes: &[PathBuf]) -> bool {
    path.starts_with(Path::new(".archon").join(crate::task_set_lineage::PIN_STORE_NAMESPACE))
        || excludes.iter().any(|e| path.starts_with(e))
        || matches!(
            path.to_str(),
            Some(
                "credentials"
                    | "credentials.toml"
                    | "config.toml"
                    | "config.json"
                    | ".env"
                    | ".archon/config.toml"
            )
        )
        || path.starts_with(".archon/workflows")
        || path.starts_with(".git")
}

pub(crate) fn freeze_owned(path: &Path) -> bool {
    path.starts_with(Path::new(".archon").join(crate::task_set_lineage::PIN_STORE_NAMESPACE))
}

pub(super) fn project_excludes(policy: &ScratchPolicy) -> WorkflowResult<Vec<PathBuf>> {
    let project = std::fs::canonicalize(&policy.project)
        .map_err(|error| WorkflowError::io(&policy.project, error))?;
    let tasks = std::fs::canonicalize(&policy.task_root)
        .map_err(|error| WorkflowError::io(&policy.task_root, error))?;
    let relative = tasks
        .strip_prefix(&project)
        .map_err(|_| invalid("task root must belong to declared project"))?;
    let mut excludes = policy.project_input_excludes.clone();
    excludes.push(relative.join(crate::task_set_contract::ACCEPTANCE_CONTRACT_FILE));
    excludes.push(relative.join(crate::task_set_contract::ACCEPTANCE_LOCK_FILE));
    Ok(excludes)
}
pub(super) fn copy_project(
    root: &Path,
    relative: &Path,
    dest: &Path,
    excludes: &[PathBuf],
    remaining: &mut u64,
) -> WorkflowResult<()> {
    control::check()?;
    if excluded(relative, excludes) {
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
    map: &mut BTreeMap<String, String>,
) -> WorkflowResult<()> {
    control::check()?;
    if excluded(rel, excludes) {
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
                map,
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[test]
fn freeze_outputs_are_excluded_from_scratch_project_inputs() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let task_root = project.join("tasks/set");
    std::fs::create_dir_all(&task_root).unwrap();
    let policy = ScratchPolicy {
        repository: temp.path().join("repository"),
        project: project.clone(),
        task_root,
        scratch_parent: temp.path().join("scratch"),
        project_inputs: Vec::new(),
        project_input_excludes: Vec::new(),
        combined: false,
        toolchain_path: String::new(),
        environment: BTreeMap::new(),
        environment_allowlist: Vec::new(),
        cargo_seed: None,
        timeout_secs: 1,
        output_bytes: 1,
        scratch_bytes: 1,
        build_cache: None,
    };
    let excludes = project_excludes(&policy).unwrap();
    let task_contract =
        Path::new("tasks/set").join(crate::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
    assert!(excluded(&task_contract, &excludes));
    assert!(!excluded(
        Path::new(crate::task_set_contract::ACCEPTANCE_CONTRACT_FILE),
        &excludes
    ));
    for path in [
        Path::new("tasks/set").join(crate::task_set_contract::ACCEPTANCE_LOCK_FILE),
        Path::new(".archon")
            .join(crate::task_set_lineage::PIN_STORE_NAMESPACE)
            .join("set.json"),
    ] {
        assert!(excluded(&path, &excludes), "{}", path.display());
    }
    assert!(!excluded(Path::new("tasks/set/input.json"), &excludes));
}

#[cfg(test)]
#[test]
fn same_named_nested_project_data_is_included_and_digested() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("config/acceptance-contract.json");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"project data").unwrap();
    let relative = Path::new("config/acceptance-contract.json");
    assert!(!excluded(relative, &[]));
    let mut map = BTreeMap::new();
    audit_path(temp.path(), relative, &[], &mut map).unwrap();
    assert_eq!(
        map.get("config/acceptance-contract.json"),
        Some(&file_digest(&file).unwrap())
    );
}
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
    for input in &policy.project_inputs {
        audit_path(&policy.project, input, &project_excludes, &mut project)?;
    }
    let mut tasks = BTreeMap::new();
    let task_outputs = [
        PathBuf::from(crate::task_set_contract::ACCEPTANCE_CONTRACT_FILE),
        PathBuf::from(crate::task_set_contract::ACCEPTANCE_LOCK_FILE),
    ];
    audit_path(&policy.task_root, Path::new(""), &task_outputs, &mut tasks)?;
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
