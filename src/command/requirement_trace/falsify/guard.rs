//! The restore guard: a mutation that cannot outlive the run that made it.
//!
//! # Every exit path, and the one `Drop` cannot see
//!
//! `Drop` covers returning normally, an early `?`, a failed verifier, a non-zero
//! exit, a timeout, and a panic — the workspace sets no `panic = "abort"`, so a
//! panic unwinds and destructors run. It does not cover Ctrl-C, a `SIGKILL`, or
//! the machine losing power: those end the process without unwinding, and no
//! destructor in any language runs.
//!
//! So the mutation is never the only copy of anything. The original bytes go to
//! a sidecar file *before* the working file is touched, and the sidecar is
//! removed only after the restore has been read back and compared. If the
//! process dies where `Drop` cannot follow, what is left behind is a mutated
//! file, the exact bytes to restore it, and a filename that says what happened —
//! and the next `--falsify` run refuses to start until someone reconciles it
//! (`RefusedToRun::UnreconciledBackup`).
//!
//! That refusal is the point. Auto-restoring a stranded backup would be the
//! friendlier behaviour and the wrong one: from here there is no way to tell a
//! stranded mutation from a human edit made afterwards, and overwriting the
//! second with the first is the data loss this whole module exists to prevent.
//!
//! # Why the restore is verified rather than assumed
//!
//! [`MutationGuard::restore`] writes the original bytes back and then reads them
//! again. A write that silently short-changes (a full disk, a file locked by a
//! compiler the timeout kill did not reach) would otherwise leave a corrupted
//! source file and a clean-looking report. If the comparison fails there is
//! nothing left to do but be loud, so it is loud on stderr even from `Drop`.

use std::io;
use std::path::{Path, PathBuf};

/// The suffix of the crash-safety copy written beside a mutated file.
///
/// Beside the file rather than in a temp directory, so whoever finds a stranded
/// mutation finds its original in the same place. The suffix is deliberately not
/// a source extension: `data_lake.rs.archon-falsify-backup` is invisible to
/// cargo, to `tsc`, and to every other build tool that globs by extension.
pub(super) const BACKUP_SUFFIX: &str = ".archon-falsify-backup";

pub(super) fn backup_path(path: &Path) -> PathBuf {
    let mut raw = path.as_os_str().to_os_string();
    raw.push(BACKUP_SUFFIX);
    PathBuf::from(raw)
}

/// Owns a mutated file and restores it on every path out.
pub(super) struct MutationGuard {
    path: PathBuf,
    backup: PathBuf,
    original: Vec<u8>,
    restored: bool,
}

impl MutationGuard {
    /// Write the sidecar, then the mutant. Ordering is the whole safety story:
    /// the original exists in two places before it exists in one.
    ///
    /// If writing the mutant fails, the guard built a moment earlier is dropped
    /// on the way out and undoes the sidecar, so a failed install leaves nothing
    /// behind either.
    pub(super) fn install(path: &Path, original: Vec<u8>, mutant: &[u8]) -> io::Result<Self> {
        let backup = backup_path(path);
        std::fs::write(&backup, &original)?;
        let mut guard = Self {
            path: path.to_path_buf(),
            backup,
            original,
            restored: false,
        };
        guard.write_mutant(mutant)?;
        Ok(guard)
    }

    fn write_mutant(&mut self, mutant: &[u8]) -> io::Result<()> {
        std::fs::write(&self.path, mutant)
    }

    /// Put the original bytes back and prove it. Idempotent, so calling it
    /// explicitly and letting `Drop` call it again is safe.
    pub(super) fn restore(&mut self) -> io::Result<()> {
        if self.restored {
            return Ok(());
        }
        std::fs::write(&self.path, &self.original)?;
        let readback = std::fs::read(&self.path)?;
        if readback != self.original {
            return Err(io::Error::other(format!(
                "restored {} does not match the original bytes ({} read back, {} expected); \
                 the pre-mutation copy is still at {}",
                self.path.display(),
                readback.len(),
                self.original.len(),
                self.backup.display()
            )));
        }
        self.restored = true;
        // Only now: while this file exists, the next run refuses to start.
        std::fs::remove_file(&self.backup)
    }

    #[cfg(test)]
    pub(super) fn backup_file(&self) -> &Path {
        &self.backup
    }
}

impl Drop for MutationGuard {
    fn drop(&mut self) {
        let Err(err) = self.restore() else {
            return;
        };
        // A panicking `Drop` during an unwind aborts the process, which would
        // strand the mutation this handler exists to report. Shouting on stderr
        // is the loudest thing that is still safe here.
        eprintln!(
            "archon requirements trace --falsify: FAILED TO RESTORE {}: {err}\n\
             The mutation is still in your working tree. The original bytes are at {}. \
             Restore them by hand before doing anything else.",
            self.path.display(),
            self.backup.display()
        );
    }
}
