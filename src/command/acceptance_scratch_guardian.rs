//! Owned observation subprocess. Parent keeps stdin open as a liveness pipe.
use archon_workflow::acceptance_scratch::{
    OBSERVATION_STALLED, ObservationResult, ScratchPolicy, observation_stall,
    observe_commands_cancellable,
};
use archon_workflow::acceptance_world::{AcceptanceCommandKind, FrozenCommandRef};
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, AcceptanceCheck, AcceptanceContract, AcceptancePin, content_digest,
    validate_acceptance_bundle,
};
use archon_workflow::{WorkflowError, WorkflowResult};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[path = "acceptance_scratch_guardian_diagnostics.rs"]
pub(crate) mod diagnostics;

const FLAG: &str = "--internal-native-observer";
/// How long the guardian waits with no request byte arriving. The parent
/// writes the line straight after spawn, so this only ends a parent that
/// stalled mid-line. A no-progress window, renewed by every byte (#356),
/// never a total; it starts when the guardian begins reading, so process
/// start-up never counts against it (Issue 302). Its expiry pauses.
const REQUEST_IDLE: std::time::Duration = std::time::Duration::from_secs(5);
/// Sidecar key on the request line that narrows an observation to a set of
/// pinned check ids. Carried beside `Request` rather than inside it so the R2
/// wire struct is byte-identical: the authored run's acceptance stage uses it
/// to re-run only the checks that failed the round before. It never widens —
/// an id outside the pinned chain is refused.
const CHECK_IDS_KEY: &str = "check_ids";
pub(crate) type CheckSelection = Option<std::collections::BTreeSet<String>>;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub policy: ScratchPolicy,
    pub source_commit: String,
    pub pin_path: PathBuf,
    pub expected_pin_digest: String,
    pub evidence: PathBuf,
}
pub(crate) fn validate_selected(
    request: &Request,
    selection: &CheckSelection,
) -> WorkflowResult<(AcceptanceContract, String, Vec<FrozenCommandRef>)> {
    let _read = crate::command::workflow_task_set::ChainRead::workflow_at(
        &request.pin_path,
        &request.policy.task_root,
    )?;
    let bytes = std::fs::read(&request.pin_path).map_err(|e| WorkflowError::Io {
        path: request.pin_path.clone(),
        source: e,
    })?;
    if content_digest(&bytes) != request.expected_pin_digest {
        return Err(WorkflowError::ArtifactInvalid(
            "native observer pin changed".into(),
        ));
    }
    let pin: AcceptancePin = serde_json::from_slice(&bytes)?;
    archon_workflow::task_skeleton::validate_full_chain(&request.policy.task_root, &pin)
        .map_err(|e| WorkflowError::ArtifactInvalid(e.to_string()))?;
    let path = request.policy.task_root.join(ACCEPTANCE_CONTRACT_FILE);
    let raw = std::fs::read(&path).map_err(|e| WorkflowError::Io {
        path: path.clone(),
        source: e,
    })?;
    let contract: AcceptanceContract = serde_json::from_slice(&raw)?;
    let ids = contract.acceptance.iter().map(|c| c.id.clone()).collect();
    let contract = validate_acceptance_bundle(&request.policy.task_root, Some(&pin), &ids)
        .map_err(|e| WorkflowError::ArtifactInvalid(e.to_string()))?;
    let digest = content_digest(&raw);
    if let Some(selected) = selection {
        let pinned: std::collections::BTreeSet<&str> = contract
            .acceptance
            .iter()
            .chain(&contract.supplementary)
            .map(|entry| entry.id.as_str())
            .collect();
        if let Some(unknown) = selected.iter().find(|id| !pinned.contains(id.as_str())) {
            return Err(WorkflowError::ArtifactInvalid(format!(
                "acceptance check '{unknown}' is not in the pinned contract"
            )));
        }
    }
    let refs = contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .filter(|entry| {
            selection
                .as_ref()
                .is_none_or(|selected| selected.contains(&entry.id))
        })
        .filter_map(|entry| {
            let (kind, command) = match &entry.check {
                AcceptanceCheck::Command { command, .. } => {
                    (AcceptanceCommandKind::Command, command.as_str())
                }
                AcceptanceCheck::Floor { contract } => (
                    AcceptanceCommandKind::NestedVerifier,
                    contract.typed_verifier_command.as_deref()?,
                ),
            };
            Some(FrozenCommandRef {
                acceptance_id: entry.id.clone(),
                kind,
                chain_digest: digest.clone(),
                command_digest: content_digest(command.as_bytes()),
            })
        })
        .collect();
    Ok((contract, digest, refs))
}
pub(crate) async fn entry() -> anyhow::Result<bool> {
    if std::env::args().nth(1).as_deref() != Some(FLAG) {
        return Ok(false);
    }
    match serve().await {
        // Issue 338: the only pause a guardian meets is a task set whose
        // interrupted publish no read can settle; its parent pauses on it.
        Err(WorkflowError::ControlPaused(evidence)) => {
            crate::command::workflow_host_command_operational::exit_unsettled_publish(&evidence)
        }
        served => served.map_err(anyhow::Error::new)?,
    }
    Ok(true)
}
async fn serve() -> WorkflowResult<()> {
    let stdin = std::io::stdin();
    let mut reader = std::io::BufReader::new(stdin);
    // Read the descriptor directly so BufReader prefetch cannot hide bytes
    // from poll. The untouched reader subsequently owns the liveness pipe.
    let line = read_request(reader.get_ref(), REQUEST_IDLE)?;
    let (request, selection) = parse_request_line(&line)?;
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    std::thread::spawn(move || {
        let mut byte = [0];
        let _ = reader.read(&mut byte);
        flag.store(true, Ordering::SeqCst);
    });
    let lock_root = std::env::temp_dir().join("archon-native-observer-locks");
    let identity = request
        .policy
        .repository
        .canonicalize()
        .map(archon_shell::paths::plain)
        .map_err(|e| WorkflowError::SpecInvalid(e.to_string()))?;
    let _lease = acquire_lease(&lock_root, &identity.to_string_lossy())?;
    let (contract, digest, refs) = validate_selected(&request, &selection)?;
    let result = observe_commands_cancellable(
        &request.policy,
        &request.source_commit,
        &contract,
        &digest,
        &refs,
        &request.evidence,
        cancel,
    )
    .await?;
    // #356: a phase or git child that made no progress is the host's, and
    // resumable: the parent pauses on it (its evidence is already written).
    if let Some(stall) = observation_stall(
        result
            .operational_errors
            .iter()
            .chain(&result.cleanup_error),
    ) {
        return Err(WorkflowError::ControlPaused(stall.clone()));
    }
    if !result.teardown_verified || !result.live_roots_unchanged {
        return Err(WorkflowError::ArtifactInvalid(
            "native observation void: live-root audit or teardown failed; inspect scratch evidence"
                .into(),
        ));
    }
    // Revalidate after command completion; live pin changes are never adopted.
    validate_selected(&request, &selection)?;
    Ok(())
}
pub(crate) fn parse_request_line(line: &str) -> WorkflowResult<(Request, CheckSelection)> {
    let mut value: serde_json::Value = serde_json::from_str(line)?;
    let selection = value
        .as_object_mut()
        .and_then(|object| object.remove(CHECK_IDS_KEY))
        .map(serde_json::from_value::<std::collections::BTreeSet<String>>)
        .transpose()?;
    Ok((serde_json::from_value(value)?, selection))
}
pub(crate) fn request_line(
    request: &Request,
    selection: &CheckSelection,
) -> WorkflowResult<Vec<u8>> {
    let mut value = serde_json::to_value(request)?;
    if let (Some(selected), Some(object)) = (selection, value.as_object_mut()) {
        object.insert(CHECK_IDS_KEY.to_string(), serde_json::to_value(selected)?);
    }
    let mut bytes = serde_json::to_vec(&value)?;
    bytes.push(b'\n');
    Ok(bytes)
}
#[path = "acceptance_scratch_guardian_launch.rs"]
pub(crate) mod launch;
pub(crate) use launch::{launch, launch_selected};

