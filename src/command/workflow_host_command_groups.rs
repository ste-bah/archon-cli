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
    /// The name of the command's Job Object on Windows (Issue 273), which a
    /// resume opens to ask whether any process in it still runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) job: Option<String>,
    /// Processes (pid, start time) left alive when teardown stalled
    /// (Issue 270 round 2). A survivor that escaped the group and the
    /// session is named here, and the record runs while any of them lives.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) survivors: Vec<(u32, u64)>,
    /// Teardown stalled and this record was kept (Issue 270 round 3): the
    /// run pauses, and no operational retry starts, while it still runs.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) stalled: bool,
    /// Teardown stalled and its survivors are not known (or could not be
    /// written down): the record runs until someone verifies the tree.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) survivors_unknown: bool,
    pub(crate) command_id: String,
    pub(crate) host_pid: u32,
    pub(crate) started_at: String,
    /// Where the record was read from, for a message that names it.
    #[serde(skip)]
    pub(crate) file: Option<PathBuf>,
}

/// The suffix of a record whose survivors could not be written down: it
/// runs, whatever it says, until someone verifies the tree and removes it.
const UNKNOWN_SUFFIX: &str = ".unknown.json";

/// Removes its record when the supervisor is done with the group, unless
/// teardown stalled and the record is kept for a resume to see.
#[derive(Debug)]
pub(crate) struct GroupRecordGuard {
    path: PathBuf,
    kept: bool,
}

impl GroupRecordGuard {
    /// Keep the record: teardown stalled. `survivors` (pid, start time) are
    /// written into it so a resume refuses while any of them still runs;
    /// `None` means they are not known, and the record then runs until the
    /// tree is verified. If the record cannot be rewritten, it is renamed to
    /// `<pgid>.unknown.json`, which means the same: never the old contents,
    /// whose empty survivor list would let a resume go ahead.
    pub(crate) fn keep(mut self, survivors: Option<&[(u32, u64)]>) {
        self.kept = true;
        let rewritten = std::fs::read(&self.path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| {
                serde_json::from_slice::<HostCommandGroupRecord>(&bytes)
                    .map_err(|error| error.to_string())
            })
            .and_then(|mut record| {
                record.stalled = true;
                record.survivors_unknown = survivors.is_none();
                record.survivors = survivors.map(<[_]>::to_vec).unwrap_or_default();
                let bytes = serde_json::to_vec(&record).map_err(|error| error.to_string())?;
                let staged = self.path.with_extension("json.tmp");
                std::fs::write(&staged, bytes).map_err(|error| error.to_string())?;
                std::fs::rename(&staged, &self.path).map_err(|error| error.to_string())
            });
        if let Err(error) = rewritten {
            let unknown = unknown_path(&self.path);
            let renamed = std::fs::rename(&self.path, &unknown);
            tracing::error!(
                %error,
                renamed = renamed.is_ok(),
                record = %self.path.display(),
                "writing the survivors of a stalled host command teardown failed; the record now means unknown survivors"
            );
        }
    }
}

fn unknown_path(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!("{stem}{UNKNOWN_SUFFIX}"))
}

