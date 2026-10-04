//! Bounded native command process groups; no command text in argv.
use super::*;
use std::process::Stdio;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

/// The operational error of a check stopped at its site's timeout.
pub const CHECK_TIMED_OUT: &str = "native acceptance command timed out";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckResult {
    pub acceptance_id: String,
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub quota_walk_count: u64,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub operational_error: Option<String>,
}
/// The spawned check: a Unix process-group leader, or, on Windows, a child
/// confined to a Job Object so its descendants can be reaped and teardown
/// verified by waiting on the job (Issue-234).
#[cfg(windows)]
type RunChild = Box<dyn process_wrap::tokio::ChildWrapper>;
#[cfg(not(windows))]
type RunChild = tokio::process::Child;

/// Kills the check's process tree if `run_at` stops before its teardown.
/// `0` once the tree is confirmed empty.
struct GroupGuard {
    pid: i32,
    /// Once the leader is reaped its pid may be reused, so the tree is no
    /// longer reached through it by ancestry.
    #[cfg_attr(windows, allow(dead_code))]
    reaped: bool,
}
impl Drop for GroupGuard {
    fn drop(&mut self) {
        // On Windows the child's Job Object (`process_wrap`'s `KillOnDrop`)
        // reaps the whole job when the child drops; there is no process group
        // to signal here.
        #[cfg(unix)]
        if self.pid > 0 {
            let _ = tree(self.pid, !self.reaped).kill(Duration::from_millis(250));
        }
    }
}

/// Every process the check started (Issue 270): its group, and by ancestry
/// every descendant that moved to a group of its own or left the session.
/// Ancestry from the leader only while it is unreaped.
#[cfg(unix)]
fn tree(leader: i32, leader_unreaped: bool) -> archon_shell::process_tree::Scope {
    let leader = u32::try_from(leader).unwrap_or(0);
    archon_shell::process_tree::Scope {
        roots: if leader_unreaped {
            vec![leader]
        } else {
            Vec::new()
        },
        groups: vec![leader],
        sessions: Vec::new(),
    }
}

/// Kill the check's tree off the runtime thread; returns the survivors.
#[cfg(unix)]
async fn kill_tree(leader: i32, leader_unreaped: bool) -> WorkflowResult<Vec<u32>> {
    let scope = tree(leader, leader_unreaped);
    tokio::task::spawn_blocking(move || scope.kill(Duration::from_secs(3)))
        .await
        .map_err(|e| invalid(format!("scratch teardown task failed: {e}")))?
        .map_err(|e| invalid(format!("scratch teardown failed: {e}")))
}

/// Spawn the prepared command as a confined child: a new Unix process group,
/// or a Windows Job Object with kill-on-drop. On Linux the leader also reaps
/// its orphans, so a descendant whose parent exits stays in its tree.
#[cfg(not(windows))]
fn spawn_confined(mut process: tokio::process::Command, cwd: &Path) -> WorkflowResult<RunChild> {
    process.process_group(0);
    #[cfg(unix)]
    // SAFETY: prctl is an async-signal-safe syscall.
    unsafe {
        process.pre_exec(archon_shell::process_tree::become_subreaper);
    }
    process.spawn().map_err(|e| WorkflowError::io(cwd, e))
}
#[cfg(windows)]
fn spawn_confined(process: tokio::process::Command, cwd: &Path) -> WorkflowResult<RunChild> {
    use process_wrap::tokio::{CommandWrap, JobObject, KillOnDrop};
    let mut wrap = CommandWrap::from(process);
    wrap.wrap(JobObject);
    wrap.wrap(KillOnDrop);
    wrap.spawn().map_err(|e| WorkflowError::io(cwd, e))
}

