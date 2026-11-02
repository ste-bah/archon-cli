//! Parent-owned commit evidence, published in the artifact transaction itself.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::workflow_host_command_catalog::HostCommandResolutionContext;
use archon_workflow::{PublicationReceiptV1, WorkflowError, WorkflowResult};

pub(crate) const RECEIPT_FILE: &str = "publication-receipt.json";

/// A journal rename committed the transaction even though its flush failed.
#[derive(Debug)]
pub(crate) struct CommitRecorded;
impl std::fmt::Display for CommitRecorded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("publication commit was recorded; observe its committed receipt")
    }
}
impl std::error::Error for CommitRecorded {}

/// The admission that may recover this transaction, independent of content identity.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Claim {
    call_id: String,
    input_hash: String,
    admission_sequence: u64,
}
impl Claim {
    pub(crate) fn of(record: archon_workflow::WorkflowV2CallRecord) -> Option<Self> {
        Some(Self {
            call_id: record.call.id,
            input_hash: record.input_hash,
            admission_sequence: record.admission_sequence?,
        })
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Committed {
    pub(crate) receipt: PublicationReceiptV1,
    pub(crate) claim: Option<Claim>,
}

pub(crate) fn path(envelope: &Path) -> PathBuf {
    envelope.with_file_name(RECEIPT_FILE)
}

pub(crate) fn load(
    run_root: &Path,
    context: &HostCommandResolutionContext,
    call_id: &str,
    command_id: &str,
    claim: Option<&Claim>,
) -> WorkflowResult<Option<PublicationReceiptV1>> {
    let Some(current) = claim else {
        return Ok(None);
    };
    let marker = run_root
        .join("host-command-results")
        .join(call_id)
        .join(RECEIPT_FILE);
    // Settle first: an applying journal may roll the marker back with its artifacts.
    let _read = super::workflow_task_set::ChainRead::of(&context.project_root, &context.task_root)
        .map_err(|error| {
            super::workflow_host_fault::workflow(
                error,
                format!(
                    "settling publication before reading committed receipt {}",
                    marker.display()
                ),
            )
        })?;
    let metadata = match std::fs::symlink_metadata(&marker) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(WorkflowError::Io {
                path: marker,
                source,
            });
        }
        Ok(metadata) => metadata,
    };
    let corrupt = |cause: String| {
        WorkflowError::StateCorrupt(format!(
            "committed receipt {} {cause}; restore this receipt from the authenticated publication record",
            marker.display(),
        ))
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(corrupt("is not a regular parent-owned file".into()));
    }
    let store = archon_workflow::WorkflowStore::project(&context.project_root);
    let authority = store
        .root()
        .parent()
        .ok_or_else(|| corrupt("has no store authority".into()))?;
    super::workflow_task_set::validate_existing_parents(&marker, authority).map_err(|error| {
        WorkflowError::port(
            error
                .context(format!(
                    "committed receipt {} has invalid authority; restore its authenticated path",
                    marker.display()
                ))
                .into_boxed_dyn_error(),
        )
    })?;
    let bytes = std::fs::read(&marker).map_err(|source| WorkflowError::Io {
        path: marker.clone(),
        source,
    })?;
    let committed: Committed = serde_json::from_slice(&bytes)
        .map_err(|error| corrupt(format!("is malformed JSON: {error}")))?;
    let receipt = committed.receipt;
    let Some(prior) = committed.claim else {
        return Ok(None);
    };
    if current.call_id != prior.call_id || current.input_hash != prior.input_hash {
        return Ok(None);
    }
    let results = archon_workflow::WorkflowV2ResultStore::new(run_root.join("v2"));
    if !results.admission_needs_publication_observation(
        &prior.call_id,
        &prior.input_hash,
        prior.admission_sequence,
    )? {
        return Ok(None);
    }
    if receipt.schema_version != archon_workflow::PUBLICATION_RECEIPT_SCHEMA_VERSION
        || receipt.call_id != call_id
        || receipt.command_id != command_id
    {
        return Err(corrupt("does not match its capability identity".into()));
    }
    Ok(Some(receipt))
}

pub(crate) fn validate_destinations(
    receipt: &PublicationReceiptV1,
    destinations: &BTreeMap<String, PathBuf>,
) -> WorkflowResult<()> {
    use super::workflow_host_command_publish::{path_comparison_key, path_text};
    let distinct = receipt
        .entries
        .iter()
        .map(|entry| &entry.relative_path)
        .collect::<std::collections::BTreeSet<_>>();
    if distinct.len() != receipt.entries.len()
        || receipt.entries.len() != destinations.len()
        || receipt.entries.iter().any(|entry| {
            destinations.get(&entry.relative_path).is_none_or(|path| {
                path_comparison_key(&path_text(path))
                    != path_comparison_key(&entry.destination_path)
            })
        })
    {
        return Err(WorkflowError::StateCorrupt(format!(
            "committed receipt {} has incompatible destinations; restore its authenticated publication metadata",
            receipt.call_id,
        )));
    }
    Ok(())
}
