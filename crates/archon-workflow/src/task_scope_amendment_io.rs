//! Batch O: the scope-amendment ledger's file operations -- the atomic
//! replace, the decision log and the ledger lock.

use std::path::{Path, PathBuf};

use super::{ScopeAmendmentError, ScopeAmendmentOutcome, error, log_path};

pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ScopeAmendmentError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let temp = parent.join(format!(
        ".scope-amendments.{}.new",
        uuid::Uuid::new_v4().simple()
    ));
    let written = std::fs::create_dir_all(parent)
        .and_then(|()| std::fs::write(&temp, bytes))
        .and_then(|()| std::fs::rename(&temp, path));
    written.map_err(|err| {
        let _ = std::fs::remove_file(&temp);
        error(format!("{} could not be written: {err}", path.display()))
    })
}

pub(super) fn append_log(
    run_root: &Path,
    trigger: &str,
    outcome: &ScopeAmendmentOutcome,
) -> Result<(), ScopeAmendmentError> {
    use std::io::Write;
    let line = serde_json::json!({
        "at": chrono::Utc::now().to_rfc3339(),
        "trigger": trigger,
        "outcome": outcome,
    });
    let path = log_path(run_root);
    std::fs::create_dir_all(path.parent().unwrap_or(run_root))
        .and_then(|()| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
        })
        .and_then(|mut file| writeln!(file, "{line}"))
        .map_err(|err| error(format!("{} could not be appended: {err}", path.display())))
}

/// An exclusive lock beside the ledger, released on drop; one left by a
/// process that died more than ten minutes ago is broken.
pub(super) struct LedgerLock(PathBuf);

impl LedgerLock {
    pub(super) fn acquire(ledger: &Path) -> Result<Self, ScopeAmendmentError> {
        let path = ledger.with_extension("lock");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| error(err.to_string()))?;
        }
        for _ in 0..600 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(Self(path)),
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(&path)
                        .and_then(|meta| meta.modified())
                        .ok()
                        .and_then(|at| at.elapsed().ok())
                        .is_some_and(|age| age.as_secs() > 600);
                    if stale {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(err) => return Err(error(format!("{}: {err}", path.display()))),
            }
        }
        Err(error(format!(
            "{} is held by another amendment",
            path.display()
        )))
    }
}

impl Drop for LedgerLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
