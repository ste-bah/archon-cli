//! What the review tripwire watches and how it records it: every file under
//! the watched roots (ignored files included), by (type, size, mtime), with
//! the bytes of everything git cannot give back spilled to disk.
//!
//! A root is watched whole, minus the excludes inside it and the host's
//! toolchain directories (`SHARED_TOOLCHAIN_DIRS`, the ones a read-only
//! call may write). An exclude that contains a root does not hide it: a
//! declared root inside a host directory is still watched. A root that does
//! not exist yet is watched too: whatever appears there is new.
//!
//! A path whose type or size changed has changed. One whose mtime alone
//! moved is compared by content (against git's blob or the spilled copy), so
//! a touch is not a change.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The roots to watch, the paths inside them not to, and the checkout git
/// can restore from.
#[derive(Debug, Clone, Default)]
pub(crate) struct WatchSet {
    pub(crate) repo: Option<PathBuf>,
    pub(crate) roots: Vec<PathBuf>,
    pub(crate) excludes: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    File,
    Dir,
    Link(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Meta {
    kind: Kind,
    size: u64,
    mtime: Option<SystemTime>,
}

/// One walk of the watch set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Snapshot {
    head: Option<String>,
    entries: BTreeMap<PathBuf, Meta>,
}

pub(crate) fn git(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = archon_shell::spawn::command("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

fn toolchain_dir(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| archon_workflow::v2::write::SHARED_TOOLCHAIN_DIRS.contains(&name))
}

fn meta_of(path: &Path) -> Option<Meta> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    let kind = if meta.file_type().is_symlink() {
        Kind::Link(std::fs::read_link(path).unwrap_or_default())
    } else if meta.is_dir() {
        Kind::Dir
    } else {
        Kind::File
    };
    Some(Meta {
        size: if kind == Kind::File { meta.len() } else { 0 },
        mtime: (kind == Kind::File).then(|| meta.modified().ok()).flatten(),
        kind,
    })
}

pub(super) fn snapshot(watch: &WatchSet) -> Snapshot {
    let mut entries = BTreeMap::new();
    for root in &watch.roots {
        let inside: Vec<&PathBuf> = (watch.excludes.iter())
            .filter(|exclude| exclude.starts_with(root) && *exclude != root)
            .collect();
        let mut stack = vec![root.clone()];
        while let Some(path) = stack.pop() {
            if inside.iter().any(|exclude| path.starts_with(exclude)) {
                continue;
            }
            let Some(meta) = meta_of(&path) else { continue };
            if meta.kind == Kind::Dir {
                if path != *root && path.file_name().is_some_and(toolchain_dir) {
                    continue;
                }
                if let Ok(children) = std::fs::read_dir(&path) {
                    stack.extend(children.flatten().map(|entry| entry.path()));
                }
            }
            entries.insert(path, meta);
        }
    }
    let head = (watch.repo.as_deref())
        .and_then(|repo| git(repo, &["rev-parse", "HEAD"]))
        .map(|out| String::from_utf8_lossy(&out).trim().to_string());
    Snapshot { head, entries }
}

/// What a change is put back from: git's HEAD for a file the checkout holds
/// clean, else the bytes spilled to `dir` when the tripwire armed.
pub(super) struct Restore {
    repo: Option<PathBuf>,
    spilled: BTreeMap<PathBuf, PathBuf>,
    dir: PathBuf,
}

impl Restore {
    /// Spill every recorded file git cannot restore into `dir`.
    pub(super) fn spill(watch: &WatchSet, base: &Snapshot, dir: &Path) -> Option<Self> {
        std::fs::create_dir_all(dir).ok()?;
        let clean = watch.repo.as_deref().map(clean_tracked).unwrap_or_default();
        let mut spilled = BTreeMap::new();
        for (index, (path, meta)) in base.entries.iter().enumerate() {
            if meta.kind != Kind::File || clean.contains(path) {
                continue;
            }
            let copy = dir.join(index.to_string());
            // A clone where the filesystem can (macOS `std::fs::copy`).
            std::fs::copy(path, &copy).ok()?;
            spilled.insert(path.clone(), copy);
        }
        Some(Self {
            repo: watch.repo.clone(),
            spilled,
            dir: dir.to_path_buf(),
        })
    }