#[cfg(unix)]
fn read_request(
    source: &impl std::os::fd::AsRawFd,
    limit: std::time::Duration,
) -> WorkflowResult<String> {
    let mut last = std::time::Instant::now();
    let fd = source.as_raw_fd();
    let mut bytes = Vec::new();
    loop {
        if last.elapsed() >= limit {
            return Err(WorkflowError::ControlPaused(format!(
                "{OBSERVATION_STALLED}: no guardian request byte for {}s",
                limit.as_secs()
            )));
        }
        let mut poll = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut poll, 1, 50) };
        if ready < 0 {
            return Err(WorkflowError::SpecInvalid(
                std::io::Error::last_os_error().to_string(),
            ));
        }
        if ready == 0 {
            continue;
        }
        let mut byte = 0u8;
        let n = unsafe { libc::read(fd, (&mut byte as *mut u8).cast(), 1) };
        if n != 1 {
            return Err(WorkflowError::SpecInvalid(
                "guardian request pipe closed before newline".into(),
            ));
        }
        last = std::time::Instant::now();
        if byte == b'\n' {
            break;
        }
        bytes.push(byte);
        if bytes.len() >= 4 * 1024 * 1024 {
            return Err(WorkflowError::SpecInvalid(
                "native guardian request exceeded limit".into(),
            ));
        }
    }
    String::from_utf8(bytes).map_err(|e| WorkflowError::SpecInvalid(e.to_string()))
}

