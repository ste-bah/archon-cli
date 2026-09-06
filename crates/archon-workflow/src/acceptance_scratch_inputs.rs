//! Input-scoped audit and immutable per-observation project baseline.
use super::*;
use std::io::Read;

fn excluded(path: &Path, excludes: &[PathBuf]) -> bool {
    excludes.iter().any(|e| path.starts_with(e))
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
    use std::os::unix::fs::OpenOptionsExt;
    opts.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
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
pub(super) fn live(
    policy: &ScratchPolicy,
    commit: &str,
) -> WorkflowResult<BTreeMap<String, BTreeMap<String, String>>> {
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
        audit_path(
            &policy.project,
            input,
            &policy.project_input_excludes,
            &mut project,
        )?;
    }
    let mut tasks = BTreeMap::new();
    audit_path(&policy.task_root, Path::new(""), &[], &mut tasks)?;
    Ok(BTreeMap::from([
        (policy.repository.display().to_string(), source),
        (policy.project.display().to_string(), project),
        (policy.task_root.display().to_string(), tasks),
    ]))
}
