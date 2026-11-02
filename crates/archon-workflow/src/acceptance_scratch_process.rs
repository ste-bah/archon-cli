//! Bounded native command process groups; no command text in argv.
use super::*;
use std::process::Stdio;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

#[path = "acceptance_scratch_process_finish.rs"]
mod finish;
#[path = "acceptance_scratch_process_mutation.rs"]
mod mutation;

/// The operational error of a check stopped at its site's no-progress window.
pub const CHECK_TIMED_OUT: &str =
    "native acceptance command timed out: no output or process-tree activity";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckResult {
    pub acceptance_id: String,
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub quota_walk_count: u64,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// Diagnostic about names withheld from the check, kept out of raw output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_note: Option<String>,
    pub operational_error: Option<String>,
    /// Why the check's own run gave no verdict, where the observation
    /// detected it ([`super::faults::classify`] judges by it). Absent on
    /// results that carry none, stored ones included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fault: Option<super::faults::FaultCause>,
    /// Classification of raw output, never recomputed from redacted evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification: Option<crate::acceptance_check_crash::CheckClassification>,
    /// Issue 544: the streams as captured, before redaction, for host
    /// verdicts only. Never serialized, stored, printed or shown.
    #[serde(skip)]
    pub verdict_streams: VerdictStreams,
}

/// The raw streams of one capture, held in memory for verdicts (Issue 544),
/// with the nonce of the host mutation protocol the capture carried. Its
/// `Debug` shows no content, so no log or panic message can carry it.
#[derive(Clone, Default)]
pub struct VerdictStreams(Option<std::sync::Arc<RawCapture>>);

struct RawCapture {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    protocol_nonce: Option<String>,
}

impl std::fmt::Debug for VerdictStreams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() {
            "VerdictStreams(kept)"
        } else {
            "VerdictStreams(none)"
        })
    }
}

impl CheckResult {
    /// Keep the streams as they are now (raw), before redaction changes
    /// them, with the nonce of the mutation protocol they may carry.
    pub fn keep_verdict_streams(&mut self, protocol_nonce: Option<&str>) {
        self.verdict_streams = VerdictStreams(Some(std::sync::Arc::new(RawCapture {
            stdout: self.stdout.clone(),
            stderr: self.stderr.clone(),
            protocol_nonce: protocol_nonce.map(str::to_string),
        })));
    }

    /// The nonce of the host mutation protocol in this capture, if any.
    pub fn protocol_nonce(&self) -> Option<&str> {
        self.verdict_streams.0.as_deref()?.protocol_nonce.as_deref()
    }

    /// The stdout a host verdict reads: the raw capture when kept, else the
    /// (redacted) stored stream.
    pub fn verdict_stdout(&self) -> &[u8] {
        self.verdict_streams
            .0
            .as_deref()
            .map_or(self.stdout.as_slice(), |raw| raw.stdout.as_slice())
    }

    /// The stderr a host verdict reads (see [`Self::verdict_stdout`]).
    pub fn verdict_stderr(&self) -> &[u8] {
        self.verdict_streams
            .0
            .as_deref()
            .map_or(self.stderr.as_slice(), |raw| raw.stderr.as_slice())
    }

    /// Whether the output reports zero work (Issue 544): the capture's own
    /// classification of the raw streams when it has one, else the streams a
    /// verdict reads. Never redacted evidence when the raw capture is known.
    pub fn reports_zero_work(&self) -> bool {
        match &self.classification {
            Some(classification) => classification.zero_work,
            None => crate::acceptance::output_reports_zero_work(
                &String::from_utf8_lossy(self.verdict_stdout()),
                &String::from_utf8_lossy(self.verdict_stderr()),
            ),
        }
    }

    /// A copy whose streams are the ones host verdicts read, for verdict code
    /// that takes a whole result. Never store or show it.
    pub fn for_verdict(&self) -> Self {
        let mut copy = self.clone();
        copy.stdout = self.verdict_stdout().to_vec();
        copy.stderr = self.verdict_stderr().to_vec();
        // The copy's own streams are now the raw ones: later edits to them
        // (filtering protocol lines) are what its readers see.
        copy.verdict_streams = VerdictStreams::default();
        copy
    }
}