/// How long a busy lease is retried before the observation is refused.
///
/// A released lease can stay locked for a moment (Issue 286): the lock
/// belongs to the open file, and a child that any thread of the process has
/// forked shares that file until its `exec` closes the CLOEXEC descriptor.
/// A real concurrent observation holds the lease for far longer than this.
#[cfg(unix)]
const LEASE_SETTLE: std::time::Duration = std::time::Duration::from_secs(3);

/// Lifetime OS lock, held by the guardian so parent death cannot release it
/// before process-group cleanup. Files remain stable; never unlink a lock inode.
pub(crate) fn acquire_lease(
    root: &std::path::Path,
    identity: &str,
) -> WorkflowResult<std::fs::File> {
    acquire_lease_observing_busy(root, identity, || {})
}

fn acquire_lease_observing_busy(
    root: &std::path::Path,
    identity: &str,
    mut on_busy: impl FnMut(),
) -> WorkflowResult<std::fs::File> {
    std::fs::create_dir_all(root).map_err(|source| WorkflowError::Io {
        path: root.into(),
        source,
    })?;
    let path = root.join(format!("{}.lock", content_digest(identity.as_bytes())));
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(WorkflowError::PolicyDenied(
            "native observation requires a supported Unix locking implementation".into(),
        ))
    }
    #[cfg(unix)]
    {
        use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
        let mut options = std::fs::OpenOptions::new();
        options.create(true).read(true).write(true).truncate(false);
        options
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .mode(0o600);
        let file = options.open(&path).map_err(|source| WorkflowError::Io {
            path: path.clone(),
            source,
        })?;
        let deadline = std::time::Instant::now() + LEASE_SETTLE;
        while unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let error = std::io::Error::last_os_error();
            let busy = matches!(
                error.raw_os_error(),
                Some(libc::EWOULDBLOCK) | Some(libc::EINTR)
            );
            if !busy {
                return Err(WorkflowError::Io {
                    path,
                    source: error,
                });
            }
            on_busy();
            if std::time::Instant::now() >= deadline {
                return Err(WorkflowError::PolicyDenied(
                    "native observation already owns this repository".into(),
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        Ok(file)
    }
}
/// The request pipe is read with `poll(2)`; there is no Windows guardian, as
/// `acquire_lease` below already refuses the lock there.
#[cfg(not(unix))]
fn read_request<T>(_source: &T, _limit: std::time::Duration) -> WorkflowResult<String> {
    Err(WorkflowError::SpecInvalid(
        "native observation guardian requires a Unix host".into(),
    ))
}

#[cfg(all(test, unix))]
#[path = "acceptance_scratch_guardian_request_tests.rs"]
mod request_tests;

#[cfg(test)]
mod tests {
    #[tokio::test]
    #[ignore = "internal subprocess entry"]
    async fn guardian_entry() {
        super::serve().await.unwrap();
    }

    /// Issue 286: any thread's spawn forks the whole descriptor table, and a
    /// child shares the lease's open file (and so its lock) until it execs,
    /// which closes the CLOEXEC descriptor. A lease released in that window is
    /// still locked for a moment; the next observation of the same repository
    /// must wait it out, not refuse.
    #[cfg(unix)]
    #[test]
    fn a_released_lease_a_forked_child_still_shares_is_acquired_after_the_child_execs() {
        use std::io::{Read, Write};
        use std::os::fd::AsRawFd;
        use std::os::unix::net::UnixStream;
        use std::os::unix::process::CommandExt;
        let root = tempfile::tempdir().unwrap();
        let lease = super::acquire_lease(root.path(), "same-repository").unwrap();
        let (mut parent, child) = UnixStream::pair().unwrap();
        parent
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let spawner = std::thread::spawn(move || {
            let mut command = std::process::Command::new("true");
            let fd = child.as_raw_fd();
            // SAFETY: only async-signal-safe read/write syscalls run between
            // fork and exec. The parent releases the child after seeing busy.
            unsafe {
                command.pre_exec(move || {
                    let mut byte = 1u8;
                    if libc::write(fd, (&byte as *const u8).cast(), 1) != 1
                        || libc::read(fd, (&mut byte as *mut u8).cast(), 1) != 1
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            command.status().unwrap()
        });
        // The child confirms it inherited the lease and waits for permission
        // to exec; scheduling delays cannot make this test skip the window.
        parent.read_exact(&mut [0u8]).unwrap();
        drop(lease);
        let mut observed_busy = false;
        let again = super::acquire_lease_observing_busy(root.path(), "same-repository", || {
            if !observed_busy {
                observed_busy = true;
                parent.write_all(&[1]).unwrap();
            }
        });
        // Also unblock the child if the acquisition failed before probing.
        if !observed_busy {
            parent.write_all(&[1]).unwrap();
        }
        assert!(spawner.join().unwrap().success());
        assert!(
            observed_busy,
            "the acquisition must encounter the inherited lock"
        );
        assert!(again.is_ok(), "{:?}", again.err());
    }
}