    /// The recorded bytes of `path`.
    fn recorded(&self, path: &Path) -> Option<Vec<u8>> {
        if let Some(copy) = self.spilled.get(path) {
            return std::fs::read(copy).ok();
        }
        let repo = self.repo.as_deref()?;
        let relative = path.strip_prefix(repo).ok()?.to_str()?;
        git(repo, &["show", &format!("HEAD:{relative}")])
    }
}

impl Drop for Restore {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Every tracked file of the checkout that matches HEAD, absolute.
fn clean_tracked(repo: &Path) -> BTreeSet<PathBuf> {
    let split = |out: Vec<u8>| -> Vec<String> {
        out.split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
            .map(|entry| String::from_utf8_lossy(entry).to_string())
            .collect()
    };
    let tracked = git(repo, &["ls-files", "-z"])
        .map(split)
        .unwrap_or_default();
    let dirty: BTreeSet<String> = git(repo, &["status", "--porcelain=v1", "-z"])
        .map(split)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| entry.get(3..).map(str::to_string))
        .collect();
    tracked
        .into_iter()
        .filter(|path| !dirty.contains(path))
        .map(|path| repo.join(path))
        .collect()
}

/// The paths that changed between `base` and `now`; an mtime-only move is
/// a change only when the content differs.
pub(super) fn changed(base: &Snapshot, now: &Snapshot, restore: &Restore) -> Vec<PathBuf> {
    let paths: BTreeSet<&PathBuf> = base.entries.keys().chain(now.entries.keys()).collect();
    paths
        .into_iter()
        .filter(
            |path| match (base.entries.get(*path), now.entries.get(*path)) {
                (Some(was), Some(is)) if was == is => false,
                (Some(was), Some(is))
                    if was.kind == Kind::File && is.kind == Kind::File && was.size == is.size =>
                {
                    restore.recorded(path) != std::fs::read(path).ok()
                }
                _ => true,
            },
        )
        .cloned()
        .collect()
}

/// Put every changed path back as `base` recorded it; one note per path.
pub(super) fn put_back(base: &Snapshot, paths: &[PathBuf], restore: &Restore) -> Vec<String> {
    let mut notes = Vec::new();
    // What is there now and should not be, deepest first.
    for path in paths.iter().rev() {
        let keep_dir = matches!(base.entries.get(path).map(|m| &m.kind), Some(Kind::Dir))
            && std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir());
        if !keep_dir {
            remove(path);
        }
    }
    // What was there, shallowest first.
    for path in paths {
        let done = match base.entries.get(path) {
            None => !path.exists(),
            Some(meta) => match &meta.kind {
                Kind::Dir => std::fs::create_dir_all(path).is_ok(),
                Kind::Link(target) => restore_link(target, path),
                Kind::File => write_back(path, meta, restore),
            },
        };
        notes.push(format!(
            "{} {}",
            path.display(),
            if done { "restored" } else { "NOT restored" }
        ));
    }
    notes
}

fn write_back(path: &Path, meta: &Meta, restore: &Restore) -> bool {
    let Some(bytes) = restore.recorded(path) else {
        return false;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::write(path, bytes).is_err() {
        return false;
    }
    if let (Some(mtime), Ok(file)) = (meta.mtime, std::fs::File::options().write(true).open(path)) {
        let _ = file.set_modified(mtime);
    }
    true
}

/// Recreate a symlink as it was. Windows needs to know whether the link names
/// a directory; a dangling target is restored as a file link.
#[cfg(unix)]
fn restore_link(target: &Path, path: &Path) -> bool {
    std::os::unix::fs::symlink(target, path).is_ok()
}

#[cfg(windows)]
fn restore_link(target: &Path, path: &Path) -> bool {
    let resolved = path
        .parent()
        .map_or_else(|| target.to_path_buf(), |p| p.join(target));
    if resolved.is_dir() {
        std::os::windows::fs::symlink_dir(target, path).is_ok()
    } else {
        std::os::windows::fs::symlink_file(target, path).is_ok()
    }
}

fn remove(path: &Path) {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => {
            let _ = std::fs::remove_dir_all(path);
        }
        Ok(_) => {
            let _ = std::fs::remove_file(path);
        }
        Err(_) => {}
    }
}

/// HEAD moved, as a note (never rewound by the host).
pub(super) fn head_note(base: &Snapshot, now: &Snapshot) -> Option<String> {
    (base.head != now.head).then(|| {
        format!(
            "HEAD moved from {} to {} (not rewound)",
            base.head.as_deref().unwrap_or("none"),
            now.head.as_deref().unwrap_or("none")
        )
    })
}
