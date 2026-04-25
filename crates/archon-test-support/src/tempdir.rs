//! `ArchonTempDir` — RAII wrapper around [`tempfile::TempDir`] that
//! lays out the standard `.archon/` skeleton expected by the session
//! and pipeline layers.
//!
//! Tests that touch `.archon/tasks/`, `.archon/pipelines/`, or
//! `.archon/agents/` can drop one of these into scope and forget
//! about cleanup — the directory is removed when the handle is
//! dropped.

use std::path::{Path, PathBuf};

use tempfile::TempDir;

/// Scoped `.archon/` layout for tests.
pub struct ArchonTempDir {
    handle: TempDir,
    archon_dir: PathBuf,
}

impl ArchonTempDir {
    /// Creates a new temp dir with `.archon/{tasks,pipelines,agents}`.
    pub fn new() -> anyhow::Result<Self> {
        let handle = tempfile::tempdir()?;
        let archon_dir = handle.path().join(".archon");
        for sub in ["tasks", "pipelines", "agents"] {
            std::fs::create_dir_all(archon_dir.join(sub))?;
        }
        Ok(Self { handle, archon_dir })
    }

    /// Root of the temp dir (parent of `.archon/`).
    pub fn root(&self) -> &Path {
        self.handle.path()
    }

    /// Path to the `.archon/` dir itself.
    pub fn archon(&self) -> &Path {
        &self.archon_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_standard_subdirs() {
        let td = ArchonTempDir::new().unwrap();
        for sub in ["tasks", "pipelines", "agents"] {
            assert!(td.archon().join(sub).is_dir(), "missing {sub}");
        }
    }
}