/// One input the host will move for a mutation check. Both operation paths
/// are relative to the captured site root; `named_path` is the check's name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MutationInputPlan {
    pub named_path: String,
    pub site_copy_alias: bool,
    pub move_path: PathBuf,
    pub aside_path: PathBuf,
}

/// Host-computed move and restore inputs for one mutation check.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MutationPlan {
    pub nonce: String,
    pub inputs: Vec<MutationInputPlan>,
}
/// The leader's handle; confinement owns the process tree separately.
type RunChild = tokio::process::Child;

#[path = "acceptance_scratch_confine.rs"]
mod confine;
#[cfg(not(any(unix, windows)))]
use confine::Confinement;
use confine::{leader_exit, reap, terminate};

/// Scan the check's tree every this many 25 ms ticks (Issue 270): a
/// descendant that leaves the group and the session is tied to the check
/// only while its parent lives, so it has to be seen while the check runs.
const SCAN_TICKS: u32 = 8;

/// Spawn the prepared command as a confined child: a new Unix process group,
/// or a Windows Job Object with kill-on-drop. On Linux the leader also reaps
/// its orphans, so a descendant whose parent exits stays in its tree.
#[cfg(not(windows))]
fn spawn_confined(mut process: tokio::process::Command, cwd: &Path) -> WorkflowResult<RunChild> {
    process.process_group(0);
    #[cfg(unix)]
    // SAFETY: prctl is an async-signal-safe syscall.
    unsafe {
        process.pre_exec(archon_shell::process_tree::become_subreaper);
    }
    process.spawn().map_err(|e| WorkflowError::io(cwd, e))
}
#[cfg(windows)]
fn spawn_confined(
    mut process: tokio::process::Command,
    cwd: &Path,
) -> WorkflowResult<confine::OwnedCheck> {
    process.creation_flags(archon_shell::job_object::CREATE_SUSPENDED_FLAG);
    let mut child = process.spawn().map_err(|e| WorkflowError::io(cwd, e))?;
    let job = archon_shell::job_object::Job::create(None).map_err(|e| WorkflowError::io(cwd, e))?;
    let (Some(pid), Some(handle)) = (child.id(), child.raw_handle()) else {
        let _ = child.start_kill();
        return Err(invalid("scratch child has no process handle"));
    };
    job.adopt_suspended(handle, pid)
        .map_err(|e| WorkflowError::io(cwd, e))?;
    Ok(confine::OwnedCheck::new(child, job))
}

/// Take the child's three pipes.
fn take_pipes(
    child: &mut RunChild,
) -> (
    tokio::process::ChildStdout,
    tokio::process::ChildStderr,
    tokio::process::ChildStdin,
) {
    (
        child.stdout.take().unwrap(),
        child.stderr.take().unwrap(),
        child.stdin.take().unwrap(),
    )
}
async fn drain(
    pipe: impl AsyncRead + Unpin,
    limit: usize,
    overflow: Arc<AtomicBool>,
    progress: archon_shell::progress::Progress,
) -> std::io::Result<(Vec<u8>, bool)> {
    let (retained, total) = drain_progress(pipe, limit, &overflow, Some(&progress)).await?;
    let truncated = total > retained.len() as u64;
    Ok((retained, truncated))
}

/// Read `pipe` to its end keeping at most `limit` bytes: what was kept and
/// how many bytes were read in all. `overflow` is set the moment a byte is
/// dropped, so a caller watching it can stop the producer.
pub(crate) async fn drain_counted(
    pipe: impl AsyncRead + Unpin,
    limit: usize,
    overflow: &AtomicBool,
) -> std::io::Result<(Vec<u8>, u64)> {
    drain_progress(pipe, limit, overflow, None).await
}

