//! The run's landing commits, and the host's revert of a refused one.
//!
//! A landing's code is one host commit on the target branch
//! (`patch_apply::wave_commit`, author `archon-workflow`, subject `archon:
//! wave <n> outputs (run <run>, stage <call>)`). Its revert goes through the
//! same machinery a landing does: under the repository's write lock, the
//! reverse of exactly that commit's diff is checked, applied and committed
//! by explicit path as `archon: revert refused landing (run <run>, stage
//! <call>)`, with the reverted commit named in a `Reverts-landing:` trailer.
//! Nothing else in the working tree is staged or touched. A reverse diff
//! that no longer applies -- a later landing changed those lines -- or
//! uncommitted work on one of its paths is a conflict: nothing is written,
//! and the landing stays for a person to resolve.

use std::path::Path;

use crate::v2::branch_cache::landing::{LANDING_AUTHOR, host_commit_stage};
use crate::write_coordinator::worktree_isolation::{run_git, run_git_with_stdin};

/// Trailer naming the landing commit a revert commit takes back out.
const REVERTS_TRAILER: &str = "Reverts-landing: ";

/// One host commit of the run on HEAD's first-parent chain.
#[derive(Debug, Clone)]
pub(super) struct HostCommit {
    pub(super) sha: String,
    pub(super) stage: String,
    /// Commit time, in nanoseconds since the epoch (to the second).
    pub(super) at: i64,
    /// The host's revert of a refused landing, not a landing.
    pub(super) revert: bool,
    /// For a revert: the landing commit it took back out.
    pub(super) reverts: Option<String>,
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    run_git(args, root)
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .map_err(|error| format!("git {}: {error}", args.join(" ")))
}

/// Every host commit of `run_id` on HEAD's first-parent chain, newest first.
pub(super) fn host_commits(root: &Path, run_id: &str) -> Result<Vec<HostCommit>, String> {
    let log = git(
        root,
        &[
            "log",
            "--first-parent",
            "--format=%x1e%H%x1f%an%x1f%ct%x1f%s%x1f%b",
            "HEAD",
        ],
    )?;
    Ok(log
        .split('\u{1e}')
        .filter_map(|entry| {
            let mut fields = entry.splitn(5, '\u{1f}');
            let (sha, author, time, subject) = (
                fields.next()?,
                fields.next()?,
                fields.next()?,
                fields.next()?,
            );
            let body = fields.next().unwrap_or_default();
            let (stage, revert) = host_commit_stage(author, subject, run_id)?;
            let reverts = revert
                .then(|| {
                    body.lines()
                        .find_map(|line| line.strip_prefix(REVERTS_TRAILER))
                })
                .flatten()
                .map(|sha| sha.trim().to_string());
            Some(HostCommit {
                sha: sha.trim().to_string(),
                stage,
                at: time
                    .trim()
                    .parse::<i64>()
                    .ok()?
                    .saturating_mul(1_000_000_000),
                revert,
                reverts,
            })
        })
        .collect())
}

fn paths_of(root: &Path, parent: &str, sha: &str) -> Result<Vec<String>, String> {
    let listed = git(
        root,
        &["diff", "--name-only", "--no-renames", "-z", parent, sha],
    )?;
    Ok(listed
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect())
}

