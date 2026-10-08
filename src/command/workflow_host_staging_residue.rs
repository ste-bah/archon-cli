//! The durable record that a call's staging may hold unsealed child output
//! (#297 round 9).
//!
//! It is written before the call's child can write staging, and removed
//! only once that staging is sealed after every teardown of the call's
//! trees was confirmed. A process that ends first -- a hard kill, an exit
//! whose drain gave up, a teardown thread that never reports -- leaves it.
//! The next resume then removes that staging, through its anchor, before
//! any child runs, but only once no process of the run's recorded groups
//! still runs: a survivor could write it again.
use std::io;
use std::path::{Path, PathBuf};

use super::workflow_host_command_publish::staging_root;
use super::workflow_host_staging_anchor::StagingAnchor;

/// Where the records live, relative to the run directory.
pub(crate) const RESIDUE_DIR: &str = "v2/host-command-staging-residue";

#[derive(serde::Serialize)]
struct ResidueRecord<'a> {
    schema_version: u32,
    call_id: &'a str,
    command_id: &'a str,
    generation: u64,
    host_pid: u32,
    recorded_at: String,
}

/// The residue record of one call's staging.
pub(crate) struct StagingResidue {
    path: PathBuf,
}

impl StagingResidue {
    pub(crate) fn at(run_root: &Path, call_id: &str) -> Self {
        let name = staging_root(run_root, call_id)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self {
            path: run_root.join(RESIDUE_DIR).join(format!("{name}.json")),
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Writes the record durably (a whole file, renamed into place).
    pub(crate) fn record(
        &self,
        call_id: &str,
        command_id: &str,
        generation: u64,
    ) -> io::Result<()> {
        let record = ResidueRecord {
            schema_version: 1,
            call_id,
            command_id,
            generation,
            host_pid: std::process::id(),
            recorded_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let staged = self.path.with_extension("json.tmp");
        std::fs::write(&staged, serde_json::to_vec(&record)?)?;
        std::fs::rename(&staged, &self.path)
    }

    /// The staging is sealed: the record goes.
    pub(crate) fn clear(&self) -> io::Result<()> {
        match std::fs::remove_file(&self.path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }
}

/// The name a record carries is a staging directory name: never a path.
fn staging_name(record: &Path) -> Option<String> {
    let name = record.file_name()?.to_str()?.strip_suffix(".json")?;
    (!name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_')))
    .then(|| name.to_string())
}

/// On resume, before any child runs: removes the staging every record left
/// under `run_dir` names, then the record. Nothing is removed while a
/// recorded group of the run still runs (or cannot be probed); the records
/// stay for a later resume. Returns the staging paths removed.
pub(crate) fn clear_left(run_dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let dir = run_dir.join(RESIDUE_DIR);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(anyhow::anyhow!(
                "cannot list host command staging residue {}: {error}; restore access to it and resume again",
                dir.display()
            ));
        }
    };
    let mut records = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            records.push(path);
        }
    }
    if records.is_empty() {
        return Ok(Vec::new());
    }
    let (running, _) = super::workflow_host_command_groups::left_groups(run_dir)?;
    if !running.is_empty() {
        tracing::warn!(
            records = records.len(),
            "host command staging residue kept: a recorded process group may still write it"
        );
        return Ok(Vec::new());
    }
    let mut cleared = Vec::new();
    for record in records {
        let Some(name) = staging_name(&record) else {
            return Err(anyhow::anyhow!(
                "host command staging residue record {} names no staging directory; remove it by hand and resume again",
                record.display()
            ));
        };
        let staging = run_dir
            .join(super::workflow_host_staging_anchor::STAGING_DIR)
            .join(&name);
        StagingAnchor::clear(run_dir, &name).map_err(|error| {
            anyhow::anyhow!(
                "cannot remove host command staging residue {} ({error}); restore write access to it and resume again",
                staging.display()
            )
        })?;
        match std::fs::remove_file(&record) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => {
                return Err(anyhow::anyhow!(
                    "cannot remove host command staging residue record {}: {error}; restore write access to its directory and resume again",
                    record.display()
                ));
            }
            _ => {}
        }
        tracing::warn!(path = %staging.display(), "removed host command staging a previous executor left unsealed");
        cleared.push(staging);
    }
    Ok(cleared)
}

#[cfg(all(test, unix))]
#[path = "workflow_host_staging_residue_tests.rs"]
mod tests;
