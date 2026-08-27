//! Host-owned serialization for staged gate outputs.
//!
//! Trusted child modes write only into a run-owned staging root. They return a
//! provisional manifest to the parent, which separately audits and publishes
//! exact bytes; this module never writes a live destination or receipt.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Component, Path};

use anyhow::{Context, Result, anyhow};
use archon_workflow::{
    PREPARED_PUBLICATION_SCHEMA_VERSION, PreparedPublicationEntry, PreparedPublicationV1,
};

use super::workflow_gate::GateEvaluation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StagedGateOutput {
    pub(crate) relative_path: String,
    pub(crate) bytes: Vec<u8>,
}

pub(crate) fn stage_gate_evaluation(
    staging_root: &Path,
    envelope_path: &Path,
    call_id: &str,
    command_id: &str,
    evaluation: GateEvaluation,
    outputs: Vec<StagedGateOutput>,
) -> Result<PreparedPublicationV1> {
    if call_id.trim().is_empty() || command_id.trim().is_empty() {
        return Err(anyhow!(
            "staged gate call and command ids must be non-empty"
        ));
    }
    let envelope_relative = relative_under(staging_root, envelope_path)?;
    let mut seen = BTreeSet::new();
    seen.insert(envelope_relative.clone());
    for output in &outputs {
        let normalized = normalize_relative(&output.relative_path)?;
        if !seen.insert(normalized) {
            return Err(anyhow!(
                "staged gate output contains duplicate path {}",
                output.relative_path
            ));
        }
    }

    let envelope = evaluation.into_envelope()?;
    let envelope_bytes = serde_json::to_vec_pretty(&envelope)?;
    let mut staged = Vec::with_capacity(outputs.len() + 1);
    staged.push(StagedGateOutput {
        relative_path: envelope_relative,
        bytes: envelope_bytes,
    });
    for mut output in outputs {
        output.relative_path = normalize_relative(&output.relative_path)?;
        staged.push(output);
    }
    staged.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    std::fs::create_dir_all(staging_root)
        .with_context(|| format!("creating staged gate root {}", staging_root.display()))?;
    for output in &staged {
        write_staged_atomic(staging_root, output)?;
    }
    let entries = staged
        .into_iter()
        .map(|output| PreparedPublicationEntry {
            relative_path: output.relative_path,
            byte_len: output.bytes.len() as u64,
            blake3: archon_workflow::task_set_contract::content_digest(&output.bytes),
        })
        .collect();
    Ok(PreparedPublicationV1 {
        schema_version: PREPARED_PUBLICATION_SCHEMA_VERSION,
        call_id: call_id.to_string(),
        command_id: command_id.to_string(),
        entries,
    })
}

fn write_staged_atomic(root: &Path, output: &StagedGateOutput) -> Result<()> {
    let target = root.join(&output.relative_path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating staged output directory {}", parent.display()))?;
    }
    let temporary = target.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    let mut file = std::fs::File::create(&temporary)
        .with_context(|| format!("creating staged output {}", temporary.display()))?;
    file.write_all(&output.bytes)
        .with_context(|| format!("writing staged output {}", temporary.display()))?;
    file.sync_all()
        .with_context(|| format!("syncing staged output {}", temporary.display()))?;
    std::fs::rename(&temporary, &target).with_context(|| {
        format!(
            "publishing staged output {} to {}",
            temporary.display(),
            target.display()
        )
    })
}

fn relative_under(root: &Path, path: &Path) -> Result<String> {
    let relative = path.strip_prefix(root).map_err(|_| {
        anyhow!(
            "staged gate envelope {} escapes staging root {}",
            path.display(),
            root.display()
        )
    })?;
    normalize_relative(&relative.to_string_lossy())
}

fn normalize_relative(raw: &str) -> Result<String> {
    let path = Path::new(raw);
    if raw.trim().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir
                    | Component::CurDir
                    | Component::RootDir
                    | Component::Prefix(_)
            )
        })
    {
        return Err(anyhow!("invalid staged relative path {raw:?}"));
    }
    Ok(path.to_string_lossy().replace('\\', "/"))
}
