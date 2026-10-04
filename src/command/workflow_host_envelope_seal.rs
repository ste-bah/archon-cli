//! The gate envelope a host-command child stages is persisted only redacted
//! (Issue 277).
//!
//! The child writes `gate-envelope.json` into its run-owned staging tree and
//! names its exact bytes in the prepared manifest. Those bytes reach the run
//! directory twice: the staged file stays behind when the parent refuses or
//! the child fails, and an accepted call publishes it to
//! `host-command-results/<call>/gate-envelope.json` with a receipt bound to
//! the published bytes. A child that printed a provider credential or a
//! credential-named allowlisted value into its report left it there in clear.
//!
//! So the parent seals the staged envelope as soon as the child has exited,
//! before any decision or persistence: an envelope holding a secret value is
//! replaced by the redacted envelope the call returns
//! ([`HostSecrets::envelope`]), or, when it is not a valid envelope, by its
//! bytes with every value replaced. The manifest entry is rebound to the
//! sealed bytes only when it described the child's raw bytes exactly, so a
//! child whose manifest misstates what it wrote is still refused by the
//! audit, and the receipt then binds the sealed bytes that are on disk.
//!
//! Boundary: the envelope's typed fields (enums, task ids, paths, error
//! kinds) keep their values, as the returned envelope does, because changing
//! them changes what the envelope means. The other staged outputs (frozen
//! contracts, locks, task bodies) are authoritative artifacts whose exact
//! bytes the locks, pins and verdicts bind; they are never rewritten here.
use std::path::Path;

use archon_workflow::task_set_contract::content_digest;
use archon_workflow::{GateEnvelopeV1, PreparedPublicationV1, WorkflowError, WorkflowResult};

use super::workflow_host_secrets::HostSecrets;

pub(crate) const ENVELOPE_FILE: &str = "gate-envelope.json";

/// The staged envelope's raw and sealed identity, when sealing changed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SealedEnvelope {
    raw: (u64, String),
    sealed: (u64, String),
}

/// Seal the envelope staged at `path`: no secret value stays in clear and
/// the file is readable by its owner only. `None` when nothing changed.
pub(crate) fn seal_staged_envelope(
    path: &Path,
    secrets: &HostSecrets,
) -> WorkflowResult<Option<SealedEnvelope>> {
    let io = |source| WorkflowError::Io {
        path: path.to_path_buf(),
        source,
    };
    // A missing or non-regular envelope is the audit's to refuse; never
    // write through a link the child left.
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io(error)),
    }
    let raw = std::fs::read(path).map_err(io)?;
    if !secrets.holds_secret(&raw) {
        owner_only(path)?;
        return Ok(None);
    }
    let sealed = match serde_json::from_slice::<GateEnvelopeV1>(&raw) {
        Ok(envelope) => serde_json::to_vec_pretty(&secrets.envelope(envelope))?,
        Err(_) => secrets.bytes(&raw),
    };
    write_owner_only(path, &sealed)?;
    Ok(Some(SealedEnvelope {
        raw: identity(&raw),
        sealed: identity(&sealed),
    }))
}

/// Point the manifest's envelope entry at the sealed bytes, only when it
/// named the child's raw bytes exactly.
pub(crate) fn rebind_manifest(prepared: &mut PreparedPublicationV1, sealed: &SealedEnvelope) {
    for entry in &mut prepared.entries {
        if entry.relative_path == ENVELOPE_FILE
            && (entry.byte_len, entry.blake3.as_str()) == (sealed.raw.0, sealed.raw.1.as_str())
        {
            (entry.byte_len, entry.blake3) = sealed.sealed.clone();
        }
    }
}

/// Restrict a published child-output file to its owner.
pub(crate) fn owner_only(path: &Path) -> WorkflowResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(
            |source| WorkflowError::Io {
                path: path.to_path_buf(),
                source,
            },
        )?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn write_owner_only(path: &Path, bytes: &[u8]) -> WorkflowResult<()> {
    use std::io::Write;
    let io = |source| WorkflowError::Io {
        path: path.to_path_buf(),
        source,
    };
    // Restrict first, so the sealed bytes are never readable by others.
    owner_only(path)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(io)?;
    file.write_all(bytes).map_err(io)?;
    file.sync_all().map_err(io)
}

fn identity(bytes: &[u8]) -> (u64, String) {
    (bytes.len() as u64, content_digest(bytes))
}