impl Drop for GroupRecordGuard {
    fn drop(&mut self) {
        if !self.kept {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Writes the record of `pgid` under `dir` before the supervisor waits on it.
pub(crate) fn record_group(
    dir: &Path,
    pgid: u32,
    pid: u32,
    session: Option<u32>,
    job: Option<&str>,
    command_id: &str,
) -> WorkflowResult<GroupRecordGuard> {
    let record = HostCommandGroupRecord {
        schema_version: 1,
        pgid,
        pid,
        session,
        job: job.map(str::to_string),
        survivors: Vec::new(),
        stalled: false,
        survivors_unknown: false,
        command_id: command_id.to_string(),
        host_pid: std::process::id(),
        started_at: chrono::Utc::now().to_rfc3339(),
        file: None,
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
    Ok(GroupRecordGuard { path, kept: false })
}

/// [`record_group`] for a command whose pid is `leader` (the supervisor makes
/// each command a group leader and, on Unix, a session leader), when the run
/// keeps records and the child has an id.
pub(crate) fn record_in(
    dir: Option<&Path>,
    leader: Option<u32>,
    job: Option<&str>,
    command_id: &str,
) -> WorkflowResult<Option<GroupRecordGuard>> {
    let session = if cfg!(unix) { leader } else { None };
    match (dir, leader) {
        (Some(dir), Some(pgid)) => {
            record_group(dir, pgid, pgid, session, job, command_id).map(Some)
        }
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

/// Whether anything `record` names still runs: its group, any process of its
/// session, a survivor it names, or (Windows) any process in its job. `None`
/// where that cannot be probed, which a caller must treat as possibly
/// running; a failed session probe counts as running.
pub(crate) fn record_running(record: &HostCommandGroupRecord) -> Option<bool> {
    if record.survivors_unknown {
        return None;
    }
    // A survivor is the same process only while its start time matches.
    #[cfg(unix)]
    if record
        .survivors
        .iter()
        .any(|(pid, start)| archon_shell::process_tree::start_of(*pid) == Some(*start))
    {
        return Some(true);
    }
    // A job is gone once no handle holds it, and killed on close with every
    // process it held; while it exists, its accounting says what still runs.
    #[cfg(windows)]
    if let Some(job) = &record.job {
        return archon_shell::job_object::named_job_running(job).ok();
    }
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
        let mut record: HostCommandGroupRecord = serde_json::from_slice(&std::fs::read(&path)?)
            .map_err(|error| {
                anyhow::anyhow!(
                    "host command group record {} is unreadable ({error}); check that group by hand, then remove the record",
                    path.display()
                )
            })?;
        if path.to_string_lossy().ends_with(UNKNOWN_SUFFIX) {
            record.stalled = true;
            record.survivors_unknown = true;
        }
        record.file = Some(path.clone());
        if record_running(&record) == Some(false) {
            std::fs::remove_file(&path)?;
            ended.push(record);
        } else {
            running.push(record);
        }
    }
    Ok((running, ended))
}

/// The kept records of stalled teardowns that still run (or whose survivors
/// are unknown): while any exists, the run must pause and no operational
/// retry may start (Issue 270 round 3).
pub(crate) fn stalled_running(run_dir: &Path) -> anyhow::Result<Vec<HostCommandGroupRecord>> {
    let (running, _) = left_groups(run_dir)?;
    Ok(running
        .into_iter()
        .filter(|record| record.stalled || record.survivors_unknown)
        .collect())
}

/// What a stalled record adds to a refusal: who may still run.
pub(crate) fn stall_note(record: &HostCommandGroupRecord) -> String {
    if record.survivors_unknown {
        " (its teardown stalled and its survivors are unknown: verify that none of its processes runs)".to_string()
    } else if record.stalled {
        format!(
            " (its teardown stalled; survivors: {})",
            record
                .survivors
                .iter()
                .map(|(pid, _)| pid.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        String::new()
    }
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
        "fixed decomposition {run_id} cannot resume: host command '{}' (process group {}, pid {}) that a previous executor started is still running{}{}; stop that group (kill -TERM -{}) and resume again. If the group is unrelated (its id was reused), remove {}",
        first.command_id,
        first.pgid,
        first.pid,
        stall_note(first),
        if others == 0 {
            String::new()
        } else {
            format!(", with {others} other group(s)")
        },
        first.pgid,
        first
            .file
            .clone()
            .unwrap_or_else(|| run_dir
                .join(GROUP_RECORDS_DIR)
                .join(format!("{}.json", first.pgid)))
            .display()
    ))
}

#[cfg(all(test, unix))]
#[path = "workflow_host_command_groups_tests.rs"]
mod tests;
