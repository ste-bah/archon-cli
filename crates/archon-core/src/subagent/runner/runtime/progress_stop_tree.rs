//! The working-tree digest the progress stop compares.
//!
//! `git status` names an untracked file but says nothing of its contents, so
//! a session growing a new file looked like a tree standing still, and a
//! regenerated tracked file flipping back beside it looked like a loop. The
//! untracked files' contents are hashed by git itself (`hash-object`), so the
//! digest moves whenever any file the session could have written moves.

use std::path::Path;
use std::time::Duration;

use tokio::io::AsyncWriteExt;

/// How long one git query may take before the digest gives up.
const GIT_TIMEOUT: Duration = Duration::from_secs(20);

/// FNV-1a over `git status --porcelain`, `git diff HEAD` and the object ids
/// of the untracked, unignored files of `dir`; `None` when `dir` is not a
/// repository or git cannot answer in time, which the write count then stands
/// in for.
pub(super) async fn tree_digest(dir: &Path) -> Option<u64> {
    let mut hash = Fnv::default();
    for args in [
        &["status", "--porcelain=v1", "--untracked-files=all"][..],
        &["diff", "HEAD", "--no-ext-diff", "--binary"][..],
    ] {
        hash.feed(&git(dir, args, None).await?);
    }
    let untracked = git(dir, &["ls-files", "-o", "--exclude-standard", "-z"], None).await?;
    // `--stdin-paths` reads one path per line, so a name holding a newline
    // cannot be passed; its presence is still in the status output above.
    let paths: Vec<&[u8]> = untracked
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty() && !path.contains(&b'\n'))
        .collect();
    if !paths.is_empty() {
        let stdin = paths.join(&b'\n');
        hash.feed(&git(dir, &["hash-object", "--stdin-paths"], Some(stdin)).await?);
    }
    Some(hash.0)
}

/// One git query in `dir`, never taking the index lock an agent's own git
/// command may need; its stdout, or `None` on failure or timeout.
async fn git(dir: &Path, args: &[&str], stdin: Option<Vec<u8>>) -> Option<Vec<u8>> {
    let mut command = tokio::process::Command::new("git");
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
    output.status.success().then_some(output.stdout)
}

/// FNV-1a, the fingerprint the bash heartbeat already uses.
struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    fn feed(&mut self, bytes: &[u8]) {
        for byte in bytes.iter().chain(b"\0") {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}
