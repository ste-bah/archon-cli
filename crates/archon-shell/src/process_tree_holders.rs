//! Which processes still use a directory: their cwd, an open file, or their
//! executable under it, and whether they hold anything there for writing.
//!
//! This is the one check that reaches a descendant no group, session,
//! ancestry or earlier scan names any more (see the parent module). It
//! cannot tell whose process a holder is, so it reports and never kills, and
//! a caller should attribute a holder only when it is a remembered member or
//! holds something for writing: an unrelated reader is not an escape.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{canonical_roots, protected, snapshot_until};

/// The bound [`holders`] gives a probe.
pub const HOLDER_PROBE_DEADLINE: Duration = Duration::from_secs(5);

/// A process that holds `path`, which lies under one of the probed roots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    pub pid: u32,
    /// Its start time, when the process table still listed it.
    pub start: Option<u64>,
    /// The first path seen under a root, preferring one held for writing.
    pub path: PathBuf,
    /// Whether it holds any file under a root open for writing.
    pub writes: bool,
}

/// [`holders_within`] with [`HOLDER_PROBE_DEADLINE`].
pub fn holders(roots: &[&Path]) -> io::Result<Vec<Holder>> {
    holders_within(roots, HOLDER_PROBE_DEADLINE)
}

/// Every process other than this one and its ancestors that holds a path
/// under any of `roots`. Roots that do not exist are skipped. An error,
/// including `TimedOut` when the probe could not finish within `deadline`,
/// means the probe proved nothing: a caller must treat it as "unknown".
pub fn holders_within(roots: &[&Path], deadline: Duration) -> io::Result<Vec<Holder>> {
    holders_using(roots, deadline, lsof_program)
}

/// [`holders_within`] with the `lsof` lookup given, so a test can name the
/// program the probe spawns without changing PATH. Linux reads `/proc` and
/// never calls `lsof`.
pub(crate) fn holders_using(
    roots: &[&Path],
    deadline: Duration,
    lsof: impl FnOnce() -> io::Result<PathBuf>,
) -> io::Result<Vec<Holder>> {
    let end = Instant::now() + deadline;
    let roots = canonical_roots(roots);
    if roots.is_empty() {
        return Ok(Vec::new());
    }
    let table = snapshot_until(end)?;
    let protected = protected(&table.processes);
    let starts: BTreeMap<u32, u64> = table.processes.iter().map(|p| (p.pid, p.start)).collect();
    let under = |path: &Path| roots.iter().any(|root| path.starts_with(root));
    let mut found: BTreeMap<u32, Holder> = BTreeMap::new();
    for (pid, path, writes) in open_paths(end, lsof)? {
        if protected.contains(&pid) || !under(&path) {
            continue;
        }
        let holder = found.entry(pid).or_insert_with(|| Holder {
            pid,
            start: starts.get(&pid).copied(),
            path: path.clone(),
            writes,
        });
        if writes && !holder.writes {
            holder.writes = true;
            holder.path = path;
        }
    }
    Ok(found.into_values().collect())
}

#[cfg(target_os = "linux")]
fn timed_out() -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        "holder probe did not finish in time",
    )
}

/// A bounded probe's error as the holder probe reports it. A timeout keeps
/// its cause: a slow `lsof` and probe slots that stayed full are different
/// faults for the operator.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub(super) fn probe_failed(error: io::Error) -> io::Error {
    if error.kind() == io::ErrorKind::TimedOut {
        io::Error::new(
            io::ErrorKind::TimedOut,
            format!("holder probe did not finish in time: {error}"),
        )
    } else {
        error
    }
}

/// Every (pid, path, held for writing) this user's processes hold.
#[cfg(target_os = "linux")]
fn open_paths(
    end: Instant,
    _lsof: impl FnOnce() -> io::Result<PathBuf>,
) -> io::Result<Vec<(u32, PathBuf, bool)>> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir("/proc")? {
        if Instant::now() >= end {
            return Err(timed_out());
        }
        let Ok(entry) = entry else { continue };
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse().ok()) else {
            continue;
        };
        let dir = entry.path();
        // Another user's process, or one that exited: not readable, skip.
        for link in ["cwd", "exe"] {
            if let Ok(target) = std::fs::read_link(dir.join(link)) {
                paths.push((pid, target, false));
            }
        }
        let Ok(fds) = std::fs::read_dir(dir.join("fd")) else {
            continue;
        };
        // A process can hold a huge number of descriptors: the deadline is
        // checked inside one process's list too, in batches.
        for (index, fd) in fds.flatten().enumerate() {
            if index % 256 == 255 && Instant::now() >= end {
                return Err(timed_out());
            }
            let Ok(target) = std::fs::read_link(fd.path()) else {
                continue;
            };
            let info = dir.join("fdinfo").join(fd.file_name());
            let writes = std::fs::read_to_string(info)
                .ok()
                .and_then(|text| fdinfo_writes(&text))
                .unwrap_or(false);
            paths.push((pid, target, writes));
        }
    }
    Ok(paths)
}

