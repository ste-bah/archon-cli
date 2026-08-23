//! Materialising the repository's gitignored working state into a new worktree.
//!
//! `git worktree add` checks out TRACKED files only. Everything the repository
//! needs that git deliberately does not carry — vendored dependencies,
//! `.env`-style local config, downloaded fixtures, installed toolchains,
//! prebuilt artefacts — is simply absent from a fresh worktree. The branch then
//! lands in a tree that cannot build, and the failure reads as the agent's
//! fault rather than the checkout's.
//!
//! # Discovered, never named
//!
//! Workflows run against whatever repository the PRD points at, in whatever
//! language it happens to be written in, so there is deliberately no list of
//! directory names here. `git ls-files --others --ignored --exclude-standard
//! --directory` asks THIS repository what it actually ignores, and
//! `--directory` collapses a wholly-ignored tree to one entry — so the answer
//! is a handful of top-level paths, not half a million files.
//!
//! # Mechanism, and why it differs by kind
//!
//! The four candidates are symlink, hardlink, byte copy and reflink. Hardlinks
//! are out immediately: they cannot span directories as a unit, and a hardlink
//! to a file an agent then edits in place mutates canonical with no copy-on-
//! write to catch it. The remaining three are chosen by the only thing that
//! actually predicts cost — whether the entry's size is bounded:
//!
//! | Kind | Mechanism | Cost | Private? |
//! |---|---|---|---|
//! | directory | real dir, immediate children symlinked | O(children) | node yes, children **no** |
//! | regular file | `clonefile` (APFS reflink), else byte copy | O(1) / O(size) | yes |
//! | symlink | the same link recreated verbatim | O(1) | yes |
//!
//! A directory is unbounded: this workspace's `target/` runs to ~10GB across
//! hundreds of thousands of files. A byte copy is the invisible disk fire
//! `isolation.rs` built the whole tier ladder to avoid. `clonefile` is
//! copy-on-write for the DATA but still walks the tree for the metadata, so a
//! 500k-file clone costs a metadata storm per branch per wave. Neither cost is
//! acceptable per branch per wave.
//!
//! ## Why a directory is MIRRORED rather than symlinked whole
//!
//! One symlink for the whole tree is cheaper still, and it is wrong. Ignore
//! patterns are overwhelmingly written directory-anchored — `vendor/`,
//! `target/`, `node_modules/` — and a trailing-slash pattern does not match a
//! SYMLINK of that name. Measured against real git: with `vendor/` ignored, a
//! `vendor` symlink in the worktree shows up as `?? vendor` in `git status`,
//! i.e. as agent-authored untracked content, which is precisely the leak the
//! check below exists to prevent.
//!
//! Recreating the directory NODE and symlinking its immediate children keeps
//! the ignore semantics exact (a real `vendor/` matches `vendor/`) while cost
//! stays O(immediate children) — a handful for `target/`, low thousands for
//! `node_modules/` — and never O(files in tree). It also buys a better
//! ownership split for free: a new TOP-LEVEL entry an agent creates in the
//! directory is private to its worktree, because the node itself is real.
//!
//! A regular file is bounded — `MAX_COPY_BYTES` bounds it — so it gets a
//! private copy and the sharing question never arises. On darwin/APFS
//! `clonefile` makes that copy free, which is why it is tried first and why the
//! byte cap only gates the fallback: the cap exists to bound the mechanism that
//! actually costs, not to refuse a file the filesystem can clone for nothing.
//!
//! # What is shared, and why that is safe here
//!
//! **The CHILDREN of every materialised directory are shared** — each is a
//! symlink into canonical, so a write beneath one is visible to every other
//! branch and to canonical. (The directory node itself is per-worktree, so
//! creating a new top-level entry in it is not.) That is safe for this engine,
//! not in general, and for reasons that live outside this file:
//!
//! - [`IsolationTier::Worktree`] REFUSES build and test commands outright
//!   (`archon-tools/src/isolation.rs`), so nothing at that tier writes into a
//!   build directory at all.
//! - [`IsolationTier::WorktreeWithBuilds`] redirects builds to a LEASED
//!   per-agent cache directory beside the worktree (`bash_build_cache.rs`), so
//!   builds do not write into `./target` either. Cloning `target/` per worktree
//!   would duplicate that lease pool and reintroduce exactly the gigabytes it
//!   exists to amortise.
//!
//! What remains genuinely shared is an agent that installs into a dependency
//! directory by hand (`npm install`, `pip install -e .`). That is the same
//! exposure the `Shared` tier has for those paths, it is recorded in the
//! returned report rather than hidden, and the alternative costs the metadata
//! storm above on every branch to protect a case that mostly does not happen.
//!
//! # Why nothing here can leak into a patch
//!
//! Every candidate is re-checked with `git check-ignore` **inside the new
//! worktree** before anything is created, and only confirmed-ignored paths are
//! materialised. The query preserves the exact spelling `git ls-files` emitted,
//! trailing slash included, because `check-ignore` answers about a PATH STRING
//! and `vendor` and `vendor/` get different answers from a `vendor/` pattern.
//!
//! That gate is not belt-and-braces: if `.gitignore` is itself untracked in
//! canonical it does not get checked out, the worktree ignores nothing, and a
//! materialised `target/` would surface as agent-authored untracked content in
//! `capture_patch`. Confirming against the worktree's own
//! git makes the leak impossible by construction rather than by convention, and
//! a path that fails the check is reported as skipped instead of guessed at.
//!
//! [`IsolationTier::Worktree`]: archon_tools::isolation::IsolationTier

