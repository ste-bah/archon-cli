//! The working-tree digest the progress stop compares.
//!
//! `git status` names an untracked file but says nothing of its contents, so
//! a session growing a new file looked like a tree standing still, and a
//! regenerated tracked file flipping back beside it looked like a loop. The
//! untracked entries are fingerprinted here too ([`super::untracked`]), each
//! by what it is, so the digest moves whenever any file the session could
//! have written moves, and no single odd entry can stop the digest.

use std::path::Path;
use std::time::Duration;

use tokio::io::AsyncWriteExt;

/// How long one git query may take before the digest gives up.
const GIT_TIMEOUT: Duration = Duration::from_secs(20);

/// FNV-1a over `git status --porcelain`, `git diff HEAD` and a fingerprint
/// of every untracked, unignored entry of `dir`; `None` for THIS round when
/// `dir` is not a repository or git cannot answer in time, which the write
/// count then stands in for. Nothing a round fails on is remembered: the next
/// round asks again.
pub(super) async fn tree_digest(dir: &Path) -> Option<u64> {
    let mut hash = Fnv::default();
    for args in [
        &["status", "--porcelain=v1", "--untracked-files=all"][..],
        &["diff", "HEAD", "--no-ext-diff", "--binary"][..],
    ] {
        hash.feed(&git(dir, args).await?);
    }
    let listed = git(dir, &["ls-files", "-o", "--exclude-standard", "-z"]).await?;
    let root = dir.to_path_buf();
    let fingerprint = tokio::task::spawn_blocking(move || {
        super::untracked::fingerprint(&root, &listed, super::untracked::Caps::default())
    })
    .await
    .ok()?;
    hash.feed(&fingerprint.to_le_bytes());
    Some(hash.0)
}

/// Whether any of `paths` is out of the digest's sight: outside `dir`, or
/// ignored by its repository. A write there moves nothing the digest reads.
pub(super) async fn any_invisible(dir: &Path, paths: &[std::path::PathBuf]) -> bool {
    let real = archon_shell::paths::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let (inside, outside): (Vec<_>, Vec<_>) = paths
        .iter()
        .partition(|path| path.starts_with(dir) || path.starts_with(&real));
    if !outside.is_empty() {
        return true;
    }
    if inside.is_empty() {
        return false;
    }
    let mut stdin = Vec::new();
    for path in inside {
        stdin.extend_from_slice(path.to_string_lossy().as_bytes());
        stdin.push(0);
    }
    // `check-ignore` exits 1 when nothing is ignored, which `git` reads as a
    // failure: only a listed path means a write the digest cannot see.
    git_lenient(dir, &["check-ignore", "--stdin", "-z"], stdin)
        .await
        .is_some_and(|out| !out.is_empty())
}

/// One git query in `dir`, never taking the index lock an agent's own git
/// command may need; its stdout, or `None` on failure or timeout.
async fn git(dir: &Path, args: &[&str]) -> Option<Vec<u8>> {
    run_git(dir, args, None, false).await
}

async fn run_git(
    dir: &Path,
    args: &[&str],
    stdin: Option<Vec<u8>>,
    accept_failure: bool,
) -> Option<Vec<u8>> {
    let mut command = archon_shell::spawn::tokio_command("git");
    command
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(if stdin.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let run = async {
        let mut child = command.spawn().ok()?;
        let pipe = child.stdin.take();
        // Fed while the output is read, so a long answer cannot fill its pipe
        // and stall the writer; the pipe closes when the feed ends.
        let feed = async move {
            if let (Some(bytes), Some(mut pipe)) = (stdin, pipe) {
                let _ = pipe.write_all(&bytes).await;
            }
        };
        let ((), output) = tokio::join!(feed, child.wait_with_output());
        output.ok()
    };
    let output = tokio::time::timeout(GIT_TIMEOUT, run).await.ok()??;
    (output.status.success() || accept_failure).then_some(output.stdout)
}

/// [`git`] for a query whose non-zero exit is an answer, not a failure.
async fn git_lenient(dir: &Path, args: &[&str], stdin: Vec<u8>) -> Option<Vec<u8>> {
    run_git(dir, args, Some(stdin), true).await
}

/// FNV-1a, the fingerprint the bash heartbeat already uses.
pub(super) struct Fnv(pub(super) u64);

impl Default for Fnv {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    pub(super) fn feed(&mut self, bytes: &[u8]) {
        for byte in bytes.iter().chain(b"\0") {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}