/// Why `commit` may not be reverted from under a later landing that
/// stands, or `None`: the first of `later` (newer host landings not being
/// reverted) that changed one of the same paths.
pub(super) fn overlapping_later<'a>(
    root: &Path,
    commit: &HostCommit,
    later: impl Iterator<Item = &'a HostCommit>,
) -> Result<Option<String>, String> {
    let parent = format!("{}^1", commit.sha);
    let own: std::collections::BTreeSet<String> =
        paths_of(root, &parent, &commit.sha)?.into_iter().collect();
    for newer in later {
        let theirs = paths_of(root, &format!("{}^1", newer.sha), &newer.sha)?;
        let shared: Vec<&String> = theirs.iter().filter(|path| own.contains(*path)).collect();
        if !shared.is_empty() {
            return Ok(Some(format!(
                "a later landing that stands ({} of {}) also changed {}; reverting under it could break work a verdict accepted",
                newer.sha,
                newer.stage,
                shared
                    .iter()
                    .map(|p| p.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
    }
    Ok(None)
}

/// Revert landing commit `sha` of `stage`, refused by `verdict`: the new
/// commit's sha and the paths it restored, or why it could not be done. The
/// caller holds the repository's write lock.
pub(super) fn revert_commit(
    root: &Path,
    run_id: &str,
    commit: &HostCommit,
    verdict: &str,
) -> Result<(String, Vec<String>), String> {
    let sha = commit.sha.as_str();
    let parent = git(
        root,
        &["rev-parse", "--verify", "--quiet", &format!("{sha}^1")],
    )?;
    let parent = parent.trim();
    if parent.is_empty() {
        return Err(format!("{sha} has no parent to restore"));
    }
    let paths = paths_of(root, parent, sha)?;
    if paths.is_empty() {
        return Ok((String::new(), paths));
    }
    // Paths are literal: a name like `[id].rs` names that file alone.
    let with_paths = |head: &[&str]| -> Vec<String> {
        std::iter::once("--literal-pathspecs".to_string())
            .chain(head.iter().map(|arg| (*arg).to_string()))
            .chain(std::iter::once("--".to_string()))
            .chain(paths.iter().cloned())
            .collect()
    };
    fn args(owned: &[String]) -> Vec<&str> {
        owned.iter().map(String::as_str).collect()
    }
    let status = with_paths(&["status", "--porcelain=v1", "--untracked-files=all"]);
    let dirty = git(root, &args(&status))?;
    if !dirty.trim().is_empty() {
        return Err(format!(
            "uncommitted work on its paths would be swept into the revert: {}",
            dirty.trim()
        ));
    }
    let diff = with_paths(&["diff", "--binary", "--no-renames", sha, parent]);
    let reverse = run_git(&args(&diff), root)
        .map_err(|error| format!("its reverse diff could not be built: {error}"))?
        .stdout;
    if let Err(error) = run_git_with_stdin(
        &["apply", "--check", "--whitespace=nowarn", "-"],
        root,
        &reverse,
    ) {
        // Every path already as the landing found it: nothing of it is left.
        let unchanged = with_paths(&["diff", "--quiet", parent]);
        if run_git(&args(&unchanged), root).is_ok() {
            return Ok((String::new(), paths));
        }
        return Err(format!("its reverse no longer applies: {error}"));
    }
    run_git_with_stdin(&["apply", "--whitespace=nowarn", "-"], root, &reverse)
        .map_err(|error| format!("its reverse did not apply: {error}"))?;
    let subject = format!(
        "archon: revert refused landing (run {run_id}, stage {})",
        commit.stage
    );
    let body = format!("{REVERTS_TRAILER}{sha}\nRefused-by: {verdict}");
    let add = with_paths(&["add", "-A"]);
    let author = format!("user.name={LANDING_AUTHOR}");
    let email = format!("user.email={LANDING_AUTHOR}@local");
    let commit_args = with_paths(&[
        "-c",
        &author,
        "-c",
        &email,
        "commit",
        "--no-gpg-sign",
        "--no-verify",
        "-m",
        &subject,
        "-m",
        &body,
    ]);
    let committed = git(root, &args(&add)).and_then(|_| git(root, &args(&commit_args)));
    if let Err(error) = committed {
        // Put the landing back exactly as it was: the reverse undone, the
        // index returned to HEAD for these paths.
        let _ = run_git_with_stdin(&["apply", "-R", "--whitespace=nowarn", "-"], root, &reverse);
        let _ = git(root, &args(&with_paths(&["reset", "-q"])));
        return Err(format!("the revert could not be committed: {error}"));
    }
    let head = git(root, &["rev-parse", "HEAD"])?;
    Ok((head.trim().to_string(), paths))
}
