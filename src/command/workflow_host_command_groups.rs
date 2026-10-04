//! Durable records of host-command process groups in flight (Issue 251).
//!
//! Each host command runs in its own process group, so a SIGKILL of its
//! parent leaves the group running. While the supervisor owns a group it
//! keeps one record per group under the run; a record left behind names a
//! group whose parent died. A resume must not start new commands while such
//! a group still runs, because both would write the same task root.

use std::path::{Path, PathBuf};

use archon_workflow::{WorkflowError, WorkflowResult};

/// Where the records live, relative to the run directory.
pub(crate) const GROUP_RECORDS_DIR: &str = "v2/host-command-groups";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct HostCommandGroupRecord {
    pub(crate) schema_version: u32,
    pub(crate) pgid: u32,
    pub(crate) pid: u32,
    /// The session the command leads (Issue 270). A nested runner gives its
    /// checks process groups of their own; they stay in this session, so the
    /// record runs while any of them does. Absent in records written before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) session: Option<u32>,
    pub(crate) command_id: String,
    pub(crate) host_pid: u32,
    pub(crate) started_at: String,
}

/// Removes its record when the supervisor is done with the group.
#[derive(Debug)]
pub(crate) struct GroupRecordGuard(PathBuf);

impl Drop for GroupRecordGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Writes the record of `pgid` under `dir` before the supervisor waits on it.
pub(crate) fn record_group(
    dir: &Path,
    pgid: u32,
    pid: u32,
    session: Option<u32>,
    command_id: &str,
) -> WorkflowResult<GroupRecordGuard> {
    let record = HostCommandGroupRecord {
        schema_version: 1,
        pgid,
        pid,
        session,
        command_id: command_id.to_string(),
        host_pid: std::process::id(),
        started_at: chrono::Utc::now().to_rfc3339(),
    };
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| WorkflowError::Io { path, source }
    };
    std::fs::create_dir_all(dir).map_err(io(dir))?;
    let path = dir.join(format!("{pgid}.json"));
    let staged = path.with_extension("json.tmp");
    std::fs::write(&staged, serde_json::to_vec(&record)?).map_err(io(&staged))?;
    std::fs::rename(&staged, &path).map_err(io(&path))?;
    Ok(GroupRecordGuard(path))
}

/// [`record_group`] for a command whose pid is `leader` (the supervisor makes
/// each command a group leader and, on Unix, a session leader), when the run
/// keeps records and the child has an id.
pub(crate) fn record_in(
    dir: Option<&Path>,
    leader: Option<u32>,
    command_id: &str,
) -> WorkflowResult<Option<GroupRecordGuard>> {
    let session = if cfg!(unix) { leader } else { None };
    match (dir, leader) {
        (Some(dir), Some(pgid)) => record_group(dir, pgid, pgid, session, command_id).map(Some),
        _ => Ok(None),
    }
}

/// Whether any process of group `pgid` exists. `None` where it cannot be
/// probed (not unix), which a caller must treat as possibly running.
pub(crate) fn group_running(pgid: u32) -> Option<bool> {
    #[cfg(unix)]
    {
        let pgid = libc::pid_t::try_from(pgid).ok().filter(|pgid| *pgid > 1)?;
        // SAFETY: signal 0 only probes; EPERM means the group exists.
        let probed = unsafe { libc::kill(-pgid, 0) };
        Some(probed == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM))
    }
    #[cfg(not(unix))]
    {
        let _ = pgid;
        None
    }
}

/// Whether anything `record` names still runs: its group, or any process of
/// its session. `None` where that cannot be probed, which a caller must
/// treat as possibly running; a failed session probe counts as running.
pub(crate) fn record_running(record: &HostCommandGroupRecord) -> Option<bool> {
    let group = group_running(record.pgid)?;
    #[cfg(unix)]
    if let (false, Some(session)) = (group, record.session) {
        let scope = archon_shell::process_tree::Scope {
            sessions: vec![session],
            ..Default::default()
        };
        return Some(scope.members().map_or(true, |members| !members.is_empty()));
    }
    Some(group)
}

/// The records left under `run_dir`, split into groups that still run (or
/// cannot be probed) and groups that have ended. Ended groups' records are
/// removed: they are returned as evidence for the caller to record.
pub(crate) fn left_groups(
    run_dir: &Path,
) -> anyhow::Result<(Vec<HostCommandGroupRecord>, Vec<HostCommandGroupRecord>)> {
    let dir = run_dir.join(GROUP_RECORDS_DIR);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), Vec::new()));
        }
        Err(error) => return Err(error.into()),
    };
    let (mut running, mut ended) = (Vec::new(), Vec::new());
    for path in entries.map(|entry| entry.map(|entry| entry.path())) {
        let path = path?;
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let record: HostCommandGroupRecord = serde_json::from_slice(&std::fs::read(&path)?)
            .map_err(|error| {
                anyhow::anyhow!(
                    "host command group record {} is unreadable ({error}); check that group by hand, then remove the record",
                    path.display()
                )
            })?;
        if record_running(&record) == Some(false) {
            std::fs::remove_file(&path)?;
            ended.push(record);
        } else {
            running.push(record);
        }
    }
    Ok((running, ended))
}

/// Refuses while a group (or a process of its session) that a dead executor
/// left behind still runs.
/// Returns the records of the groups that have ended.
pub(crate) fn require_no_running_groups(
    run_dir: &Path,
    run_id: &str,
) -> anyhow::Result<Vec<HostCommandGroupRecord>> {
    let (running, ended) = left_groups(run_dir)?;
    let Some(first) = running.first() else {
        return Ok(ended);
    };
    let others = running.len() - 1;
    Err(anyhow::anyhow!(
        "fixed decomposition {run_id} cannot resume: host command '{}' (process group {}, pid {}) that a previous executor started is still running{}; stop that group (kill -TERM -{}) and resume again. If the group is unrelated (its id was reused), remove {}",
        first.command_id,
        first.pgid,
        first.pid,
        if others == 0 {
            String::new()
        } else {
            format!(", with {others} other group(s)")
        },
        first.pgid,
        run_dir
            .join(GROUP_RECORDS_DIR)
            .join(format!("{}.json", first.pgid))
            .display()
    ))
}
