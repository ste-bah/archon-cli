//! Base-only copies retained for later proof stages.
use super::*;
use crate::command::workflow_host_probe_fault::UnrunCause;
use archon_workflow::base_snapshot::{
    BaseSnapshot, SnapshotEntry, key_path, skip_copy_entry, snapshot_excludes,
};

pub(crate) struct BaseCopy {
    copy: HermeticCopy,
    commit: String,
}
impl BaseCopy {
    pub(crate) fn repository(&self) -> &Path {
        &self.copy.repository
    }
    pub(crate) fn project(&self) -> &Path {
        &self.copy.project
    }
    pub(crate) fn layout(&self) -> CopyLayout {
        self.copy.layout()
    }
    pub(crate) fn commit(&self) -> &str {
        &self.commit
    }
    pub(crate) fn copy(&self) -> &HermeticCopy {
        &self.copy
    }
    pub(crate) fn remove(self) -> Result<(), String> {
        self.copy.remove()
    }
}

pub(crate) fn materialise_base_copy(
    parent: &Path,
    repository: &Path,
    project: &Path,
    task_root: Option<&Path>,
    snapshot: &BaseSnapshot,
    lock_commit: &str,
    build_output: &Path,
    transport: Option<&Path>,
) -> Result<BaseCopy, Unrun> {
    // Every anchored proof uses its own build-output directory.
    let keys = snapshot
        .entries
        .keys()
        .cloned()
        .collect::<Vec<_>>()
        .join(",");
    if snapshot.commit != lock_commit {
        return Err(Unrun::refused(
            UnrunCause::SnapshotTampered,
            format!("commit differs from lock ({keys})"),
        ));
    }
    if !snapshot.valid_digest() {
        return Err(Unrun::refused(
            UnrunCause::SnapshotTampered,
            format!("digest differs ({keys})"),
        ));
    }
    baseline_available(repository, lock_commit)?;
    let canonical = |p: &Path| {
        archon_shell::paths::canonicalize(p)
            .map_err(|e| Unrun::error(e.into(), format!("{}", p.display())))
    };
    let repository = canonical(repository)?;
    let project = canonical(project)?;
    let excludes = snapshot_excludes(&repository, &project, repository == project, snapshot);
    let mut copy = HermeticCopy::create(
        parent,
        &repository,
        &project,
        task_root,
        lock_commit,
        &excludes,
        true,
        transport,
    )?;
    for (key, entry) in &snapshot.entries {
        if matches!(
            entry,
            SnapshotEntry::Unverified {
                reason: archon_workflow::base_snapshot::UnverifiedReason::Submodule,
                ..
            }
        ) {
            let listed = output(&copy.repository, &["ls-tree", "-z", "HEAD", "--", key])?;
            if listed.starts_with("160000 ") {
                let path = copy.repository.join(key_path(key));
                if path.symlink_metadata().is_ok() {
                    std::fs::remove_dir(&path).map_err(|error| {
                        Unrun::error(
                            error.into(),
                            format!("{key}: removing unverified placeholder"),
                        )
                    })?;
                }
            }
        }
    }
    let guard = BaseGuard {
        build_output: build_output.to_path_buf(),
        ignore_case: ignore_case(&repository)?,
        snapshot: snapshot.clone(),
        live_head: Some(
            output(&repository, &["rev-parse", "--verify", "HEAD"])?
                .trim()
                .to_string(),
        ),
        live_repository: repository,
        live_project: project,
    };
    guard.tripwire(&copy)?;
    copy.base_guard = Some(guard);
    Ok(BaseCopy {
        copy,
        commit: lock_commit.into(),
    })
}

