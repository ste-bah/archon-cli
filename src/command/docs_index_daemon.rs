use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::Result;
use archon_workflow::process_liveness::process_alive;

use crate::cli_args::DocsIndexDaemonAction;

pub(crate) async fn handle_index_daemon(action: DocsIndexDaemonAction) -> Result<()> {
    match action {
        DocsIndexDaemonAction::Start {
            batch_size,
            window_size,
            poll_secs,
        } => start(batch_size, window_size, poll_secs),
        DocsIndexDaemonAction::Stop => stop(),
        DocsIndexDaemonAction::Status => status(),
        DocsIndexDaemonAction::Run {
            batch_size,
            window_size,
            poll_secs,
        } => run(batch_size, window_size, poll_secs).await,
    }
}

fn start(batch_size: usize, window_size: usize, poll_secs: u64) -> Result<()> {
    if let Some(pid) = read_pid()?
        && pid_owner(pid) == PidOwner::Ours
    {
        anyhow::bail!("docs index daemon already running with pid {pid}");
    }
    fs::create_dir_all(run_dir())?;
    fs::create_dir_all(log_dir())?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())?;
    let child = archon_shell::spawn::command(std::env::current_exe()?)
        .args([
            "docs",
            "index-daemon",
            "run",
            "--batch-size",
            &batch_size.to_string(),
            "--window-size",
            &window_size.to_string(),
            "--poll-secs",
            &poll_secs.to_string(),
        ])
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .spawn()?;
    fs::write(pid_path(), child.id().to_string())?;
    println!(
        "Started docs index daemon pid {} (log: {}).",
        child.id(),
        log_path().display()
    );
    Ok(())
}

fn stop() -> Result<()> {
    println!("{}", stop_pid_file(&pid_path())?);
    Ok(())
}

/// Stops the daemon the pid file at `path` names, or removes the file when
/// that pid no longer names the daemon. Returns the line to print.
fn stop_pid_file(path: &Path) -> Result<String> {
    let Some(pid) = read_pid_at(path)? else {
        return Ok("Docs index daemon is not running.".to_string());
    };
    match pid_owner(pid) {
        PidOwner::Gone => {
            fs::remove_file(path).ok();
            Ok(format!(
                "Removed stale docs index daemon pid file for pid {pid}."
            ))
        }
        PidOwner::Foreign => {
            fs::remove_file(path).ok();
            Ok(format!(
                "Removed stale docs index daemon pid file for pid {pid}: that pid now names another user's process, so no signal was sent."
            ))
        }
        PidOwner::Ours => {
            terminate_process(pid)?;
            fs::remove_file(path).ok();
            Ok(format!("Stopped docs index daemon pid {pid}."))
        }
    }
}

fn status() -> Result<()> {
    match read_pid()? {
        Some(pid) if pid_owner(pid) == PidOwner::Ours => {
            println!("Docs index daemon: running pid {pid}");
            println!("Log: {}", log_path().display());
        }
        Some(pid) => {
            println!("Docs index daemon: stale pid {pid}");
            println!("Run `archon docs index-daemon start` to restart it.");
        }
        None => println!("Docs index daemon: stopped"),
    }
    Ok(())
}

async fn run(batch_size: usize, window_size: usize, poll_secs: u64) -> Result<()> {
    fs::create_dir_all(run_dir())?;
    fs::write(pid_path(), std::process::id().to_string())?;
    loop {
        let db = crate::command::docs::open_db()?;
        crate::command::docs_index::handle_index(
            false,
            None,
            batch_size,
            Some(window_size.max(1)),
            db,
        )
        .await?;
        tokio::time::sleep(Duration::from_secs(poll_secs.max(1))).await;
    }
}

fn run_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".archon")
        .join("run")
}

fn log_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".archon")
        .join("logs")
}

fn pid_path() -> PathBuf {
    run_dir().join("docs-index-daemon.pid")
}

fn log_path() -> PathBuf {
    log_dir().join("docs-index-daemon.log")
}

fn read_pid() -> Result<Option<u32>> {
    read_pid_at(&pid_path())
}

fn read_pid_at(path: &Path) -> Result<Option<u32>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(path)?;
    Ok(text.trim().parse::<u32>().ok())
}

/// What a recorded daemon pid names now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PidOwner {
    /// No process has the pid.
    Gone,
    /// A process this user may signal: the daemon this user started.
    Ours,
    /// A live process this user may not signal. The daemon runs as the user
    /// who started it, so the OS gave its pid to another user's process.
    Foreign,
}

/// Issue 342: liveness alone is not enough here. The shared probe counts a
/// process another user owns as running, which is right for a lock, but the
/// daemon is never another user's process, and `stop` must not signal one.
fn pid_owner(pid: u32) -> PidOwner {
    if !process_alive(pid) {
        return PidOwner::Gone;
    }
    signal_access(pid)
}

#[cfg(unix)]
fn signal_access(pid: u32) -> PidOwner {
    // `process_alive` already answered "not running" for a pid that does not
    // fit `pid_t`.
    let Ok(raw) = libc::pid_t::try_from(pid) else {
        return PidOwner::Gone;
    };
    // SAFETY: signal 0 delivers nothing; it only checks whether this process
    // may signal `raw`.
    if unsafe { libc::kill(raw, 0) } == 0 {
        return PidOwner::Ours;
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::EPERM) => PidOwner::Foreign,
        // It exited after the liveness probe.
        Some(libc::ESRCH) => PidOwner::Gone,
        // Any other error proves nothing; `terminate_process` then fails
        // with the real error instead of a guess here.
        _ => PidOwner::Ours,
    }
}

#[cfg(not(unix))]
fn signal_access(_pid: u32) -> PidOwner {
    PidOwner::Ours
}

fn terminate_process(pid: u32) -> Result<()> {
    #[cfg(unix)]
    {
        let status = archon_shell::spawn::command("kill")
            .args(["-TERM", &pid.to_string()])
            .status()?;
        if !status.success() {
            anyhow::bail!("failed to stop pid {pid}");
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        anyhow::bail!("docs index daemon stop is not implemented on this platform")
    }
}

#[cfg(test)]
#[path = "docs_index_daemon_tests.rs"]
mod tests;
