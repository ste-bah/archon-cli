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
    io::{BufRead, Read},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

const FLAG: &str = "--internal-native-observer";
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub policy: ScratchPolicy,
    pub source_commit: String,
    pub pin_path: PathBuf,
    pub expected_pin_digest: String,
    pub evidence: PathBuf,
}
pub(crate) fn validate(
    request: &Request,
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
    let refs = contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
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
    let mut line = String::new();
    let stdin = std::io::stdin();
    let mut reader = std::io::BufReader::new(stdin);
    reader
        .by_ref()
        .take(4 * 1024 * 1024)
        .read_line(&mut line)
        .map_err(|e| WorkflowError::SpecInvalid(e.to_string()))?;
    if !line.ends_with('\n') {
        return Err(WorkflowError::SpecInvalid(
            "native guardian request exceeded limit".into(),
        ));
    }
    let request: Request = serde_json::from_str(&line)?;
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
    let (contract, digest, refs) = validate(&request)?;
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
    validate(&request)?;
    Ok(())
}
pub(crate) async fn launch(request: Request) -> WorkflowResult<ObservationResult> {
    use tokio::io::AsyncWriteExt;
    validate(&request)?;
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
    let mut child = command
        .spawn()
        .map_err(|e| WorkflowError::SpecInvalid(e.to_string()))?;
    let mut pipe = child.stdin.take().unwrap();
    let mut bytes = serde_json::to_vec(&request)?;
    bytes.push(b'\n');
    pipe.write_all(&bytes)
        .await
        .map_err(|e| WorkflowError::SpecInvalid(e.to_string()))?;
    let status = child
        .wait()
        .await
        .map_err(|e| WorkflowError::SpecInvalid(e.to_string()))?;
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
