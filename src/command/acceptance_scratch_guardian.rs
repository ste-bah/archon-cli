//! Owned observation subprocess. Parent keeps stdin open as a liveness pipe.
use archon_workflow::acceptance_scratch::{
    ObservationResult, ScratchPolicy, observe_commands_cancellable,
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

const FLAG: &str = "--internal-native-observer";
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
    serve().await.map_err(anyhow::Error::new)?;
    Ok(true)
}
async fn serve() -> WorkflowResult<()> {
    let stdin = std::io::stdin();
    let mut reader = std::io::BufReader::new(stdin);
    let line = read_request(&mut reader)?;
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
pub(crate) async fn launch(request: Request) -> WorkflowResult<ObservationResult> {
    launch_selected(request, None).await
}
pub(crate) async fn launch_selected(
    request: Request,
    selection: CheckSelection,
) -> WorkflowResult<ObservationResult> {
    use tokio::io::AsyncWriteExt;
    let count = validate_selected(&request, &selection)?.2.len() as u64;
    let mut command = tokio::process::Command::new(
        std::env::current_exe().map_err(|e| WorkflowError::SpecInvalid(e.to_string()))?,
    );
    #[cfg(not(test))]
    command.arg(FLAG);
    #[cfg(test)]
    command.args([
        "--exact",
        "command::acceptance_scratch_guardian::tests::guardian_entry",
        "--ignored",
        "--nocapture",
    ]);
    command
        .env_clear()
        .envs([("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    for key in &request.policy.environment_allowlist {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    let mut child = command
        .spawn()
        .map_err(|e| WorkflowError::SpecInvalid(e.to_string()))?;
    let mut pipe = child.stdin.take().unwrap();
    let bytes = request_line(&request, &selection)?;
    tokio::time::timeout(std::time::Duration::from_secs(5), pipe.write_all(&bytes))
        .await
        .map_err(|_| WorkflowError::StageFailed("guardian request delivery timed out".into()))?
        .map_err(|e| WorkflowError::SpecInvalid(e.to_string()))?;
    // Each finite filesystem phase and two subprocesses per nested floor have
    // their own limit; this outer deadline also bounds a wedged guardian.
    let budget = request
        .policy
        .timeout_secs
        .min(86400)
        .saturating_mul(count.saturating_mul(12).saturating_add(12))
        .saturating_add(30);
    let status =
        match tokio::time::timeout(std::time::Duration::from_secs(budget), child.wait()).await {
            Ok(status) => status.map_err(|e| WorkflowError::SpecInvalid(e.to_string()))?,
            Err(_) => {
                drop(pipe);
                let cleanup_grace = request
                    .policy
                    .timeout_secs
                    .clamp(5, 86400)
                    .saturating_add(10);
                if tokio::time::timeout(std::time::Duration::from_secs(cleanup_grace), child.wait())
                    .await
                    .is_err()
                {
                    let _ = child.kill().await;
                }
                return Err(WorkflowError::StageFailed(format!(
                    "native guardian lifetime exceeded; teardown not verified; evidence: {}",
                    request.evidence.display()
                )));
            }
        };
    drop(pipe);
    if !status.success() {
        return Err(WorkflowError::StageFailed(format!(
            "native observation guardian failed ({status}); evidence: {}",
            request.evidence.display()
        )));
    }
    let path = request.evidence.join("observation.json");
    serde_json::from_slice(&std::fs::read(&path).map_err(|e| WorkflowError::Io {
        path: path.clone(),
        source: e,
    })?)
    .map_err(Into::into)
}
#[cfg(unix)]
fn read_request(reader: &mut std::io::BufReader<std::io::Stdin>) -> WorkflowResult<String> {
    use std::os::fd::AsRawFd;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut bytes = Vec::new();
    loop {
        if std::time::Instant::now() >= deadline {
            return Err(WorkflowError::SpecInvalid(
                "guardian request deadline exceeded".into(),
            ));
        }
        // Read the descriptor directly so BufReader prefetch cannot hide bytes
        // from poll. The untouched reader subsequently owns the liveness pipe.
        let fd = reader.get_ref().as_raw_fd();
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
/// The request pipe is read with `poll(2)`; there is no Windows guardian, as
/// `acquire_lease` below already refuses the lock there.
#[cfg(not(unix))]
fn read_request(_reader: &mut std::io::BufReader<std::io::Stdin>) -> WorkflowResult<String> {
    Err(WorkflowError::SpecInvalid(
        "native observation guardian requires a Unix host".into(),
    ))
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    #[ignore = "internal subprocess entry"]
    async fn guardian_entry() {
        super::serve().await.unwrap();
    }
}

/// Lifetime OS lock, held by the guardian so parent death cannot release it
/// before process-group cleanup. Files remain stable; never unlink a lock inode.
pub(crate) fn acquire_lease(
    root: &std::path::Path,
    identity: &str,
) -> WorkflowResult<std::fs::File> {
    std::fs::create_dir_all(root).map_err(|source| WorkflowError::Io {
        path: root.into(),
        source,
    })?;
    let path = root.join(format!("{}.lock", content_digest(identity.as_bytes())));
    let mut options = std::fs::OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .mode(0o600);
    }
    let file = options.open(&path).map_err(|source| WorkflowError::Io {
        path: path.clone(),
        source,
    })?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(WorkflowError::PolicyDenied(
                "native observation already owns this repository".into(),
            ));
        }
    }
    #[cfg(not(unix))]
    return Err(WorkflowError::PolicyDenied(
        "native observation requires a supported Unix locking implementation".into(),
    ));
    Ok(file)
}
