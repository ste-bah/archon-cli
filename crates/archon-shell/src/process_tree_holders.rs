//! Which processes still use a directory: their cwd, an open file, or their
//! executable under it.
//!
//! This is the one check that reaches a descendant no group, session or
//! ancestry names any more (see the parent module). It cannot tell whose
//! process a holder is, so it reports and never kills.

use std::io;
use std::path::{Path, PathBuf};

use super::{canonical_roots, protected, snapshot};

/// A process that holds `path`, which lies under one of the probed roots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    pub pid: u32,
    pub path: PathBuf,
}

/// Every process other than this one and its ancestors that holds a path
/// under any of `roots` (one entry per process: the first path seen). Roots
/// that do not exist are skipped. An error means the probe itself failed,
/// which a caller must treat as "not proven free".
pub fn holders(roots: &[&Path]) -> io::Result<Vec<Holder>> {
    let roots = canonical_roots(roots);
    if roots.is_empty() {
        return Ok(Vec::new());
    }
    let protected = protected(&snapshot()?);
    let under = |path: &Path| roots.iter().any(|root| path.starts_with(root));
    let mut found = Vec::new();
    for (pid, path) in open_paths()? {
        if protected.contains(&pid) || !under(&path) {
            continue;
        }
        if found.last().is_none_or(|last: &Holder| last.pid != pid) {
            found.push(Holder { pid, path });
        }
    }
    Ok(found)
}

/// Every (pid, path) this user's processes hold, grouped by pid.
#[cfg(target_os = "linux")]
fn open_paths() -> io::Result<Vec<(u32, PathBuf)>> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir("/proc")? {
        let Ok(entry) = entry else { continue };
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse().ok()) else {
            continue;
        };
        let dir = entry.path();
        // Another user's process, or one that exited: not readable, skip.
        for link in ["cwd", "exe"] {
            if let Ok(target) = std::fs::read_link(dir.join(link)) {
                paths.push((pid, target));
            }
        }
        let Ok(fds) = std::fs::read_dir(dir.join("fd")) else {
            continue;
        };
        for fd in fds.flatten() {
            if let Ok(target) = std::fs::read_link(fd.path()) {
                paths.push((pid, target));
            }
        }
    }
    Ok(paths)
}

/// Every (pid, path) this user's processes hold, from `lsof` field output
/// (`p<pid>` starts a process, `n<path>` names one of its files).
#[cfg(not(target_os = "linux"))]
fn open_paths() -> io::Result<Vec<(u32, PathBuf)>> {
    // SAFETY: getuid cannot fail and touches no memory.
    let uid = unsafe { libc::getuid() };
    let output = std::process::Command::new("lsof")
        .args(["-nP", "-w", "-Fpn", "-u"])
        .arg(uid.to_string())
        .output()?;
    // lsof exits 1 when any process could not be fully listed; the listing
    // is still complete for those it could read. An empty one is a failure:
    // this process at least holds files.
    if output.stdout.is_empty() {
        return Err(io::Error::other(format!(
            "lsof listed no open files: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(parse_lsof_fields(&String::from_utf8_lossy(&output.stdout)))
}

/// `lsof -F pn` output as (pid, path) pairs.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub fn parse_lsof_fields(text: &str) -> Vec<(u32, PathBuf)> {
    let mut pid = None;
    let mut paths = Vec::new();
    for line in text.lines() {
        if let Some(value) = line.strip_prefix('p') {
            pid = value.parse().ok();
        } else if let (Some(value), Some(pid)) = (line.strip_prefix('n'), pid) {
            paths.push((pid, PathBuf::from(value)));
        }
    }
    paths
}
