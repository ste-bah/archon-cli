//! Bounded native command process groups; no command text in argv.
use super::*;
use std::process::Stdio;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

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
struct GroupGuard(i32);
impl Drop for GroupGuard {
    fn drop(&mut self) {
        if self.0 > 0 {
            unsafe {
                libc::kill(-self.0, libc::SIGKILL);
            }
        }
    }
}
async fn drain(
    mut pipe: impl AsyncRead + Unpin,
    limit: usize,
    overflow: Arc<AtomicBool>,
) -> std::io::Result<Vec<u8>> {
    let mut retained = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let n = pipe.read(&mut buffer).await?;
        if n == 0 {
            return Ok(retained);
        }
        let keep = n.min(limit.saturating_sub(retained.len()));
        retained.extend_from_slice(&buffer[..keep]);
        if keep < n {
            overflow.store(true, Ordering::SeqCst);
        }
    }
}
fn scratch_size(path: &Path) -> std::io::Result<u64> {
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
pub(super) async fn run(
    roots: &ScratchRoots,
    policy: &ScratchPolicy,
    id: &str,
    command: &crate::acceptance_world::AuthorizedCommand,
    cancel: Arc<AtomicBool>,
) -> WorkflowResult<CheckResult> {
    let cwd = match command.cwd() {
        crate::task_set_contract::TrustedCwd::ProjectRoot => roots.project(),
        crate::task_set_contract::TrustedCwd::RepoRoot => roots.repository(),
    };
    let mut process = tokio::process::Command::new(archon_shell::resolve_posix_shell());
    process
        .arg("-s")
        .current_dir(cwd)
        .env_clear()
        .envs(roots.environment(policy))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    process.process_group(0);
    let mut child = process.spawn().map_err(|e| WorkflowError::io(cwd, e))?;
    let mut group = GroupGuard(
        child
            .id()
            .ok_or_else(|| invalid("scratch child has no process id"))? as i32,
    );
    let overflow = Arc::new(AtomicBool::new(false));
    let stdout = tokio::spawn(drain(
        child.stdout.take().unwrap(),
        policy.output_bytes,
        overflow.clone(),
    ));
    let stderr = tokio::spawn(drain(
        child.stderr.take().unwrap(),
        policy.output_bytes,
        overflow.clone(),
    ));
    let mut stdin = child.stdin.take().unwrap();
    let bytes = command.bytes().to_vec();
    let writer = tokio::spawn(async move {
        stdin.write_all(&bytes).await?;
        stdin.write_all(b"\n").await?;
        stdin.shutdown().await
    });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(policy.timeout_secs);
    let mut error = None;
    let mut quota_walk_count = 0;
    let mut next_quota = tokio::time::Instant::now() + Duration::from_secs(5);
    let status = loop {
        tokio::select! {
                result=child.wait()=>break result.map_err(|e|WorkflowError::io(cwd,e))?,
                _=tokio::time::sleep_until(deadline)=>{error=Some("native acceptance command timed out".into());break terminate(&mut child,group.0).await?;},
                _=tokio::time::sleep(Duration::from_millis(25))=>{
                    if cancel.load(Ordering::SeqCst) {error=Some("observation parent closed or cancellation requested".into());break terminate(&mut child,group.0).await?;}
                    if overflow.load(Ordering::SeqCst) {error=Some("native acceptance output limit exceeded".into());break terminate(&mut child,group.0).await?;}
                }
                _=tokio::time::sleep_until(next_quota)=>{
                    quota_walk_count += 1;
        match scratch_size(roots.root()) {
                        Ok(size) if size>policy.scratch_bytes=>{error=Some("native acceptance scratch limit exceeded".into());break terminate(&mut child,group.0).await?;}
                        Err(e)=>{error=Some(format!("scratch size audit failed: {e}"));break terminate(&mut child,group.0).await?;}
                        _=>{}
                    }
                    next_quota = tokio::time::Instant::now() + Duration::from_secs(5);
                }
            }
    };
    writer.abort();
    // Reap remaining members even if the leader exited successfully.
    unsafe {
        libc::kill(-group.0, libc::SIGKILL);
    }
    let pipes = tokio::time::timeout(Duration::from_secs(3), async {
        let out = stdout
            .await
            .map_err(|e| invalid(e.to_string()))?
            .map_err(|e| WorkflowError::io(cwd, e))?;
        let err = stderr
            .await
            .map_err(|e| invalid(e.to_string()))?
            .map_err(|e| WorkflowError::io(cwd, e))?;
        Ok::<_, WorkflowError>((out, err))
    })
    .await
    .map_err(|_| invalid("scratch child pipes did not close after group teardown"))??;
    // group is disarmed only after ESRCH, never merely because the leader exited.
    for _ in 0..100 {
        if unsafe { libc::kill(-group.0, 0) } == -1
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        {
            group.0 = 0;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    if group.0 != 0 {
        error = Some("scratch process group teardown could not be verified".into());
    }
    if overflow.load(Ordering::SeqCst) {
        error = Some("native acceptance output limit exceeded".into());
    }
    quota_walk_count += 1;
    match scratch_size(roots.root()) {
        Ok(size) if size > policy.scratch_bytes => {
            error = Some("native acceptance scratch limit exceeded".into())
        }
        Err(e) => error = Some(format!("scratch size audit failed: {e}")),
        _ => {}
    }
    Ok(CheckResult {
        acceptance_id: id.into(),
        exit_code: status.code(),
        quota_walk_count,
        stdout: roots.redact(&pipes.0),
        stderr: roots.redact(&pipes.1),
        operational_error: error,
    })
}
async fn terminate(
    child: &mut tokio::process::Child,
    group: i32,
) -> WorkflowResult<std::process::ExitStatus> {
    unsafe {
        libc::kill(-group, libc::SIGKILL);
    }
    tokio::time::timeout(Duration::from_secs(3), child.wait())
        .await
        .map_err(|_| invalid("scratch child reap deadline exceeded"))?
        .map_err(|e| invalid(format!("scratch child reap failed: {e}")))
}
