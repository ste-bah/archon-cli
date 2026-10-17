use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, anyhow};

static RESTORE_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) enum RestoreTempFailure {
    Write,
    Sync,
}

#[cfg(test)]
thread_local! {
    pub(crate) static RESTORE_TEMP_FAILURE: std::cell::RefCell<Option<RestoreTempFailure>> =
        const { std::cell::RefCell::new(None) };
}

pub(crate) struct RestoreTempCleanup(Option<PathBuf>);

impl RestoreTempCleanup {
    pub(crate) fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for RestoreTempCleanup {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub(super) fn next_sequence() -> u64 {
    RESTORE_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
}

pub(crate) fn prepare_restore_temp(
    path: &Path,
    restored: &[u8],
) -> Result<(PathBuf, RestoreTempCleanup)> {
    use std::io::Write;

    let task_dir = path
        .parent()
        .ok_or_else(|| anyhow!("task body path has no parent: {}", path.display()))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("task body path has no file name: {}", path.display()))?;
    loop {
        let sequence = next_sequence();
        let candidate = task_dir.join(format!(
            ".{file_name}.restore-{}-{sequence}.tmp",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(mut file) => {
                let cleanup = RestoreTempCleanup(Some(candidate.clone()));
                #[cfg(test)]
                let injected_failure = RESTORE_TEMP_FAILURE.with(|failure| failure.take());
                #[cfg(test)]
                if matches!(injected_failure, Some(RestoreTempFailure::Write)) {
                    file.write_all(&restored[..restored.len() / 2])?;
                    return Err(std::io::Error::other("injected restore temp write failure"))
                        .with_context(|| {
                            format!("writing prepared task body {}", candidate.display())
                        });
                }
                file.write_all(restored).with_context(|| {
                    format!("writing prepared task body {}", candidate.display())
                })?;
                #[cfg(test)]
                if matches!(injected_failure, Some(RestoreTempFailure::Sync)) {
                    return Err(std::io::Error::other("injected restore temp sync failure"))
                        .with_context(|| {
                            format!("syncing prepared task body {}", candidate.display())
                        });
                }
                file.sync_all().with_context(|| {
                    format!("syncing prepared task body {}", candidate.display())
                })?;
                return Ok((candidate, cleanup));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("creating prepared task body {}", candidate.display())
                });
            }
        }
    }
}