pub(super) struct BaseGuard {
    pub(super) build_output: PathBuf,
    ignore_case: bool,
    snapshot: BaseSnapshot,
    live_head: Option<String>,
    live_repository: PathBuf,
    live_project: PathBuf,
}
pub(super) fn output(repository: &Path, args: &[&str]) -> Result<String, Unrun> {
    let result = archon_shell::spawn::command("git")
        .arg("-C")
        .arg(repository)
        .arg("--literal-pathspecs")
        .args(args)
        .output()
        .map_err(|e| Unrun::error(e.into(), "reading copy evidence".into()))?;
    if !result.status.success() {
        let cause = String::from_utf8_lossy(&result.stderr).trim().to_string();
        return Err(Unrun::unknown(if cause.is_empty() {
            format!("{} exited with {}", args.join(" "), result.status)
        } else {
            cause
        }));
    }
    Ok(String::from_utf8_lossy(&result.stdout).into_owned())
}
pub(super) fn baseline_available(repository: &Path, commit: &str) -> Result<(), Unrun> {
    let output = archon_shell::spawn::command("git")
        .arg("-C")
        .arg(repository)
        .args([
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{commit}^{{commit}}"),
        ])
        .output()
        .map_err(|error| Unrun::error(error.into(), "reading anchored revision".into()))?;
    if output.status.success() {
        return Ok(());
    }
    let cause = if output.status.code() == Some(1) {
        UnrunCause::BaselineUnavailable
    } else {
        UnrunCause::BaselineFetchFailed
    };
    Err(Unrun::refused(
        cause,
        format!(
            "{commit}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    ))
}
fn object_absent(repository: &Path, object: &str) -> Result<bool, Unrun> {
    let output = archon_shell::spawn::command("git")
        .arg("-C")
        .arg(repository)
        .args(["cat-file", "-e", object])
        .output()
        .map_err(|error| Unrun::error(error.into(), "checking object isolation".into()))?;
    match output.status.code() {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => Err(Unrun::unknown(
            String::from_utf8_lossy(&output.stderr).trim().into(),
        )),
    }
}
fn ignore_case(repository: &Path) -> Result<bool, Unrun> {
    let output = archon_shell::spawn::command("git")
        .arg("-C")
        .arg(repository)
        .args(["config", "--bool", "core.ignorecase"])
        .output()
        .map_err(|error| Unrun::error(error.into(), "reading path case policy".into()))?;
    match output.status.code() {
        Some(0) => Ok(output.stdout.trim_ascii() == b"true"),
        Some(1) => Ok(false),
        _ => Err(Unrun::unknown(
            String::from_utf8_lossy(&output.stderr).trim().into(),
        )),
    }
}
fn starts_with(path: &Path, prefix: &Path, insensitive: bool) -> bool {
    let parts: Vec<_> = path.components().collect();
    let parents: Vec<_> = prefix.components().collect();
    parts.len() >= parents.len()
        && parts.iter().zip(&parents).all(|(a, b)| {
            a == b
                || insensitive
                    && a.as_os_str()
                        .to_string_lossy()
                        .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
        })
}
fn present_at(root: &Path, relative: &Path, insensitive: bool) -> Result<bool, Unrun> {
    let io = |path: &Path, error: std::io::Error| {
        Unrun::error(error.into(), format!("{}", path.display()))
    };
    if !insensitive {
        return match root.join(relative).symlink_metadata() {
            Ok(_) => Ok(true),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                Ok(false)
            }
            Err(error) => Err(io(&root.join(relative), error)),
        };
    }
    let mut paths = vec![root.to_path_buf()];
    for component in relative.components() {
        let mut next = Vec::new();
        for path in paths {
            let items = match std::fs::read_dir(&path) {
                Ok(items) => items,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                    ) =>
                {
                    continue;
                }
                Err(error) => return Err(io(&path, error)),
            };
            for item in items {
                let item = item.map_err(|error| io(&path, error))?;
                if item
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&component.as_os_str().to_string_lossy())
                {
                    next.push(item.path());
                }
            }
        }
        paths = next;
    }
    Ok(!paths.is_empty())
}
impl BaseGuard {
    pub(super) fn tripwire(&self, copy: &HermeticCopy) -> Result<(), Unrun> {
        let refuse = |key: &str, state: &str| {
            Unrun::refused(UnrunCause::SnapshotTampered, format!("{key} ({state})"))
        };
        if output(&copy.repository, &["rev-parse", "HEAD"])?.trim() != self.snapshot.commit {
            return Err(refuse("commit", "copy revision differs"));
        }
        if !output(&copy.repository, &["for-each-ref"])?
            .trim()
            .is_empty()
        {
            return Err(refuse("refs", "copy exposes references"));
        }
        let alternate = output(
            &copy.repository,
            &["rev-parse", "--git-path", "objects/info/alternates"],
        )?;
        if copy
            .repository
            .join(alternate.trim())
            .symlink_metadata()
            .is_ok()
        {
            return Err(refuse("objects", "copy exposes alternates"));
        }
        if let Some(head) = &self.live_head
            && head != &self.snapshot.commit
            && !object_absent(&copy.repository, head)?
        {
            return Err(refuse(head, "copy exposes later commit"));
        }
        for (key, entry) in &self.snapshot.entries {
            if skip_copy_entry(key, entry) {
                continue;
            }
            let relative = key_path(key);
            match entry {
                SnapshotEntry::Tracked { object, .. } => {
                    let listed = output(&copy.repository, &["ls-tree", "-z", "HEAD", "--", key])?;
                    let actual = listed
                        .split_once('\t')
                        .and_then(|(h, _)| h.split_whitespace().nth(2));
                    if actual != Some(object)
                        || copy.repository.join(&relative).symlink_metadata().is_err()
                    {
                        return Err(refuse(key, "tracked"));
                    }
                }
                _ => {
                    let project_relative = self
                        .live_project
                        .strip_prefix(&self.live_repository)
                        .ok()
                        .filter(|prefix| starts_with(&relative, prefix, self.ignore_case))
                        .map(|prefix| {
                            relative
                                .components()
                                .skip(prefix.components().count())
                                .fold(PathBuf::new(), |mut path, component| {
                                    path.push(component.as_os_str());
                                    path
                                })
                        })
                        .filter(|p| !p.as_os_str().is_empty());
                    let mut present = false;
                    for root in [&copy.repository, &copy.project] {
                        present |= present_at(root, &relative, self.ignore_case)?;
                    }
                    if let Some(relative) = project_relative {
                        present |= present_at(&copy.project, &relative, self.ignore_case)?;
                    }
                    if present {
                        return Err(refuse(
                            key,
                            if matches!(entry, SnapshotEntry::Absent) {
                                "absent"
                            } else {
                                "unverified"
                            },
                        ));
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn before_layout(&self, contract: &AcceptanceContract) -> Result<(), Unrun> {
        use archon_workflow::task_set_contract::{AcceptanceCheck, TrustedCwd};
        for entry in contract.acceptance.iter().chain(&contract.supplementary) {
            if let AcceptanceCheck::Command { command, cwd } = &entry.check {
                let root = if *cwd == TrustedCwd::RepoRoot {
                    &self.live_repository
                } else {
                    &self.live_project
                };
                for sibling in archon_workflow::acceptance_sibling_paths::discover(
                    root,
                    std::slice::from_ref(command),
                ) {
                    for (key, state) in &self.snapshot.entries {
                        if matches!(state, SnapshotEntry::Tracked { .. })
                            || skip_copy_entry(key, state)
                        {
                            continue;
                        }
                        for root in [&self.live_repository, &self.live_project] {
                            let path = root.join(key_path(key));
                            if starts_with(&sibling.source, &path, self.ignore_case)
                                || starts_with(&path, &sibling.source, self.ignore_case)
                            {
                                return Err(Unrun::unknown(format!(
                                    "sibling source {} overlaps {key} (unverified or absent)",
                                    sibling.source.display()
                                )));
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "workflow_acceptance_executability_base_copy_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "workflow_acceptance_executability_base_copy_filter_tests.rs"]
mod filter_tests;
