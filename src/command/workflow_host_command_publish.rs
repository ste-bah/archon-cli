//! Parent-only audit and publication of trusted child staging output.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_workflow::task_set_contract::content_digest;
use archon_workflow::{
    PREPARED_PUBLICATION_SCHEMA_VERSION, PUBLICATION_RECEIPT_SCHEMA_VERSION, PreparedPublicationV1,
    PublicationReceiptV1, PublishedArtifactReceipt,
};

use super::workflow_host_command_catalog::ResolvedHostCommand;

#[derive(Debug, Clone)]
pub(crate) struct CommandStaging {
    pub(crate) root: PathBuf,
    pub(crate) call_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SentinelValue {
    path: PathBuf,
    digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LiveMutationSentinels {
    values: Vec<SentinelValue>,
}

impl LiveMutationSentinels {
    pub(crate) fn capture(paths: &[PathBuf]) -> Result<Self> {
        let mut values = Vec::with_capacity(paths.len());
        for path in paths {
            values.push(SentinelValue {
                path: path.clone(),
                digest: path_digest(path)?,
            });
        }
        Ok(Self { values })
    }

    fn verify(&self) -> Result<()> {
        for expected in &self.values {
            let actual = path_digest(&expected.path)?;
            if actual != expected.digest {
                return Err(anyhow!(
                    "mutation sentinel changed for {}: expected {:?}, actual {:?}",
                    expected.path.display(),
                    expected.digest,
                    actual
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
struct AuditedEntry {
    relative_path: String,
    source_path: PathBuf,
    bytes: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct AuditedPublication {
    call_id: String,
    command_id: String,
    entries: Vec<AuditedEntry>,
    sentinels: LiveMutationSentinels,
}

pub(crate) fn prepare_staging(run_root: &Path, call_id: &str) -> Result<CommandStaging> {
    let safe = sanitize_component(call_id);
    let root = run_root.join("host-command-staging").join(safe);
    if root.exists() {
        std::fs::remove_dir_all(&root)
            .with_context(|| format!("clearing stale host command staging {}", root.display()))?;
    }
    std::fs::create_dir_all(&root)
        .with_context(|| format!("creating host command staging {}", root.display()))?;
    Ok(CommandStaging {
        root,
        call_id: call_id.to_string(),
    })
}

pub(crate) fn audit_prepared_publication(
    staging: &CommandStaging,
    prepared: &PreparedPublicationV1,
    command: &ResolvedHostCommand,
    sentinels: LiveMutationSentinels,
) -> Result<AuditedPublication> {
    if prepared.schema_version != PREPARED_PUBLICATION_SCHEMA_VERSION {
        return Err(anyhow!(
            "prepared publication schema version {} is unsupported",
            prepared.schema_version
        ));
    }
    if prepared.call_id != staging.call_id {
        return Err(anyhow!(
            "prepared publication call id '{}' does not match staging call '{}'",
            prepared.call_id,
            staging.call_id
        ));
    }
    if prepared.command_id != command.command_id {
        return Err(anyhow!(
            "prepared publication command id '{}' does not match resolved command '{}'",
            prepared.command_id,
            command.command_id
        ));
    }
    sentinels.verify()?;

    let actual = staged_files(&staging.root)?;
    let declared = declared_relative_paths(&staging.root, &command.declared_write_set)?;
    let manifest = prepared
        .entries
        .iter()
        .map(|entry| normalize_relative(&entry.relative_path))
        .collect::<Result<BTreeSet<_>>>()?;
    if actual != declared || actual != manifest {
        return Err(anyhow!(
            "staged tree does not exactly match declared write set and manifest: actual={actual:?}, declared={declared:?}, manifest={manifest:?}"
        ));
    }
    if manifest.len() != prepared.entries.len() {
        return Err(anyhow!("prepared publication contains duplicate paths"));
    }

    let by_path = prepared
        .entries
        .iter()
        .map(|entry| Ok((normalize_relative(&entry.relative_path)?, entry)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let mut entries = Vec::with_capacity(actual.len());
    for relative_path in actual {
        let source_path = staging.root.join(&relative_path);
        let metadata = std::fs::symlink_metadata(&source_path)
            .with_context(|| format!("reading staged metadata {}", source_path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(anyhow!(
                "staged output {} is a symlink",
                source_path.display()
            ));
        }
        if !metadata.is_file() {
            return Err(anyhow!(
                "staged output {} is not a regular file",
                source_path.display()
            ));
        }
        let bytes = std::fs::read(&source_path)
            .with_context(|| format!("reading staged output {}", source_path.display()))?;
        let expected = by_path
            .get(&relative_path)
            .expect("set equality established manifest entry");
        if expected.byte_len != bytes.len() as u64 {
            return Err(anyhow!(
                "staged output {} byte length mismatch: manifest {}, actual {}",
                relative_path,
                expected.byte_len,
                bytes.len()
            ));
        }
        let digest = content_digest(&bytes);
        if expected.blake3 != digest {
            return Err(anyhow!(
                "staged output {} digest mismatch: manifest {}, actual {}",
                relative_path,
                expected.blake3,
                digest
            ));
        }
        entries.push(AuditedEntry {
            relative_path,
            source_path,
            bytes,
        });
    }
    Ok(AuditedPublication {
        call_id: prepared.call_id.clone(),
        command_id: prepared.command_id.clone(),
        entries,
        sentinels,
    })
}

pub(crate) fn publish_audited(
    audited: AuditedPublication,
    destinations: &BTreeMap<String, PathBuf>,
) -> Result<PublicationReceiptV1> {
    audited.sentinels.verify()?;
    let expected = audited
        .entries
        .iter()
        .map(|entry| entry.relative_path.clone())
        .collect::<BTreeSet<_>>();
    let actual = destinations.keys().cloned().collect::<BTreeSet<_>>();
    if expected != actual {
        return Err(anyhow!(
            "publication destinations do not exactly match audited output: expected={expected:?}, actual={actual:?}"
        ));
    }

    let mut prior = BTreeMap::new();
    let mut files = Vec::with_capacity(audited.entries.len());
    for entry in &audited.entries {
        let destination = destinations
            .get(&entry.relative_path)
            .expect("destination set equality established");
        prior.insert(entry.relative_path.clone(), path_digest(destination)?);
        files.push((destination.clone(), entry.bytes.clone()));
    }
    // Reuse the freeze publisher's tested rollback behaviour. The child never
    // calls this; parent-only audit is complete before this point.
    super::workflow_task_set::publish_files_atomically(
        &files,
        "restart the host command phase after inspecting publication state",
    )?;

    let mut receipts = Vec::with_capacity(audited.entries.len());
    for entry in audited.entries {
        let destination = destinations
            .get(&entry.relative_path)
            .expect("destination set equality established");
        let bytes = std::fs::read(destination)
            .with_context(|| format!("reading published output {}", destination.display()))?;
        let live_digest = content_digest(&bytes);
        let expected_digest = content_digest(&entry.bytes);
        if live_digest != expected_digest {
            return Err(anyhow!(
                "published output {} does not match audited bytes",
                destination.display()
            ));
        }
        let _ = std::fs::remove_file(&entry.source_path);
        receipts.push(PublishedArtifactReceipt {
            relative_path: entry.relative_path.clone(),
            destination_path: destination.to_string_lossy().replace('\\', "/"),
            byte_len: bytes.len() as u64,
            blake3: live_digest,
            prior_blake3: prior.remove(&entry.relative_path).flatten(),
        });
    }
    receipts.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(PublicationReceiptV1 {
        schema_version: PUBLICATION_RECEIPT_SCHEMA_VERSION,
        call_id: audited.call_id,
        command_id: audited.command_id,
        entries: receipts,
        committed_at: chrono::Utc::now().to_rfc3339(),
    })
}

fn staged_files(root: &Path) -> Result<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    collect_files(root, root, &mut out)?;
    Ok(out)
}

fn collect_files(root: &Path, current: &Path, out: &mut BTreeSet<String>) -> Result<()> {
    for entry in std::fs::read_dir(current)
        .with_context(|| format!("enumerating staged tree {}", current.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(anyhow!("staged tree contains symlink {}", path.display()));
        }
        if metadata.is_dir() {
            collect_files(root, &path, out)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .expect("enumerated path stays under staging root");
            out.insert(path_text(relative));
        } else {
            return Err(anyhow!(
                "staged tree contains unsupported file type {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn declared_relative_paths(root: &Path, paths: &[PathBuf]) -> Result<BTreeSet<String>> {
    paths
        .iter()
        .map(|path| {
            let relative = path.strip_prefix(root).map_err(|_| {
                anyhow!(
                    "declared staged output {} escapes staging root {}",
                    path.display(),
                    root.display()
                )
            })?;
            normalize_relative(&path_text(relative))
        })
        .collect()
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
    Ok(path_text(path))
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn path_digest(path: &Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(anyhow!("mutation sentinel {} is a symlink", path.display()));
    }
    if !metadata.is_file() {
        return Err(anyhow!(
            "mutation sentinel {} is not a regular file",
            path.display()
        ));
    }
    Ok(Some(content_digest(&std::fs::read(path)?)))
}

fn sanitize_component(value: &str) -> String {
    let value = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    if value.is_empty() {
        "call".to_string()
    } else {
        value
    }
}