use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use super::{IsolationError, run_git};

/// How many ignored top-level entries one worktree will materialise.
///
/// `git ls-files` returns sorted output and the cap takes the first N, so a
/// retried worktree materialises the SAME set rather than a different slice of
/// a repository that grew between attempts.
pub const MAX_ENTRIES: usize = 256;

/// Byte ceiling for a file materialised by plain copy.
///
/// Gates the fallback only. A reflink is copy-on-write, so its cost does not
/// depend on this number and it is not applied there.
pub const MAX_COPY_BYTES: u64 = 32 * 1024 * 1024;

/// How one ignored entry was reproduced in the worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mechanism {
    /// APFS `clonefile`: copy-on-write, private to this worktree, free.
    Reflink,
    /// Byte copy: private to this worktree, bounded by [`MAX_COPY_BYTES`].
    Copy,
    /// The directory node recreated, its immediate children symlinked into
    /// canonical. **Those children are shared with every other worktree.**
    SharedDirectory,
    /// The source is itself a symlink; the same link is recreated verbatim.
    ReplicatedSymlink,
}

/// Why an ignored entry was left out — recorded, never silent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// [`MAX_ENTRIES`] was already reached.
    EntryLimit,
    /// The worktree's own git does not consider this path ignored, so
    /// materialising it would make it look like agent-authored content.
    NotIgnoredInWorktree,
    /// Listed by git, gone by the time it was read.
    SourceVanished,
    /// The worktree already has something at this path.
    AlreadyPresent,
    /// No reflink available and the file is over [`MAX_COPY_BYTES`].
    TooLargeToCopy(u64),
    /// The filesystem refused. Carried verbatim; nothing is retried by another
    /// mechanism, because a link that failed for a real reason is a fact about
    /// the tree, not a prompt to try harder.
    Failed(String),
}

/// What a worktree got, and what it did not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaterializedIgnored {
    pub materialized: Vec<(String, Mechanism)>,
    pub skipped: Vec<(String, SkipReason)>,
}

impl MaterializedIgnored {
    /// The mechanism recorded for `path`, if it was materialised.
    pub fn mechanism(&self, path: &str) -> Option<Mechanism> {
        self.materialized
            .iter()
            .find(|(entry, _)| entry == path)
            .map(|(_, mechanism)| *mechanism)
    }

