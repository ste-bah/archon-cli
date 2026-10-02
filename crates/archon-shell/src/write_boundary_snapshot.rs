//! Issue-234: a host-side snapshot-and-restore boundary for hosts where no
//! operating-system write sandbox can be applied (Windows, and any platform
//! without `sandbox-exec` or Landlock).
//!
//! The kernel boundaries (`sandbox-exec` on macOS, Landlock on Linux) refuse a
//! write to a sealed root as it happens. Where neither exists, this is the
//! guarantee that replaces them so a bounded child is never simply refused
//! (Issue-227) and never runs completely unbounded (Issue-213, the write-branch
//! agent that edited the canonical checkout):
//!
//! 1. [`SnapshotBoundary::capture`] records every file under the sealed roots
//!    that is not inside a re-opened writable subtree, before the command runs.
//! 2. the command runs with no OS confinement.
//! 3. [`SnapshotBoundary::verify_restore`] compares afterwards. Any sealed file
//!    the command created, changed or removed is put back and named, and the
//!    caller fails the call — exactly as a kernel `EPERM` would have, only after
//!    the fact rather than at the write.
//!
//! It is weaker than the kernel boundaries in one way, which is documented so no
//! caller mistakes it for them: a command that reads a sealed file it corrupted
//! *within its own run*, before the restore, can act on the corrupted bytes.
//! The restore still guarantees nothing persists and the call is failed, so a
//! verdict taken from a tampered tree never stands — it is re-run on the
//! restored tree. The file tools stay bounded by the guard either way.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Most files one snapshot records; a tree past this is recorded truncated and
/// any later change under it that the snapshot did not reach is reported
/// unverifiable rather than silently allowed.
const MAX_FILES: usize = 200_000;
/// Largest file kept byte-for-byte; a larger one is remembered by length and
/// modified time, enough to detect a change but not to put it back.
const PER_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// Most bytes kept across one snapshot; past it, files are remembered by
/// length and modified time only.
const TOTAL_BYTES: u64 = 512 * 1024 * 1024;

/// One recorded file's pre-command state.
#[derive(Clone, PartialEq, Eq)]
enum State {
    /// Kept byte-for-byte: restorable.
    Bytes(Vec<u8>),
    /// Too large (or the keep budget was spent): length and modified time,
    /// enough to notice a change, not enough to put it back.
    Meta { len: u64, modified: Option<u128> },
    /// A symlink and where it pointed; never followed.
    Link(PathBuf),
}

/// The sealed roots as they stood before one command, and the re-opened
/// writable subtrees that were deliberately left out.
pub struct SnapshotBoundary {
    sealed: Vec<PathBuf>,
    writable: Vec<PathBuf>,
    files: BTreeMap<PathBuf, State>,
    /// The file ceiling was hit: a new or changed file the walk did not reach
    /// cannot be ruled out, so verification says so instead of passing.
    truncated: bool,
}

impl std::fmt::Debug for SnapshotBoundary {
    /// Never prints the kept file bytes, only how many were recorded.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SnapshotBoundary")
            .field("sealed", &self.sealed)
            .field("writable", &self.writable)
            .field("files", &self.files.len())
            .field("truncated", &self.truncated)
            .finish()
    }
}

/// What one command did to the sealed roots despite the boundary.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SnapshotViolation {
    /// Sealed paths that were changed, created or removed.
    pub changed: Vec<PathBuf>,
    /// Changed paths that could not be put back (too large to have been kept,
    /// a directory now in the way, or an I/O error), each with the reason.
    pub unrestored: Vec<(PathBuf, String)>,
    /// The snapshot was truncated, so some change may be unaccounted for.
    pub truncated: bool,
}

