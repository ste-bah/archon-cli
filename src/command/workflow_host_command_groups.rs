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
    /// Only a synced teardown checkpoint can prove an empty Windows job after
    /// its name disappears. Legacy supervision markers carry no such proof.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) teardown_complete: bool,
    pub(crate) command_id: String,
    pub(crate) host_pid: u32,
    /// The start time of `host_pid` (Issue 270 round 5): with it, the
    /// identity of the writer, so a reader tells an owner that still runs
    /// from one that exited or whose pid was reused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) host_start: Option<u64>,
    /// The start time of the leader `pgid` (Issue 270 round 5): a process
    /// holding that pid with another start time proves the recorded group
    /// and session ended, whatever now uses their number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) leader_start: Option<u64>,
    pub(crate) started_at: String,
    /// Where the record was read from, for a message that names it.
    #[serde(skip)]
    pub(crate) file: Option<PathBuf>,
}

/// The suffix of a record whose survivors could not be written down: it
/// runs, whatever it says, until someone verifies the tree and removes it.
const UNKNOWN_SUFFIX: &str = ".unknown.json";

#[path = "workflow_host_command_groups_owner.rs"]
mod owner;
use owner::Owner;

#[path = "workflow_host_command_groups_guard.rs"]
mod guard;
pub(crate) use guard::{GroupEvidence, GroupRecordGuard};