    /// The reason recorded for `path`, if it was skipped.
    pub fn skip_reason(&self, path: &str) -> Option<&SkipReason> {
        self.skipped
            .iter()
            .find(|(entry, _)| entry == path)
            .map(|(_, reason)| reason)
    }
}

/// Reproduce canonical's gitignored entries inside a freshly created worktree.
///
/// Runs AFTER the baseline commit is sealed, so no ordering accident can put an
/// ignored path into the commit even in a repository whose `.gitignore` is not
/// itself committed.
pub(super) fn materialize_ignored(
    canonical_root: &Path,
    isolated: &Path,
) -> Result<MaterializedIgnored, IsolationError> {
    let mut report = MaterializedIgnored::default();
    let mut candidates = discover_ignored(canonical_root)?;
    if candidates.len() > MAX_ENTRIES {
        for candidate in candidates.split_off(MAX_ENTRIES) {
            report
                .skipped
                .push((candidate.path, SkipReason::EntryLimit));
        }
    }
    if candidates.is_empty() {
        return Ok(report);
    }
    let confirmed = ignored_in_worktree(isolated, &candidates)?;
    for candidate in candidates {
        let Candidate { raw: _, path } = candidate;
        if !confirmed.contains(&path) {
            report
                .skipped
                .push((path, SkipReason::NotIgnoredInWorktree));
            continue;
        }
        match materialize_one(&canonical_root.join(&path), &isolated.join(&path)) {
            Ok(mechanism) => report.materialized.push((path, mechanism)),
            Err(reason) => report.skipped.push((path, reason)),
        }
    }
    Ok(report)
}

/// One ignored entry, in both spellings that matter.
///
/// `raw` is exactly what `git ls-files` emitted (`vendor/`), which is the only
/// form `check-ignore` answers correctly for a directory-anchored pattern.
/// `path` is the same entry without the trailing slash, for joining and
/// reporting.
struct Candidate {
    raw: String,
    path: String,
}

/// Ask THIS repository what it ignores. No names are assumed.
fn discover_ignored(canonical_root: &Path) -> Result<Vec<Candidate>, IsolationError> {
    let listing = run_git(
        &[
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
            "--no-empty-directory",
            "-z",
        ],
        canonical_root,
    )?
    .stdout;
    Ok(listing
        .split(|byte| *byte == 0)
        .filter(|bytes| !bytes.is_empty())
        .map(|bytes| {
            let raw = String::from_utf8_lossy(bytes).into_owned();
            let path = raw.trim_end_matches('/').to_string();
            Candidate { raw, path }
        })
        .filter(|candidate| !candidate.path.is_empty() && !engine_owned(&candidate.path))
        .collect())
}

/// Two directories are excluded by name, and neither is an ecosystem guess.
///
/// `.git` is git's own store, which a worktree already has its own view of.
/// `.archon` is where this engine puts run state — including, per `WritePlan`,
/// the worktrees themselves, so symlinking it into a worktree would point the
/// tree at its own parent.
fn engine_owned(path: &str) -> bool {
    matches!(
        path.split('/').next().unwrap_or_default(),
        ".git" | ".archon"
    )
}

/// Confirm each candidate is ignored **by the worktree's own git**.
///
/// `git check-ignore` exits 1 when nothing matched, which is an answer rather
/// than a failure; any other non-zero status is a real error and is raised.
fn ignored_in_worktree(
    isolated: &Path,
    candidates: &[Candidate],
) -> Result<BTreeSet<String>, IsolationError> {
    let mut stdin_bytes = Vec::new();
    for candidate in candidates {
        stdin_bytes.extend_from_slice(candidate.raw.as_bytes());
        stdin_bytes.push(0);
    }
    let mut child = Command::new("git")
        .current_dir(isolated)
        .args(["check-ignore", "-z", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                IsolationError::GitMissing
            } else {
                IsolationError::Io(err)
            }
        })?;
    child
        .stdin
        .take()
        .ok_or_else(|| IsolationError::ProcessFailed {
            stderr: "git check-ignore stdin unavailable".into(),
        })?
        .write_all(&stdin_bytes)?;
    let output = child.wait_with_output()?;
    if !matches!(output.status.code(), Some(0 | 1)) {
        return Err(IsolationError::ProcessFailed {
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|raw| !raw.is_empty())
        .map(|raw| {
            let path = String::from_utf8_lossy(raw);
            path.trim_end_matches('/').to_string()
        })
        .collect())
}

