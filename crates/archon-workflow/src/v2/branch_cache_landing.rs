//! Whether a recorded remediation landing still stands on the tree as the
//! run left it (Issue-108).
//!
//! # The rule it replaces
//!
//! A replayed remediation write stood only when every path its apply
//! manifest recorded still had exactly that post-state (e7e19bd36). That
//! reads the run's OWN later landings as tampering: round 2 of a unit
//! rewrites the file round 1 landed, so on a resume round 1 no longer
//! "holds", is dispatched afresh on top of round 2's tree, and everything
//! after it follows a different path than the uninterrupted run did.
//!
//! # The rule now: order from git, never from content
//!
//! A resume must equal the uninterrupted run. Every landing the host applies
//! is committed by the host itself (`patch_apply::wave_commit`: author
//! `archon-workflow`, message `archon: wave <n> outputs (run <run>, stage
//! <call>)`), so the run's landings are an ORDERED sequence of commits on
//! HEAD's first-parent chain. The host records no commit sha on a manifest,
//! so a manifest is matched to its commit by its own stage id and proven by
//! content: the newest run commit of that stage whose blobs for every path
//! the manifest wrote are the manifest's post-states (a deletion: absent).
//!
//! Landing R stands only when:
//!
//! - R wrote something tracked and such a commit of R's exists on HEAD's
//!   first-parent chain (a manifest that wrote nothing needs none);
//! - every path R recorded (post-hashes and deletions alike) is, in the
//!   working tree, the blob the LAST run commit touching that path left --
//!   R's own, or a later landing of this run; a path no run commit touched
//!   must still be what R recorded;
//! - a gitignored path, which no commit carries, still holds exactly what R
//!   recorded, when R wrote it (an ignored path R only hashed is none of its
//!   landing): no order is invented for it -- unless R materialized it as a
//!   project artifact, which is judged where it is verified, in the order of
//!   the run's own copies (`branch_cache_materialized`, Issue-113).
//!
//! A path changed outside the run's landings -- an edit, an operator
//! commit, a reset past R -- matches no such blob and refuses. Anything git
//! cannot answer refuses. Agent data is never read.

use std::collections::BTreeSet;
use std::path::Path;

use crate::write_coordinator::worktree_isolation::run_git;
use crate::write_coordinator::{ManifestStatus, PatchManifest};

/// The author the host commits every landing as.
pub(crate) const LANDING_AUTHOR: &str = "archon-workflow";

/// Why the landing of `manifest` does not stand on `repository_root`, or
/// `Ok` when every path it recorded holds what the run's last landing there
/// left.
pub fn landing_holds(repository_root: &Path, manifest: &PatchManifest) -> Result<(), String> {
    let run = run_commits(repository_root, &manifest.run_id)?;
    let ignored = |path: &str| ignored(repository_root, path);
    let writes = written(manifest);
    let mut tracked_writes = Vec::new();
    for path in &writes {
        if !ignored(path)? {
            tracked_writes.push(path.clone());
        }
    }
    if manifest.status == ManifestStatus::Applied && !tracked_writes.is_empty() {
        own_commit(repository_root, &run, manifest, &tracked_writes)?;
    }
    for (path, landed) in recorded_states(manifest) {
        // A materialized project artifact is judged where it is verified, in
        // the run's own copy order (`materialized`, Issue-113), not here.
        if manifest.materialized.contains_key(&path) && ignored(&path)? {
            continue;
        }
        // An ignored path R did not write -- hashed only as part of its
        // declared scope -- is nothing R landed: a later command of the run
        // regenerating that project artifact is no change to R's answer.
        if !writes.contains(&path) && ignored(&path)? {
            continue;
        }
        let current = current_state(&repository_root.join(&path));
        let expected = if ignored(&path)? {
            landed
        } else {
            match last_run_commit(repository_root, &run, &path)? {
                Some(commit) => blob_state(repository_root, &commit, &path)?,
                None => landed,
            }
        };
        if !same(&expected, &current) {
            return Err(format!(
                "{path} is {current}, but the run's last landing there left {expected}"
            ));
        }
    }
    Ok(())
}

