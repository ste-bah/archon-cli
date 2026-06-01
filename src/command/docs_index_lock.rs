use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Result;

pub(crate) struct DocsIndexLock {
    path: PathBuf,
}

impl DocsIndexLock {
    pub(crate) fn acquire() -> Result<Self> {
        fs::create_dir_all(run_dir())?;
        let path = lock_path();
        if path.exists() && stale_lock(&path) {
            fs::remove_file(&path).ok();
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                anyhow::anyhow!(
                    "docs index writer already appears active at {}: {error}",
                    path.display()
                )
            })?;
        writeln!(file, "pid={}", std::process::id())?;
        Ok(Self { path })
    }
}

impl Drop for DocsIndexLock {
    fn drop(&mut self) {
        fs::remove_file(&self.path).ok();
    }
}

fn run_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".archon")
        .join("run")
}

fn lock_path() -> PathBuf {
    run_dir().join("docs-index.lock")
}

fn stale_lock(path: &PathBuf) -> bool {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| {
            text.trim()
                .strip_prefix("pid=")
                .and_then(|pid| pid.parse().ok())
        })
        .is_some_and(|pid| !process_running(pid))
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
        let _ = pid;
        true
    }
}