fn materialize_one(src: &Path, dst: &Path) -> Result<Mechanism, SkipReason> {
    let meta = match std::fs::symlink_metadata(src) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(SkipReason::SourceVanished);
        }
        Err(err) => return Err(SkipReason::Failed(err.to_string())),
    };
    if std::fs::symlink_metadata(dst).is_ok() {
        return Err(SkipReason::AlreadyPresent);
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|err| SkipReason::Failed(err.to_string()))?;
    }
    let kind = meta.file_type();
    if kind.is_symlink() {
        let target = std::fs::read_link(src).map_err(|err| SkipReason::Failed(err.to_string()))?;
        symlink_at(&target, dst, false)?;
        return Ok(Mechanism::ReplicatedSymlink);
    }
    if kind.is_dir() {
        return mirror_directory(src, dst);
    }
    if clone_file(src, dst).is_ok() {
        return Ok(Mechanism::Reflink);
    }
    if meta.len() > MAX_COPY_BYTES {
        return Err(SkipReason::TooLargeToCopy(meta.len()));
    }
    std::fs::copy(src, dst).map_err(|err| SkipReason::Failed(err.to_string()))?;
    Ok(Mechanism::Copy)
}

/// Recreate the directory node and symlink each immediate child.
///
/// One level only. The node has to be real so the repository's own
/// directory-anchored ignore pattern still matches it; below that, symlinks are
/// enough to make the whole subtree readable, and going deeper would start
/// paying the per-file cost this module exists to avoid.
fn mirror_directory(src: &Path, dst: &Path) -> Result<Mechanism, SkipReason> {
    std::fs::create_dir(dst).map_err(|err| SkipReason::Failed(err.to_string()))?;
    let entries = std::fs::read_dir(src).map_err(|err| SkipReason::Failed(err.to_string()))?;
    for entry in entries {
        let entry = entry.map_err(|err| SkipReason::Failed(err.to_string()))?;
        let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        symlink_at(&entry.path(), &dst.join(entry.file_name()), is_dir)?;
    }
    Ok(Mechanism::SharedDirectory)
}

#[cfg(unix)]
fn symlink_at(target: &Path, link: &Path, _dir: bool) -> Result<(), SkipReason> {
    std::os::unix::fs::symlink(target, link).map_err(|err| SkipReason::Failed(err.to_string()))
}

#[cfg(windows)]
fn symlink_at(target: &Path, link: &Path, dir: bool) -> Result<(), SkipReason> {
    let result = if dir {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    };
    result.map_err(|err| SkipReason::Failed(err.to_string()))
}

/// APFS copy-on-write clone. `dst` must not exist, which `materialize_one`
/// has already established.
#[cfg(target_os = "macos")]
fn clone_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let src_c = CString::new(src.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let dst_c = CString::new(dst.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    // SAFETY: both pointers are NUL-terminated C strings owned by this frame
    // and outlive the call; `clonefile` reads them and returns.
    let rc = unsafe { libc::clonefile(src_c.as_ptr(), dst_c.as_ptr(), 0) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// No reflink primitive here; the caller falls through to the bounded copy.
#[cfg(not(target_os = "macos"))]
fn clone_file(_src: &Path, _dst: &Path) -> std::io::Result<()> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

#[cfg(test)]
#[path = "ignored_deps_tests.rs"]
mod tests;