/// The run's landing commits on HEAD's first-parent chain, newest first, as
/// (sha, stage id).
fn run_commits(root: &Path, run_id: &str) -> Result<Vec<(String, String)>, String> {
    let log = git(
        root,
        &["log", "--first-parent", "--format=%H%x1f%an%x1f%s", "HEAD"],
    )?;
    let marker = format!("(run {run_id}, stage ");
    Ok(log
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\u{1f}');
            let (sha, author, subject) = (fields.next()?, fields.next()?, fields.next()?);
            if author != LANDING_AUTHOR || !subject.starts_with("archon: wave ") {
                return None;
            }
            let stage = subject.split_once(&marker)?.1.strip_suffix(')')?;
            Some((sha.to_string(), stage.to_string()))
        })
        .collect())
}

/// The newest run commit of `manifest`'s stage that carries every tracked
/// path it wrote with the post-state it recorded.
fn own_commit(
    root: &Path,
    run: &[(String, String)],
    manifest: &PatchManifest,
    writes: &[String],
) -> Result<String, String> {
    for (sha, _) in run.iter().filter(|(_, stage)| *stage == manifest.stage_id) {
        let mut carries = true;
        for path in writes {
            let Some(post) = post_state(manifest, path) else {
                return Err(format!("{path}: the manifest recorded no post-state"));
            };
            if !same(&blob_state(root, sha, path)?, &post) {
                carries = false;
                break;
            }
        }
        if carries {
            return Ok(sha.clone());
        }
    }
    Err(format!(
        "no landing commit of {}/{} on HEAD's first-parent chain carries what it recorded",
        manifest.stage_id, manifest.item_id
    ))
}

/// The newest run commit on the first-parent chain that touched `path`.
fn last_run_commit(
    root: &Path,
    run: &[(String, String)],
    path: &str,
) -> Result<Option<String>, String> {
    let shas: BTreeSet<&str> = run.iter().map(|(sha, _)| sha.as_str()).collect();
    let log = git(
        root,
        &["log", "--first-parent", "--format=%H", "HEAD", "--", path],
    )?;
    Ok(log
        .lines()
        .find(|sha| shas.contains(sha))
        .map(str::to_string))
}

/// `path`'s state at `commit`: its content hash, or `deleted` when the
/// commit's tree has no such file.
fn blob_state(root: &Path, commit: &str, path: &str) -> Result<String, String> {
    let listing = git(root, &["ls-tree", commit, "--", path])?;
    let Some(object) = listing.lines().find_map(|line| {
        let (meta, name) = line.split_once('\t')?;
        let mut meta = meta.split_whitespace();
        let (kind, object) = (meta.nth(1)?, meta.next()?);
        (name == path && kind == "blob").then(|| object.to_string())
    }) else {
        return Ok("deleted".to_string());
    };
    let bytes = run_git(&["cat-file", "blob", &object], root)
        .map_err(|error| format!("git cat-file {object}: {error}"))?
        .stdout;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn ignored(root: &Path, path: &str) -> Result<bool, String> {
    let status = std::process::Command::new("git")
        .current_dir(root)
        .args(["check-ignore", "-q", "--", path])
        .status()
        .map_err(|error| format!("git check-ignore: {error}"))?;
    match status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(format!("git check-ignore {path} failed")),
    }
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    run_git(args, root)
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .map_err(|error| format!("git {}: {error}", args.join(" ")))
}

/// Every path a manifest recorded and the state it left there.
fn recorded_states(manifest: &PatchManifest) -> Vec<(String, String)> {
    let mut states: Vec<(String, String)> = manifest
        .post_hashes
        .iter()
        .map(|(path, hash)| (path.clone(), hash.clone()))
        .collect();
    for path in &manifest.deleted_files {
        if !manifest.post_hashes.contains_key(path) {
            states.push((path.clone(), "deleted".to_string()));
        }
    }
    states
}

