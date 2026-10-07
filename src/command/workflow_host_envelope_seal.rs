//! Decode, redact and verify staged host envelopes before publication (#277).
//! A manifest must describe the child's raw bytes; only an honest entry can
//! be rebound to the sealed bytes. Temporary envelopes are removed on exit.
//! Every read and rewrite goes through the call's staging anchor (#297).
use std::path::Path;

use archon_workflow::task_set_contract::content_digest;
use archon_workflow::{GateEnvelopeV1, PreparedPublicationV1, WorkflowError, WorkflowResult};

use super::workflow_host_secrets::HostSecrets;
use super::workflow_host_staging_anchor::StagingAnchor;
use super::workflow_host_staging_pause::StagingPause;

pub(crate) const ENVELOPE_FILE: &str = "gate-envelope.json";

/// Covers early returns and cancellation of the executor's future too. A
/// failure on cancellation is made durable by the pause sealing records.
pub(crate) struct EnvelopeCleanup<'a> {
    pub(crate) anchor: &'a StagingAnchor,
    pub(crate) secrets: &'a HostSecrets,
    pub(crate) pause: &'a StagingPause,
    pub(crate) armed: bool,
}

impl EnvelopeCleanup<'_> {
    pub(crate) fn finish(mut self) -> WorkflowResult<()> {
        self.armed = false;
        self.secrets
            .seal_staged_evidence(self.anchor, None, self.pause)
    }
}

impl Drop for EnvelopeCleanup<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // Sealing pauses the run (or, when the run is no longer this call's,
        // records the residue) before it returns; nothing is left to decide.
        if let Err(error) = self
            .secrets
            .seal_staged_evidence(self.anchor, None, self.pause)
        {
            let evidence = self.secrets.text(&error.to_string());
            tracing::warn!(error = %evidence, "sealing a cancelled call's staging failed");
        }
    }
}

fn io_at(path: &Path) -> impl Fn(std::io::Error) -> WorkflowError + '_ {
    move |source| WorkflowError::Io {
        path: path.to_path_buf(),
        source,
    }
}

pub(crate) fn seal_staged_envelope(
    anchor: &StagingAnchor,
    secrets: &HostSecrets,
    mut prepared: Option<&mut PreparedPublicationV1>,
) -> WorkflowResult<()> {
    let path = anchor.root().join(ENVELOPE_FILE);
    let io = io_at(&path);
    remove_temporary_envelopes(anchor)?;
    // Never follow a link the child left. The publication audit refuses it.
    let Some(raw) = anchor.read_file(ENVELOPE_FILE).map_err(&io)? else {
        return Ok(());
    };
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
        anchor.write_file(ENVELOPE_FILE, &sealed).map_err(&io)?;
    } else {
        anchor.owner_only(ENVELOPE_FILE).map_err(&io)?;
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
        // Persist only the verified typed value. Parsing may discard duplicate
        // keys and normalize numeric tokens even when redaction changed nothing.
        return Ok(serde_json::to_vec_pretty(&verified)?);
    }
    // A redaction refusal is valid operational evidence, never JSON null.
    if secrets.holds_serialized_secret(&serde_json::to_vec(&value)?) {
        return refusal_envelope();
    }
    let sealed = serde_json::to_vec_pretty(&value)?;
    Ok(sealed)
}

pub(crate) fn refuse_staged_evidence(
    anchor: &StagingAnchor,
    prepared: Option<&mut PreparedPublicationV1>,
) -> WorkflowResult<()> {
    let bytes = refusal_envelope()?;
    let path = anchor.root().join(ENVELOPE_FILE);
    // Whatever the child left under the name is replaced, never written through.
    anchor
        .write_file(ENVELOPE_FILE, &bytes)
        .map_err(io_at(&path))?;
    if let Some(prepared) = prepared {
        for entry in &mut prepared.entries {
            if entry.relative_path == ENVELOPE_FILE {
                (entry.byte_len, entry.blake3) = identity(&bytes);
            }
        }
    }
    Ok(())
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

fn remove_temporary_envelopes(anchor: &StagingAnchor) -> WorkflowResult<()> {
    anchor
        .scan(false, &mut |relative, _| {
            relative.file_name().is_some_and(|name| {
                let name = name.to_string_lossy();
                name.starts_with("gate-envelope.") && name.ends_with(".tmp")
            })
        })
        .map(drop)
        .map_err(io_at(anchor.root()))
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

fn identity(bytes: &[u8]) -> (u64, String) {
    (bytes.len() as u64, content_digest(bytes))
}