/// Take the child's three pipes, however it was spawned.
#[cfg(not(windows))]
fn take_pipes(
    child: &mut RunChild,
) -> (
    tokio::process::ChildStdout,
    tokio::process::ChildStderr,
    tokio::process::ChildStdin,
) {
    (
        child.stdout.take().unwrap(),
        child.stderr.take().unwrap(),
        child.stdin.take().unwrap(),
    )
}
#[cfg(windows)]
fn take_pipes(
    child: &mut RunChild,
) -> (
    tokio::process::ChildStdout,
    tokio::process::ChildStderr,
    tokio::process::ChildStdin,
) {
    (
        child.stdout().take().unwrap(),
        child.stderr().take().unwrap(),
        child.stdin().take().unwrap(),
    )
}
async fn drain(
    mut pipe: impl AsyncRead + Unpin,
    limit: usize,
    overflow: Arc<AtomicBool>,
) -> std::io::Result<(Vec<u8>, bool)> {
    let mut retained = Vec::new();
    let mut truncated = false;
    let mut buffer = [0; 8192];
    loop {
        let n = pipe.read(&mut buffer).await?;
        if n == 0 {
            return Ok((retained, truncated));
        }
        let keep = n.min(limit.saturating_sub(retained.len()));
        retained.extend_from_slice(&buffer[..keep]);
        if keep < n {
            truncated = true;
            overflow.store(true, Ordering::SeqCst);
        }
    }
}
pub(super) fn scratch_size(path: &Path) -> std::io::Result<u64> {
    let m = std::fs::symlink_metadata(path)?;
    if m.is_dir() {
        let mut n = 0u64;
        for entry in std::fs::read_dir(path)? {
            let path = entry?.path();
            match scratch_size(&path) {
                Ok(size) => n = n.saturating_add(size),
                // A live command may remove a temp file between listing and stat.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        Ok(n)
    } else if m.is_file() {
        Ok(m.len())
    } else {
        Ok(0)
    }
}
/// Where and under what bounds one authorized command runs.
///
/// The scratch observer builds one from its prepared roots; the authored
/// run's acceptance stage builds one over the live checkout when no
/// `[workflow.acceptance_execution]` policy is configured. Both go through
/// the same `run_at`, so there is one bounded process-group runner.
pub struct CommandSite<'a> {
    pub project: &'a Path,
    pub repository: &'a Path,
    pub environment: BTreeMap<String, String>,
    /// Audited against `scratch_bytes` while a command runs; `None` for a
    /// site in live roots, whose size is not the command's to bound.
    pub audit_root: Option<&'a Path>,
    /// A build cache outside `audit_root` (Batch J2), audited with it.
    pub audit_target: Option<&'a Path>,
    pub scratch_bytes: u64,
    pub output_bytes: usize,
    pub timeout_secs: u64,
    /// Redacts allowlisted host values from captured output; a direct site
    /// forwards the host environment unredacted, as agent shells do.
    pub(super) redactor: Option<&'a ScratchRoots>,
}
impl CommandSite<'_> {
    fn redact(&self, bytes: &[u8], truncated: bool) -> Vec<u8> {
        match self.redactor {
            Some(roots) => roots.redact_output(bytes, truncated),
            None => bytes.to_vec(),
        }
    }
}
impl CommandSite<'_> {
    fn audited_size(&self, root: &Path) -> std::io::Result<u64> {
        let target = match self.audit_target {
            Some(target) if !target.starts_with(root) => scratch_size(target)?,
            _ => 0,
        };
        Ok(scratch_size(root)?.saturating_add(target))
    }
}
pub async fn run_at(
    site: &CommandSite<'_>,
    id: &str,
    command: &crate::acceptance_world::AuthorizedCommand,
    cancel: Arc<AtomicBool>,
) -> WorkflowResult<CheckResult> {
    let cwd = match command.cwd() {
        crate::task_set_contract::TrustedCwd::ProjectRoot => site.project,
        crate::task_set_contract::TrustedCwd::RepoRoot => site.repository,
    };
    let mut process = tokio::process::Command::new(archon_shell::resolve_posix_shell());
    process
        .arg("-s")
        .current_dir(cwd)
        .env_clear()
        .envs(site.environment.clone())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // A scratch slot lists every group it spawns, so a later holder can
    // tell whether one outlived a killed observation (`cache`).
    if let Some(root) = site.audit_root {
        super::cache::record_group(root, None)?;
    }
    let mut child = spawn_confined(process, cwd)?;
    let mut group = GroupGuard {
        pid: child
            .id()
            .ok_or_else(|| invalid("scratch child has no process id"))? as i32,
        reaped: false,
    };
    if let Some(root) = site.audit_root {
        super::cache::record_group(root, Some(group.pid))?;
    }
    let (stdout_pipe, stderr_pipe, stdin_pipe) = take_pipes(&mut child);
    let overflow = Arc::new(AtomicBool::new(false));
    let mut stdout = tokio::spawn(drain(stdout_pipe, site.output_bytes, overflow.clone()));
    let mut stderr = tokio::spawn(drain(stderr_pipe, site.output_bytes, overflow.clone()));
    let mut stdin = stdin_pipe;
    let bytes = command.bytes().to_vec();
    let writer = tokio::spawn(async move {
        stdin.write_all(&bytes).await?;
        stdin.write_all(b"\n").await?;
        stdin.shutdown().await
    });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(site.timeout_secs);
    let mut error = None;
    let mut quota_walk_count = 0;
    // A site with no audit root never walks a quota; the branch below stays
    // dormant by never being scheduled rather than by a flag it could forget.
    let quota_period = if site.audit_root.is_some() {
        Duration::from_secs(5)
    } else {
        Duration::from_secs(365 * 86_400)
    };
    let mut next_quota = tokio::time::Instant::now() + quota_period;
    let status = loop {
        tokio::select! {
            result=child.wait()=>break result.map_err(|e|WorkflowError::io(cwd,e))?,
            _=tokio::time::sleep_until(deadline)=>{error=Some(CHECK_TIMED_OUT.into());break terminate(&mut child,group.pid).await?;},
            _=tokio::time::sleep(Duration::from_millis(25))=>{
                if cancel.load(Ordering::SeqCst) {error=Some("observation parent closed or cancellation requested".into());break terminate(&mut child,group.pid).await?;}
                if overflow.load(Ordering::SeqCst) {error=Some("native acceptance output limit exceeded".into());break terminate(&mut child,group.pid).await?;}
            }
            _=tokio::time::sleep_until(next_quota)=>{
                quota_walk_count += 1;
                if let Some(root) = site.audit_root {
                    match site.audited_size(root) {
                        Ok(size) if size>site.scratch_bytes=>{error=Some("native acceptance scratch limit exceeded".into());break terminate(&mut child,group.pid).await?;}
                        Err(e)=>{error=Some(format!("scratch size audit failed: {e}"));break terminate(&mut child,group.pid).await?;}
                        _=>{}
                    }
                }
                next_quota = tokio::time::Instant::now() + quota_period;
            }
        }
    };
    writer.abort();
    group.reaped = true;
    // Reap remaining members even if the leader exited successfully: the
    // whole tree, not only the members still in the leader's group.
    #[cfg(unix)]
    let survivors = kill_tree(group.pid, false).await?;
    // Windows: terminate the Job Object and wait on it, so every process the
    // check started is reaped before its output is read (Issue-234).
    #[cfg(windows)]
    let survivors: Vec<u32> = {
        let _ = child.start_kill();
        let _ = child.wait().await;
        Vec::new()
    };
    // A descendant outside the tree that kept the check's stdout or stderr
    // open is the evidence of an escape, not a reason to fail the runner.
    let pipes = match tokio::time::timeout(Duration::from_secs(3), async {
        let out = (&mut stdout)
            .await
            .map_err(|e| invalid(e.to_string()))?
            .map_err(|e| WorkflowError::io(cwd, e))?;
        let err = (&mut stderr)
            .await
            .map_err(|e| invalid(e.to_string()))?
            .map_err(|e| WorkflowError::io(cwd, e))?;
        Ok::<_, WorkflowError>((out, err))
    })
    .await
    {
        Ok(pipes) => pipes?,
        Err(_) => {
            stdout.abort();
            stderr.abort();
            error = Some(
                "scratch child output pipes stayed open after teardown: a process outside the check's process tree still holds them".into(),
            );
            ((Vec::new(), false), (Vec::new(), false))
        }
    };
    // The tree is disarmed only once no member is left, never merely because
    // the leader exited.
    if survivors.is_empty() {
        group.pid = 0;
    } else {
        error = Some(format!(
            "scratch process tree teardown could not be verified (still alive: {})",
            pids(&survivors)
        ));
    }
    // A scratch is private to the observation: whatever still holds it after
    // the tree is gone is a descendant no group or ancestry names any more.
    if let Some(root) = site.audit_root
        && let Some(escape) = detached_holders(root, site.audit_target).await
    {
        error = Some(escape);
    }
    if overflow.load(Ordering::SeqCst) {
        error = Some("native acceptance output limit exceeded".into());
    }
    if let Some(root) = site.audit_root {
        quota_walk_count += 1;
        match site.audited_size(root) {
            Ok(size) if size > site.scratch_bytes => {
                error = Some("native acceptance scratch limit exceeded".into())
            }
            Err(e) => error = Some(format!("scratch size audit failed: {e}")),
            _ => {}
        }
    }
    Ok(CheckResult {
        acceptance_id: id.into(),
        exit_code: status.code(),
        quota_walk_count,
        stdout: site.redact(&pipes.0.0, pipes.0.1),
        stderr: site.redact(&pipes.1.0, pipes.1.1),
        operational_error: error,
    })
}
async fn terminate(child: &mut RunChild, group: i32) -> WorkflowResult<std::process::ExitStatus> {
    // While the leader is unreaped, so its descendants are reached through it.
    #[cfg(unix)]
    kill_tree(group, true).await?;
    // Windows: terminate the whole Job Object, not just the leader.
    #[cfg(windows)]
    {
        let _ = group;
        let _ = child.start_kill();
    }
    tokio::time::timeout(Duration::from_secs(3), child.wait())
        .await
        .map_err(|_| invalid("scratch child reap deadline exceeded"))?
        .map_err(|e| invalid(format!("scratch child reap failed: {e}")))
}