/// The paths a manifest wrote: changed, created or deleted.
fn written(manifest: &PatchManifest) -> Vec<String> {
    let mut paths: Vec<String> = manifest
        .changed_files
        .iter()
        .chain(&manifest.created_files)
        .chain(&manifest.deleted_files)
        .cloned()
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

fn post_state(manifest: &PatchManifest, path: &str) -> Option<String> {
    manifest.post_hashes.get(path).cloned().or_else(|| {
        manifest
            .deleted_files
            .iter()
            .any(|deleted| deleted == path)
            .then(|| "deleted".to_string())
    })
}

/// Two recorded states agree: a pre-image's `absent` and a post-image's
/// `deleted` both mean no file.
fn same(left: &str, right: &str) -> bool {
    let norm = |state: &str| if state == "absent" { "deleted" } else { state }.to_string();
    norm(left) == norm(right)
}

/// A path's state as a manifest records it: its content hash, or `deleted`
/// only when nothing is there. A directory or an unreadable file matches no
/// recorded state.
pub(super) fn current_state(path: &Path) -> String {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "deleted".to_string(),
        Err(_) => "<unreadable>".to_string(),
        Ok(meta) if !meta.is_file() => "<not a file>".to_string(),
        Ok(_) => crate::write_coordinator::patch_apply::hash_file(path)
            .unwrap_or_else(|| "<unreadable>".to_string()),
    }
}

/// One landing commit of a run and the paths it touched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunLanding {
    pub commit: String,
    pub stage: String,
    pub paths: Vec<String>,
}

/// The run's landings after `from` up to and including `to`, oldest first:
/// the host's own commits on `to`'s first-parent chain that `from` does not
/// reach, each with the paths it changed, created or deleted (Issue-111).
/// `from` must be an ancestor of `to`; anything git cannot answer refuses.
pub fn run_landings_between(
    repository_root: &Path,
    run_id: &str,
    from: &str,
    to: &str,
) -> Result<Vec<RunLanding>, String> {
    git(repository_root, &["merge-base", "--is-ancestor", from, to])
        .map_err(|error| format!("{from} is not an ancestor of {to}: {error}"))?;
    let range = format!("{from}..{to}");
    let log = git(
        repository_root,
        &[
            "log",
            "--first-parent",
            "--reverse",
            "--no-renames",
            "--name-only",
            "--format=%x1e%H%x1f%an%x1f%s",
            &range,
        ],
    )?;
    let marker = format!("(run {run_id}, stage ");
    Ok(log
        .split('\u{1e}')
        .filter_map(|entry| {
            let mut lines = entry.lines();
            let mut fields = lines.next()?.split('\u{1f}');
            let (sha, author, subject) = (fields.next()?, fields.next()?, fields.next()?);
            if author != LANDING_AUTHOR || !subject.starts_with("archon: wave ") {
                return None;
            }
            let stage = subject.split_once(&marker)?.1.strip_suffix(')')?;
            let paths = lines
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect();
            Some(RunLanding {
                commit: sha.to_string(),
                stage: stage.to_string(),
                paths,
            })
        })
        .collect())
}

/// The commit `commit`'s first parent: the HEAD a host baseline was sealed
/// on top of.
pub fn first_parent(repository_root: &Path, commit: &str) -> Result<String, String> {
    let parent = format!("{commit}^1");
    let sha = git(
        repository_root,
        &["rev-parse", "--verify", "--quiet", &parent],
    )?;
    let sha = sha.trim();
    if sha.is_empty() {
        return Err(format!("{commit} has no first parent"));
    }
    Ok(sha.to_string())
}

#[cfg(test)]
#[path = "branch_cache_landing_tests.rs"]
mod tests;