fn unknown_path(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!("{stem}{UNKNOWN_SUFFIX}"))
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
    let leader_start = owner::leader_start(pid);
    let record = HostCommandGroupRecord {
        schema_version: 1,
        pgid,
        pid,
        session,
        job: job.map(str::to_string),
        survivors: Vec::new(),
        stalled: false,
        survivors_unknown: true,
        teardown_complete: false,
        command_id: command_id.to_string(),
        host_pid: std::process::id(),
        host_start: owner::own_start(),
        leader_start,
        started_at: chrono::Utc::now().to_rfc3339(),
        file: None,
    };
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| WorkflowError::Io { path, source }
    };
    std::fs::create_dir_all(dir).map_err(io(dir))?;
    // Named by the leader's identity, so a group id reused after an old
    // record was left never collides with it; held live from here, so no
    // reader takes this running command for a stall or an ended group.
    let (path, pending) = owner::claim(dir, pgid, leader_start);
    let mut guard = GroupRecordGuard {
        path,
        pending,
        kept: false,
        marker: None,
        record: record.clone(),
    };
    // The record first, then the marker that says "not settled": both exist
    // before any teardown can change permissions or exhaust the filesystem.
    // A crash between the two leaves a record judged by what it names.
    let staged = guard.path.with_extension("json.tmp");
    std::fs::write(&staged, serde_json::to_vec(&record)?).map_err(io(&staged))?;
    std::fs::rename(&staged, &guard.path).map_err(io(&guard.path))?;
    use std::io::Write;
    let mut marker = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&guard.pending)
        .map_err(io(&guard.pending))?;
    marker
        .write_all(&guard::supervised_marker(&record)?)
        .map_err(io(&guard.pending))?;
    marker.sync_all().map_err(io(&guard.pending))?;
    #[cfg(unix)]
    std::fs::File::open(dir)
        .and_then(|dir| dir.sync_all())
        .map_err(io(dir))?;
    guard.marker = Some(GroupEvidence::new(marker, record));
    Ok(guard)
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
    #[cfg(all(test, unix))]
    if REGISTER_DELAY.with(std::cell::Cell::get) {
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
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

#[cfg(any(unix, windows))]
fn survivors_running(
    record: &HostCommandGroupRecord,
    mut read: impl FnMut(u32) -> std::io::Result<Option<u64>>,
) -> Option<bool> {
    let mut unknown = false;
    for (pid, start) in &record.survivors {
        match read(*pid) {
            Ok(Some(actual)) if actual == *start => return Some(true),
            Ok(_) => {}
            Err(_) => unknown = true,
        }
    }
    if unknown { None } else { Some(false) }
}

/// Whether anything `record` names still runs: its group, any process of its
/// session, a survivor it names, or (Windows) any process in its job. `None`
/// where that cannot be probed, which a caller must treat as possibly
/// running; a failed session probe counts as running.
pub(crate) fn record_running(record: &HostCommandGroupRecord) -> Option<bool> {
    if record.survivors_unknown {
        return None;
    }
    #[cfg(windows)]
    {
        match survivors_running(record, archon_shell::job_object::identity_of) {
            Some(false) => {}
            other => return other,
        }
        if let Some(job) = &record.job {
            let active = archon_shell::job_object::named_job_running(job).ok()?;
            // Neither an initial nor a legacy supervision marker proves exits
            // merely because the job name disappeared. Explicit completion or
            // recorded identities must establish that termination finished.
            if !active && !record.teardown_complete && record.survivors.is_empty() {
                return None;
            }
            return Some(active);
        }
    }
    // A survivor is the same process only while its start time matches.
    #[cfg(unix)]
    match survivors_running(record, archon_shell::process_tree::identity_of) {
        Some(false) => {}
        other => return other,
    }
    // A stranger holding the leader's pid proves the group and session
    // ended: their numbers name someone else's processes now.
    #[cfg(unix)]
    match owner::leader_replaced(record) {
        Some(false) => {}
        Some(true) => return Some(false),
        None => return None,
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
    left_groups_impl(run_dir, true)
}

/// Like [`left_groups`], but leaves settled records and pending markers in
/// place. Used by planning paths that must inspect host state without healing it.
pub(crate) fn left_groups_read_only(
    run_dir: &Path,
) -> anyhow::Result<(Vec<HostCommandGroupRecord>, Vec<HostCommandGroupRecord>)> {
    left_groups_impl(run_dir, false)
}

fn left_groups_impl(
    run_dir: &Path,
    remove_healed_records: bool,
) -> anyhow::Result<(Vec<HostCommandGroupRecord>, Vec<HostCommandGroupRecord>)> {
    let dir = run_dir.join(GROUP_RECORDS_DIR);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), Vec::new()));
        }
        Err(error) => {
            return Err(anyhow::anyhow!(
                "cannot list host command record directory {}: {error}",
                dir.display()
            ));
        }
    };
    let (mut running, mut ended) = (Vec::new(), Vec::new());
    for path in entries.map(|entry| entry.map(|entry| entry.path())) {
        let path = path?;
        let extension = path.extension().and_then(|ext| ext.to_str());
        if !matches!(extension, Some("json" | "pending")) {
            continue;
        }
        let lone_marker = extension == Some("pending");
        if lone_marker {
            let original = path.with_extension("json");
            if original.exists() || unknown_path(&original).exists() {
                continue;
            }
        }
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            // Settled since the listing (here, with its record, or by its
            // guard): there is nothing left to judge.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "cannot read host command record {}: {error}",
                    path.display()
                ));
            }
        };
        let marker_record = if lone_marker {
            guard::pending_record(&path)?
        } else {
            None
        };
        let mut authoritative = marker_record.as_ref().is_some_and(|marker| marker.settled);
        let mut record: HostCommandGroupRecord = marker_record.map(|marker| Ok(marker.record)).unwrap_or_else(|| serde_json::from_slice(&bytes))
            .map_err(|error| {
                anyhow::anyhow!(
                    "host command group record {} is unreadable ({error}); verify by hand that none of its processes runs, then remove {} and resume again",
                    path.display(), refusal_files(&path)
                )
            })?;
        let unknown = path.to_string_lossy().ends_with(UNKNOWN_SUFFIX);
        let pending = path.with_extension("pending");
        let marked = !unknown && !lone_marker && pending.exists();
        record.file = Some(path.clone());
        let owner = owner::owner(&record, &pending);
        // A live local guard may be updating its marker right now. It is
        // still running work, never a stall or an ended record.
        if owner == Owner::Supervising {
            // Incomplete on disk protects crash recovery, but a live local
            // guard is ordinary running work, not a settled stall.
            record.survivors_unknown = false;
            running.push(record);
            continue;
        }
        if marked && let Some(marker) = guard::pending_record(&pending)? {
            authoritative |= marker.settled;
            record = marker.record;
        }
        record.file = Some(path.clone());
        match owner {
            // A marker its owner can no longer settle: the record is judged
            // by what it names (a crash), or by its stall if it was kept.
            Owner::Exited if marked => {}
            _ if marked && !authoritative && !record.stalled => {
                record.stalled = true;
                record.survivors_unknown = true;
            }
            _ => {}
        }
        // An unknown record, or a marker left alone by a stall whose
        // survivors could not be written: unknown until someone verifies.
        if unknown || (lone_marker && !authoritative) {
            record.stalled = true;
            record.survivors_unknown = true;
        }
        let probe = record_running(&record);
        if probe.is_none() {
            record.survivors_unknown = true;
        }
        if probe == Some(false) {
            if remove_healed_records {
                // Remove the marker first: a failed removal retains both names,
                // and the error says exactly which entry needs permission repair.
                if marked {
                    remove_healed(&pending)?;
                }
                remove_healed(&path)?;
            }
            ended.push(record);
        } else {
            running.push(record);
        }
    }
    Ok((running, ended))
}