/// Whether a `/proc/<pid>/fdinfo/<fd>` text says the file is open for
/// writing (`flags`, octal, access mode not `O_RDONLY`).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn fdinfo_writes(text: &str) -> Option<bool> {
    let flags = text
        .lines()
        .find_map(|line| line.strip_prefix("flags:"))?
        .trim();
    let flags = i64::from_str_radix(flags, 8).ok()?;
    Some(flags & i64::from(libc::O_ACCMODE) != i64::from(libc::O_RDONLY))
}

/// Every (pid, path, held for writing) this user's processes hold, from
/// `lsof` field output, read within the deadline.
#[cfg(not(target_os = "linux"))]
fn open_paths(
    end: Instant,
    lsof: impl FnOnce() -> io::Result<PathBuf>,
) -> io::Result<Vec<(u32, PathBuf, bool)>> {
    // No lsof proves nothing: the error reaches the caller as "unknown".
    let program = lsof()?;
    // SAFETY: getuid cannot fail and touches no memory.
    let uid = unsafe { libc::getuid() };
    let mut command = crate::spawn::command(program);
    command
        .args(["-nP", "-w", "-Fpan", "-u"])
        .arg(uid.to_string());
    let left = end.saturating_duration_since(Instant::now());
    let stdout = super::bounded::stdout_within(command, left).map_err(probe_failed)?;
    // lsof exits 1 when any process could not be fully listed; the listing
    // is still complete for those it could read. An empty one is a failure:
    // this process at least holds files.
    if stdout.is_empty() {
        return Err(io::Error::other("lsof listed no open files"));
    }
    Ok(parse_lsof_fields(&String::from_utf8_lossy(&stdout)))
}

/// Where `lsof` is installed, tried in this order before PATH.
pub(crate) const STANDARD_LSOF: &[&str] = &[
    "/usr/sbin/lsof",
    "/usr/bin/lsof",
    "/usr/local/sbin/lsof",
    "/usr/local/bin/lsof",
];

/// The `lsof` every caller spawns. Both callers are teardown safety checks,
/// so a fixed system location wins: a writable directory early on PATH (a
/// toolchain directory, say) could hold a planted `lsof` that hides holders.
/// PATH is the fallback only, for a host whose `lsof` is in none of them; a
/// probe often runs under a confined PATH such as `/usr/bin:/bin`, which the
/// standard locations cover. Linux holder probes read `/proc`, but other
/// callers (a build cache's idle check) spawn `lsof` there too.
///
/// An error means no executable `lsof` was found: a caller must treat its
/// check as "unknown", never as "no holders" or "idle".
pub fn lsof_program() -> io::Result<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    lsof_program_from(
        STANDARD_LSOF,
        std::env::split_paths(&path).map(|dir| dir.join("lsof")),
    )
}

/// The first executable file among the standard locations, else among the
/// PATH entries (absolute ones only: a relative entry resolves against
/// whatever the working directory is), with a warning that names it.
pub(crate) fn lsof_program_from(
    standard: &[&str],
    on_path: impl Iterator<Item = PathBuf>,
) -> io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let executable = |path: &PathBuf| {
        std::fs::metadata(path)
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    };
    if let Some(found) = standard.iter().map(PathBuf::from).find(executable) {
        return Ok(found);
    }
    let mut on_path = on_path.filter(|path| path.is_absolute());
    match on_path.find(executable) {
        Some(found) => {
            if first_path_fallback(&found) {
                tracing::warn!(
                    lsof = %found.display(),
                    standard = ?standard,
                    "no executable lsof at a standard location; using the one PATH names"
                );
            }
            Ok(found)
        }
        None => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no executable lsof at {} or on PATH", standard.join(", ")),
        )),
    }
}

/// Whether this process has not yet used `path` as a PATH fallback, so a
/// probe that runs in a loop warns once per process and path.
pub(crate) fn first_path_fallback(path: &Path) -> bool {
    static WARNED: std::sync::Mutex<std::collections::BTreeSet<PathBuf>> =
        std::sync::Mutex::new(std::collections::BTreeSet::new());
    WARNED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(path.to_path_buf())
}

/// `lsof -F pan` output as (pid, path, held for writing): `p` starts a
/// process, `f` a file, `a` gives its access mode (`r`, `w`, `u`), and `n`
/// names it.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub fn parse_lsof_fields(text: &str) -> Vec<(u32, PathBuf, bool)> {
    let mut pid = None;
    let mut writes = false;
    let mut paths = Vec::new();
    for line in text.lines() {
        if let Some(value) = line.strip_prefix('p') {
            pid = value.parse().ok();
        } else if line.starts_with('f') {
            writes = false;
        } else if let Some(mode) = line.strip_prefix('a') {
            writes = matches!(mode.trim(), "w" | "u");
        } else if let (Some(value), Some(pid)) = (line.strip_prefix('n'), pid) {
            paths.push((pid, PathBuf::from(value), writes));
        }
    }
    paths
}