impl SnapshotViolation {
    fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.unrestored.is_empty()
    }

    /// One line for a result note, an error, or a log.
    pub fn message(&self, what: &str) -> String {
        let list = |paths: &[PathBuf]| {
            paths
                .iter()
                .take(10)
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut text = format!(
            "{what} wrote {} path(s) under a sealed root that the host restored \
             (the host-side write boundary for a platform with no OS sandbox): {}",
            self.changed.len(),
            list(&self.changed)
        );
        if !self.unrestored.is_empty() {
            text.push_str(&format!(
                ". {} could NOT be put back: {}",
                self.unrestored.len(),
                self.unrestored
                    .iter()
                    .take(10)
                    .map(|(p, why)| format!("{} ({why})", p.display()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if self.truncated {
            text.push_str(
                ". The snapshot was truncated at its file ceiling, so a further change \
                 may be unaccounted for.",
            );
        }
        text
    }
}

fn modified_nanos(meta: &std::fs::Metadata) -> Option<u128> {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
}

/// Whether `path` (canonical) lies in, or is, any re-opened writable subtree
/// (canonical).
fn under_any(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

/// `path` with every link resolved and its one plain spelling
/// ([`crate::paths::plain`]), existing or not: the nearest existing ancestor
/// canonical, the rest appended. The same directory reaches a boundary under
/// several spellings (on Windows a temp directory's 8.3 short name, its long
/// name, and the verbatim `\\?\` form); comparing those as given would
/// treat one directory as two, so a re-opened worktree would not exempt its
/// own writes under a sealed root spelled differently.
fn canonical(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut cursor = path;
    loop {
        if let Ok(base) = cursor.canonicalize().map(crate::paths::plain) {
            return rest.iter().rev().fold(base, |at, part| at.join(part));
        }
        let (Some(parent), Some(name)) = (cursor.parent(), cursor.file_name()) else {
            return path.to_path_buf();
        };
        rest.push(name.to_os_string());
        cursor = parent;
    }
}

/// `paths` canonical, deduplicated, and with any path inside another dropped
/// (its files are recorded by the outer walk).
fn outermost(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut all: Vec<PathBuf> = paths.into_iter().map(|path| canonical(&path)).collect();
    all.sort();
    all.dedup();
    let copy = all.clone();
    all.retain(|path| {
        !copy
            .iter()
            .any(|other| other != path && path.starts_with(other))
    });
    all
}

impl SnapshotBoundary {
    /// Record the sealed roots now, skipping every re-opened writable subtree.
    pub fn capture(sealed: &[PathBuf], writable: &[PathBuf]) -> Self {
        let sealed = outermost(sealed.iter().filter(|p| p.is_absolute()).cloned());
        // A writable directory that contains a sealed root would re-open it: it
        // is dropped, as every boundary drops it. Judged canonically -- a temp
        // directory given as `/var/...` contains a root sealed as
        // `/private/var/...` (on Windows, an 8.3 name its long twin).
        let writable: Vec<PathBuf> = (writable.iter().map(|path| canonical(path)))
            .filter(|dir| !sealed.iter().any(|root| root.starts_with(dir)))
            .collect();
        let mut boundary = Self {
            sealed: sealed.clone(),
            writable,
            files: BTreeMap::new(),
            truncated: false,
        };
        let mut kept = 0u64;
        for root in &sealed {
            boundary.walk(root, &mut kept);
        }
        boundary
    }

    fn record(&mut self, path: &Path, meta: &std::fs::Metadata, kept: &mut u64) {
        let len = meta.len();
        let state = if len <= PER_FILE_BYTES && *kept + len <= TOTAL_BYTES {
            match std::fs::read(path) {
                Ok(bytes) => {
                    *kept += bytes.len() as u64;
                    State::Bytes(bytes)
                }
                Err(_) => State::Meta {
                    len,
                    modified: modified_nanos(meta),
                },
            }
        } else {
            State::Meta {
                len,
                modified: modified_nanos(meta),
            }
        };
        self.files.insert(path.to_path_buf(), state);
    }

    fn walk(&mut self, dir: &Path, kept: &mut u64) {
        if under_any(dir, &self.writable) {
            return;
        }
        let meta = match std::fs::symlink_metadata(dir) {
            Ok(meta) => meta,
            Err(_) => return,
        };
        if meta.file_type().is_symlink() {
            if let Ok(target) = std::fs::read_link(dir) {
                self.files.insert(dir.to_path_buf(), State::Link(target));
            }
            return;
        }
        if meta.is_file() {
            if self.files.len() >= MAX_FILES {
                self.truncated = true;
                return;
            }
            self.record(dir, &meta, kept);
            return;
        }
        if !meta.is_dir() {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut names: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        names.sort();
        for child in names {
            if self.files.len() >= MAX_FILES {
                self.truncated = true;
                return;
            }
            self.walk(&child, kept);
        }
    }

    /// Compare the sealed roots with now. Every sealed file the command
    /// created, changed or removed outside the writable subtrees is restored
    /// where it can be, and the result names what changed. `Ok(())` when
    /// nothing outside the writable subtrees moved.
    pub fn verify_restore(&self) -> Result<(), SnapshotViolation> {
        let mut now: BTreeMap<PathBuf, std::fs::Metadata> = BTreeMap::new();
        for root in &self.sealed {
            collect_now(root, &self.writable, &mut now);
        }
        let mut violation = SnapshotViolation {
            truncated: self.truncated,
            ..Default::default()
        };
        // Files that were there before: changed or removed?
        for (path, before) in &self.files {
            match now.get(path) {
                Some(meta) if !self.differs(path, before, meta) => {}
                _ => {
                    violation.changed.push(path.clone());
                    self.restore(path, before, &mut violation);
                }
            }
        }
        // Files that are there now but were not: the command created them.
        for path in now.keys() {
            if !self.files.contains_key(path) {
                violation.changed.push(path.clone());
                match std::fs::remove_file(path) {
                    Ok(()) => {}
                    Err(error) => violation
                        .unrestored
                        .push((path.clone(), format!("could not remove: {error}"))),
                }
            }
        }
        if violation.is_empty() && !violation.truncated {
            Ok(())
        } else if violation.is_empty() {
            // Truncated but nothing seen to change: the ceiling is a caveat,
            // not a violation.
            Ok(())
        } else {
            violation.changed.sort();
            violation.changed.dedup();
            Err(violation)
        }
    }

    fn differs(&self, path: &Path, before: &State, meta: &std::fs::Metadata) -> bool {
        if meta.file_type().is_symlink() {
            return match before {
                State::Link(target) => std::fs::read_link(path).ok().as_ref() != Some(target),
                _ => true,
            };
        }
        if !meta.is_file() {
            return true;
        }
        match before {
            State::Bytes(bytes) => std::fs::read(path).map(|now| &now != bytes).unwrap_or(true),
            State::Meta { len, modified } => {
                meta.len() != *len || modified_nanos(meta) != *modified
            }
            State::Link(_) => true,
        }
    }

    fn restore(&self, path: &Path, before: &State, violation: &mut SnapshotViolation) {
        let unrestorable = |violation: &mut SnapshotViolation, why: String| {
            violation.unrestored.push((path.to_path_buf(), why));
        };
        if std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir()) {
            unrestorable(violation, "a directory now stands where it was".into());
            return;
        }
        match before {
            State::Bytes(bytes) => {
                if let Some(parent) = path.parent()
                    && let Err(error) = std::fs::create_dir_all(parent)
                {
                    unrestorable(violation, format!("parent could not be made: {error}"));
                    return;
                }
                if let Err(error) = std::fs::write(path, bytes) {
                    unrestorable(violation, format!("could not be written back: {error}"));
                }
            }
            State::Meta { .. } => unrestorable(
                violation,
                "too large to have been kept byte-for-byte".into(),
            ),
            State::Link(_) => unrestorable(violation, "was a symlink".into()),
        }
    }
}

fn collect_now(dir: &Path, writable: &[PathBuf], out: &mut BTreeMap<PathBuf, std::fs::Metadata>) {
    if under_any(dir, writable) || out.len() >= MAX_FILES * 2 {
        return;
    }
    let Ok(meta) = std::fs::symlink_metadata(dir) else {
        return;
    };
    if meta.file_type().is_symlink() || meta.is_file() {
        out.insert(dir.to_path_buf(), meta);
        return;
    }
    if !meta.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for child in entries.flatten() {
        collect_now(&child.path(), writable, out);
    }
}

#[cfg(test)]
#[path = "write_boundary_snapshot_tests.rs"]
mod tests;