async fn drain_progress(
    mut pipe: impl AsyncRead + Unpin,
    limit: usize,
    overflow: &AtomicBool,
    progress: Option<&archon_shell::progress::Progress>,
) -> std::io::Result<(Vec<u8>, u64)> {
    let mut retained = Vec::new();
    let mut total = 0u64;
    let mut buffer = [0; 8192];
    loop {
        let n = pipe.read(&mut buffer).await?;
        if n == 0 {
            return Ok((retained, total));
        }
        if let Some(progress) = progress {
            progress.record();
        }
        total += n as u64;
        let keep = n.min(limit.saturating_sub(retained.len()));
        retained.extend_from_slice(&buffer[..keep]);
        if keep < n {
            overflow.store(true, Ordering::SeqCst);
        }
    }
}
pub(super) fn scratch_size(path: &Path) -> std::io::Result<u64> {
    let m = std::fs::symlink_metadata(path)?;
    if m.is_dir() {
        let mut n = 0u64;
        for entry in std::fs::read_dir(path)? {
            let path = entry?.path();
            match scratch_size(&path) {
                Ok(size) => n = n.saturating_add(size),
                // A live command may remove a temp file between listing and stat.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        Ok(n)
    } else if m.is_file() {
        Ok(m.len())
    } else {
        Ok(0)
    }
}
/// Where and under what bounds one authorized command runs.
///
/// The scratch observer builds one from its prepared roots; the authored
/// run's acceptance stage builds one over the live checkout when no
/// `[workflow.acceptance_execution]` policy is configured. Both go through
/// the same `run_at`, so there is one bounded process-group runner.
pub struct CommandSite<'a> {
    pub project: &'a Path,
    pub repository: &'a Path,
    pub environment: BTreeMap<String, String>,
    /// Resolved operator configuration, applied as archon-owned defaults.
    pub resource_limits: archon_tools::workflow_resource_env::CargoResourceLimits,
    /// Audited against `scratch_bytes` while a command runs; `None` for a
    /// site in live roots, whose size is not the command's to bound.
    pub audit_root: Option<&'a Path>,
    /// A build cache outside `audit_root` (Batch J2), audited with it.
    pub audit_target: Option<&'a Path>,
    pub scratch_bytes: u64,
    pub output_bytes: usize,
    /// No output or process-tree activity for this long stops the command.
    pub timeout_secs: u64,
    /// The proof code computes this before the check is spawned.
    pub mutation_plan: Option<&'a MutationPlan>,
    /// Root identity captured when the scratch site was created.
    pub mutation_root: Option<super::mutation_fd::SiteRoot>,
    /// Redacts allowlisted host values from captured output. A direct site
    /// redacts nothing: with no policy it binds no credential-shaped name
    /// (Issue 345), and under a configured policy it runs only the host's
    /// own floor predicates, which get the policy's forwarded values.
    pub(super) redactor: Option<&'a ScratchRoots>,
}
impl CommandSite<'_> {
    fn redact(&self, bytes: &[u8], truncated: bool) -> Vec<u8> {
        match self.redactor {
            Some(roots) => roots.redact_output(bytes, truncated),
            None => bytes.to_vec(),
        }
    }

    /// Issue 544: a stream with each secret as an indexed placeholder, for
    /// typed facts computed from the raw capture (see `io::placeholders`).
    pub(super) fn placeholder(&self, bytes: &[u8], truncated: bool) -> Vec<u8> {
        match self.redactor {
            Some(roots) => roots.placeholder_output(bytes, truncated),
            None => bytes.to_vec(),
        }
    }
}
impl CommandSite<'_> {
    fn audited_size(&self, root: &Path) -> std::io::Result<u64> {
        let target = match self.audit_target {
            Some(target) if !target.starts_with(root) => scratch_size(target)?,
            _ => 0,
        };
        Ok(scratch_size(root)?.saturating_add(target))
    }
}

#[cfg(unix)]
fn record_holder_audit(
    audit: confine::HolderAudit,
    error: &mut Option<String>,
    fault: &mut Option<super::faults::FaultCause>,
) {
    match audit {
        confine::HolderAudit::Escaped(escape) => {
            *error = Some(escape);
            *fault = Some(super::faults::FaultCause::LeftBehind);
        }
        confine::HolderAudit::HostFailure(message) => {
            *error = Some(message);
            *fault = None;
        }
    }
}

#[cfg(all(test, unix))]
#[path = "acceptance_scratch_process_tests.rs"]
mod tests;

#[cfg(all(test, windows))]
#[path = "acceptance_scratch_process_windows_tests.rs"]
mod windows_tests;

#[path = "acceptance_scratch_process_run.rs"]
mod run;
pub use run::run_at;
