//! Decode, redact and verify staged host envelopes before publication (#277).
//! A manifest must describe the child's raw bytes; only an honest entry can
//! be rebound to the sealed bytes. Temporary envelopes are removed on exit.
use std::path::Path;

use archon_workflow::task_set_contract::content_digest;
use archon_workflow::{GateEnvelopeV1, PreparedPublicationV1, WorkflowError, WorkflowResult};

use super::workflow_host_secrets::HostSecrets;

pub(crate) const ENVELOPE_FILE: &str = "gate-envelope.json";

/// Covers early returns and cancellation of the executor's future too.
pub(crate) struct EnvelopeCleanup<'a> {
    pub(crate) path: &'a Path,
    pub(crate) secrets: &'a HostSecrets,
}

impl Drop for EnvelopeCleanup<'_> {
    fn drop(&mut self) {
        if let Err(error) = seal_staged_envelope(self.path, self.secrets, None) {
            let _ = std::fs::remove_file(self.path);
            tracing::warn!(%error, "sealing an exiting call's staged envelope failed");
        }
    }
}

pub(crate) fn seal_staged_envelope(
    path: &Path,
    secrets: &HostSecrets,
    mut prepared: Option<&mut PreparedPublicationV1>,
) -> WorkflowResult<()> {
    let io = |source| WorkflowError::Io {
        path: path.to_path_buf(),
        source,
    };
    if let Some(parent) = path.parent() {
        remove_temporary_envelopes(parent)?;
    }
    // Never follow a link the child left. The publication audit refuses it.
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io(error)),
    }
    let raw = std::fs::read(path).map_err(io)?;
    let raw_identity = identity(&raw);
    // Check before sealing, but still scrub a dishonest child's staging.
    let mismatch = prepared.as_ref().is_some_and(|manifest| {
        let entries: Vec<_> = manifest
            .entries
            .iter()
            .filter(|entry| entry.relative_path == ENVELOPE_FILE)
            .collect();
        entries.len() != 1
            || entries.iter().any(|entry| {
                (entry.byte_len, entry.blake3.as_str()) != (raw_identity.0, raw_identity.1.as_str())
            })
    });
    let sealed = seal_bytes(&raw, secrets)?;
    if sealed != raw {
        write_owner_only(path, &sealed)?;
    } else {
        owner_only(path)?;
    }
    if mismatch {
        return Err(WorkflowError::ArtifactInvalid(
            "staged output gate-envelope.json raw manifest identity mismatch; return the length and digest of the bytes actually staged".into(),
        ));
    }
    if let Some(manifest) = prepared.as_mut() {
        for entry in &mut manifest.entries {
            if entry.relative_path == ENVELOPE_FILE {
                (entry.byte_len, entry.blake3) = identity(&sealed);
            }
        }
    }
    Ok(())
}

fn seal_bytes(raw: &[u8], secrets: &HostSecrets) -> WorkflowResult<Vec<u8>> {
    // A malformed document cannot be safely traversed: discard the whole field.
    let Ok(decoded) = serde_json::from_slice::<serde_json::Value>(raw) else {
        return Ok(b"null".to_vec());
    };
    let mut clean = decoded.clone();
    secrets.strings(&mut clean);
    let changed = clean != decoded;
    let value = if changed {
        match serde_json::from_value::<GateEnvelopeV1>(decoded) {
            Ok(envelope) => serde_json::to_value(secrets.envelope(envelope))?,
            Err(_) => clean,
        }
    } else {
        clean
    };
    // Verification traverses only data the child can supply. Wire-contract
    // keys and closed enum constants are owned by the schema, even when a
    // credential happens to have the same spelling (for example "body").
    if let Ok(envelope) = serde_json::from_value::<GateEnvelopeV1>(value.clone()) {
        let verified = secrets.envelope(envelope.clone());
        if verified != envelope || secrets.envelope_holds_secret(&envelope) {
            return refusal_envelope();
        }
        return if changed {
            Ok(serde_json::to_vec_pretty(&value)?)
        } else {
            Ok(raw.to_vec())
        };
    }
    // A redaction refusal is valid operational evidence, never JSON null.
    if secrets.holds_serialized_secret(&serde_json::to_vec(&value)?) {
        return refusal_envelope();
    }
    let sealed = serde_json::to_vec_pretty(&value)?;
    Ok(sealed)
}

fn refusal_envelope() -> WorkflowResult<Vec<u8>> {
    Ok(serde_json::to_vec_pretty(&GateEnvelopeV1 {
        schema_version: 1,
        report: serde_json::Value::Null,
        policy_findings: Vec::new(),
        operational_error: Some(archon_workflow::GateOperationalError {
            kind: "secret_redaction_refused".into(),
            text: "Host evidence could not be safely redacted; update the forwarded credentials and retry".into(),
        }),
    })?)
}

fn remove_temporary_envelopes(root: &Path) -> WorkflowResult<()> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(WorkflowError::Io {
                path: root.into(),
                source,
            });
        }
    };
    for entry in entries {
        let entry = entry.map_err(|source| WorkflowError::Io {
            path: root.into(),
            source,
        })?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|source| WorkflowError::Io {
            path: path.clone(),
            source,
        })?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !kind.is_dir() && name.starts_with("gate-envelope.") && name.ends_with(".tmp") {
            std::fs::remove_file(&path).map_err(|source| WorkflowError::Io { path, source })?;
        } else if kind.is_dir() {
            remove_temporary_envelopes(&path)?;
        }
    }
    Ok(())
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