pub(super) fn refusal_files(path: &Path) -> String {
    let mut files = vec![path.to_path_buf()];
    if path.extension().is_some_and(|ext| ext == "pending") {
        let original = path.with_extension("json");
        for record in [original.clone(), unknown_path(&original)] {
            if record.exists() {
                files.push(record);
            }
        }
    } else {
        let pending = if let Some(stem) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(UNKNOWN_SUFFIX))
        {
            path.with_file_name(format!("{stem}.pending"))
        } else {
            path.with_extension("pending")
        };
        if pending.exists() {
            files.push(pending);
        }
    }
    files
        .iter()
        .map(|file| file.display().to_string())
        .collect::<Vec<_>>()
        .join(" and ")
}

fn remove_healed(path: &Path) -> anyhow::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(anyhow::anyhow!(
            "cannot remove healed host command record {}: {error}; restore write permission to its directory and resume again",
            path.display()
        )),
    }
}

#[path = "workflow_host_command_groups_stalled.rs"]
mod stalled;
pub(crate) use stalled::{stall_note, stalled_running};

/// Refuses while a group (or a process of its session) that a dead executor
/// left behind still runs.
/// Returns the records of the groups that have ended.
pub(crate) fn require_no_running_groups(
    run_dir: &Path,
    run_id: &str,
) -> anyhow::Result<Vec<HostCommandGroupRecord>> {
    let (running, ended) = left_groups(run_dir)?;
    require_no_running_groups_from(run_dir, run_id, running, ended)
}

/// Read-only form for dry-run admission. The ended records are what a live
/// resume would remove; this function never removes them or their markers.
pub(crate) fn require_no_running_groups_read_only(
    run_dir: &Path,
    run_id: &str,
) -> anyhow::Result<Vec<HostCommandGroupRecord>> {
    let (running, ended) = left_groups_read_only(run_dir)?;
    require_no_running_groups_from(run_dir, run_id, running, ended)
}

fn require_no_running_groups_from(
    run_dir: &Path,
    run_id: &str,
    running: Vec<HostCommandGroupRecord>,
    ended: Vec<HostCommandGroupRecord>,
) -> anyhow::Result<Vec<HostCommandGroupRecord>> {
    let Some(first) = running.first() else {
        return Ok(ended);
    };
    if first.survivors_unknown {
        let path = first.file.clone().unwrap_or_else(|| {
            run_dir
                .join(GROUP_RECORDS_DIR)
                .join(format!("{}.json", first.pgid))
        });
        let files = refusal_files(&path);
        return Err(anyhow::anyhow!(
            "fixed decomposition {run_id} cannot resume: host command '{}' has unknown survivors; verify by hand that none of its processes runs, then remove {files} and resume again",
            first.command_id
        ));
    }
    Err(anyhow::anyhow!(
        "{}",
        remedy::known_refusal(first, run_id, running.len() - 1)
    ))
}

#[cfg(all(test, unix))]
#[path = "workflow_host_command_groups_tests.rs"]
mod tests;

#[cfg(all(test, windows))]
#[path = "workflow_host_command_groups_windows_tests.rs"]
mod windows_tests;

#[cfg(all(test, unix))]
thread_local! {
    pub(crate) static REGISTER_DELAY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[path = "workflow_host_command_groups_remedy.rs"]
mod remedy;
