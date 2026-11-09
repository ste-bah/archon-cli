//! Bounds and watched locations for a declared verification process.
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone)]
pub(crate) struct VerifyLimits {
    pub refuse_tokens: Vec<String>,
    pub timeout: Duration,
    pub min_free_disk_mib: u64,
    pub free_space: fn(&Path) -> io::Result<u64>,
    /// Allow the saved default action after cleanup; in-process callers disable this.
    pub reraise_default: bool,
}

impl From<&archon_core::config::VerifyConfig> for VerifyLimits {
    fn from(config: &archon_core::config::VerifyConfig) -> Self {
        Self {
            refuse_tokens: config.refuse_tokens.clone(),
            timeout: Duration::from_secs(config.timeout_secs),
            min_free_disk_mib: config.min_free_disk_mib,
            free_space: |path| fs2::available_space(path),
            reraise_default: false,
        }
    }
}

impl Default for VerifyLimits {
    fn default() -> Self {
        Self::from(&archon_core::config::VerifyConfig::default())
    }
}

#[derive(Debug, Clone)]
pub(super) struct DiskFloor {
    pub path: String,
    pub free_mib: u64,
    pub floor_mib: u64,
}

/// Watch the working tree and process temporary directory, never lookup or
/// read-only directories inferred from arbitrary environment values.
pub(super) fn watched(cwd: &Path) -> Vec<PathBuf> {
    let mut paths = vec![cwd.to_path_buf()];
    let temporary = std::env::temp_dir();
    if !paths.contains(&temporary) {
        paths.push(temporary);
    }
    paths
}

pub(super) fn disk_floor(
    paths: &[PathBuf],
    limits: &VerifyLimits,
) -> io::Result<Option<DiskFloor>> {
    for path in paths {
        let free_mib = (limits.free_space)(path).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("free-space check for {}: {error}", path.display()),
            )
        })? / (1024 * 1024);
        if free_mib < limits.min_free_disk_mib {
            return Ok(Some(DiskFloor {
                path: path.display().to_string(),
                free_mib,
                floor_mib: limits.min_free_disk_mib,
            }));
        }
    }
    Ok(None)
}

impl DiskFloor {
    pub(super) fn refusal(self) -> archon_knowledge::traceability::RefusedToRun {
        archon_knowledge::traceability::RefusedToRun::DiskFloor {
            path: self.path,
            free_mib: self.free_mib,
            floor_mib: self.floor_mib,
        }
    }

    pub(super) fn inconclusive(self) -> archon_knowledge::traceability::Inconclusive {
        archon_knowledge::traceability::Inconclusive::DiskFloor {
            path: self.path,
            free_mib: self.free_mib,
            floor_mib: self.floor_mib,
        }
    }
}

#[cfg(test)]
impl VerifyLimits {
    pub(super) fn for_test() -> Self {
        Self {
            free_space: |_| Ok(u64::MAX),
            reraise_default: true,
            ..Self::default()
        }
    }
}