fn pids(pids: &[u32]) -> String {
    pids.iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The operational error for processes that still hold the scratch (or the
/// build target) once the check's tree is gone, if any. Polled for a short
/// window: a process the kill just reached releases its files as it exits.
/// A probe that cannot run proves nothing, so it is an error too.
#[cfg(unix)]
async fn detached_holders(root: &Path, target: Option<&Path>) -> Option<String> {
    let roots: Vec<PathBuf> = std::iter::once(root)
        .chain(target)
        .map(Path::to_path_buf)
        .collect();
    let mut last = Vec::new();
    for attempt in 0..HOLDER_PROBES {
        let roots = roots.clone();
        let probed = tokio::task::spawn_blocking(move || {
            let roots: Vec<&Path> = roots.iter().map(PathBuf::as_path).collect();
            archon_shell::process_tree::holders(&roots)
        })
        .await;
        match probed {
            Ok(Ok(holders)) if holders.is_empty() => return None,
            Ok(Ok(holders)) => last = holders,
            Ok(Err(e)) => return Some(format!("scratch holder probe failed: {e}")),
            Err(e) => return Some(format!("scratch holder probe task failed: {e}")),
        }
        if attempt + 1 < HOLDER_PROBES {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    Some(format!(
        "processes outside the check's process tree still use the scratch: {}",
        last.iter()
            .map(|h| format!("pid {} ({})", h.pid, h.path.display()))
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// Windows confines a check to its Job Object, whose wait drains every
/// process in it; there is nothing outside it to find.
#[cfg(not(unix))]
async fn detached_holders(_root: &Path, _target: Option<&Path>) -> Option<String> {
    None
}

#[cfg(unix)]
const HOLDER_PROBES: u32 = 20;

#[cfg(all(test, unix))]
#[path = "acceptance_scratch_process_tests.rs"]
mod tests;
