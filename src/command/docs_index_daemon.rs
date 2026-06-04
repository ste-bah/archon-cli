use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::Result;

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
        && process_running(pid)
    {
        anyhow::bail!("docs index daemon already running with pid {pid}");
    }
    fs::create_dir_all(run_dir())?;
    fs::create_dir_all(log_dir())?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())?;
    let child = Command::new(std::env::current_exe()?)
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
    let Some(pid) = read_pid()? else {
        println!("Docs index daemon is not running.");
        return Ok(());
    };
    if !process_running(pid) {
        fs::remove_file(pid_path()).ok();
        println!("Removed stale docs index daemon pid file for pid {pid}.");
        return Ok(());
    }
    terminate_process(pid)?;
    fs::remove_file(pid_path()).ok();
    println!("Stopped docs index daemon pid {pid}.");
    Ok(())
}

fn status() -> Result<()> {
    match read_pid()? {
        Some(pid) if process_running(pid) => {
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
    let path = pid_path();
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(path)?;
    Ok(text.trim().parse::<u32>().ok())
}

fn process_running(pid: u32) -> bool {
    #[cfg(unix)]
    {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        false
    }
}

fn terminate_process(pid: u32) -> Result<()> {
    #[cfg(unix)]
    {
        let status = Command::new("kill")
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
