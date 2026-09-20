use super::*;

pub(super) fn registry_backup_path(data_root: &Path, timestamp: &str) -> PathBuf {
    data_root.join(format!("registry.json.backup-{}", safe_path(timestamp)))
}

pub(super) fn latest_registry_backup(data_root: &Path) -> Result<Option<String>, DataStoreError> {
    if !data_root.exists() {
        return Ok(None);
    }
    let mut backups = Vec::new();
    for entry in std::fs::read_dir(data_root).map_err(io_error)? {
        let path = entry.map_err(io_error)?.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("registry.json.backup-"))
        {
            backups.push(path.to_string_lossy().to_string());
        }
    }
    backups.sort();
    Ok(backups.pop())
}

struct VerifiedArtifacts {
    metadata: DatasetMetadata,
    metadata_was_quarantined: bool,
    validation: ValidationReport,
    manifest: StoredDatasetRecord,
    normalized_sha256: String,
    raw_sha256: String,
}

impl VerifiedArtifacts {
    fn read(root: &Path, record: &StoredDatasetRecord) -> Result<Self, DataStoreError> {
        let metadata_read = read_dataset_metadata_with_quarantine(root, record)?;
        let validation = read_json(&root.join(&record.validation_path))?;
        let manifest = read_stored_dataset_record(&root.join(&record.manifest_path))?;
        let normalized = std::fs::read(root.join(&record.normalized_path)).map_err(io_error)?;
        let raw = std::fs::read(root.join(&record.raw_response_path)).map_err(io_error)?;
        Ok(Self {
            metadata: metadata_read.metadata,
            metadata_was_quarantined: metadata_read.quarantined,
            validation,
            manifest,
            normalized_sha256: bytes_checksum(&normalized),
            raw_sha256: bytes_checksum(&raw),
        })
    }
}

pub(super) fn verify_artifacts(
    root: &Path,
    record: &StoredDatasetRecord,
) -> Result<(), DataStoreError> {
    for path in [
        record.metadata_path.clone(),
        record.normalized_path.clone(),
        record.raw_path.clone(),
        record.raw_response_path.clone(),
        record.raw_request_path.clone(),
        record.redacted_headers_path.clone(),
        record.provider_notes_path.clone(),
        record.validation_path.clone(),
        record.manifest_path.clone(),
    ]
    .into_iter()
    .chain(required_raw_artifact_paths(record))
    {
        if path.trim().is_empty() {
            return Err(DataStoreError::IncompleteArtifactContract(path));
        }
        if !root.join(&path).exists() {
            return Err(DataStoreError::IncompleteArtifactContract(path));
        }
    }
    if record.dataset_path.trim().is_empty()
        || record.metadata_checksum.trim().is_empty()
        || record.raw_checksum.trim().is_empty()
        || record.validation_checksum.trim().is_empty()
    {
        return Err(DataStoreError::IncompleteArtifactContract(
            record.dataset_id.clone(),
        ));
    }
    let artifacts = VerifiedArtifacts::read(root, record)?;
    verify_checksum_chain(record, &artifacts)?;
    Ok(())
}

pub(super) fn read_dataset_metadata(
    root: &Path,
    record: &StoredDatasetRecord,
) -> Result<DatasetMetadata, DataStoreError> {
    read_dataset_metadata_with_quarantine(root, record).map(|metadata| metadata.metadata)
}

fn read_stored_dataset_record(path: &Path) -> Result<StoredDatasetRecord, DataStoreError> {
    match read_json(path) {
        Ok(record) => Ok(record),
        Err(error) if json_unknown_field(&error, "remediation_note") => {
            read_record_without_workflow_note_fields(path)
        }
        Err(error) if json_unknown_field(&error, "quality_status") => {
            read_record_without_workflow_note_fields(path)
        }
        Err(error) => Err(error),
    }
}

fn read_record_without_workflow_note_fields(
    path: &Path,
) -> Result<StoredDatasetRecord, DataStoreError> {
    let mut value = read_json::<serde_json::Value>(path)?;
    let Some(object) = value.as_object_mut() else {
        return Err(DataStoreError::Json("record must be a JSON object".into()));
    };
    object.remove("quality_status");
    object.remove("remediation_note");
    serde_json::from_value(value).map_err(|err| DataStoreError::Json(err.to_string()))
}

/// Whether this dataset's metadata carries a quarantine marker.
///
/// Quarantine lives in the metadata, but registry status is DERIVED from the
/// validation report — so a quarantined dataset whose validation.json still
/// claimed `passed` was being stamped `Healthy` on every load. Thirty-three of
/// them were, which is the latent false-success a quarantine exists to prevent:
/// the operator marks a dataset untrustworthy and the next read promotes it
/// back.
pub(super) fn dataset_is_quarantined(root: &Path, record: &StoredDatasetRecord) -> bool {
    read_dataset_metadata_with_quarantine(root, record)
        .map(|read| read.quarantined)
        .unwrap_or(false)
}

fn read_dataset_metadata_with_quarantine(
    root: &Path,
    record: &StoredDatasetRecord,
) -> Result<DatasetMetadataRead, DataStoreError> {
    let path = root.join(&record.metadata_path);
    match read_json(&path) {
        Ok(metadata) => Ok(DatasetMetadataRead {
            metadata,
            quarantined: false,
        }),
        Err(error) if json_unknown_field(&error, "quarantined_at") => {
            read_metadata_without_workflow_note_fields(&path, true)
        }
        Err(error) if json_unknown_field(&error, "quarantine_reason") => {
            read_metadata_without_workflow_note_fields(&path, true)
        }
        Err(error) if json_unknown_field(&error, "remediation_note") => {
            read_metadata_without_workflow_note_fields(&path, false)
        }
        Err(error) => Err(error),
    }
}

struct DatasetMetadataRead {
    metadata: DatasetMetadata,
    quarantined: bool,
}

fn read_metadata_without_workflow_note_fields(
    path: &Path,
    quarantined: bool,
) -> Result<DatasetMetadataRead, DataStoreError> {
    let mut value = read_json::<serde_json::Value>(path)?;
    let Some(object) = value.as_object_mut() else {
        return Err(DataStoreError::Json(
            "metadata must be a JSON object".into(),
        ));
    };
    object.remove("quarantined_at");
    object.remove("quarantine_reason");
    object.remove("remediation_note");
    Ok(DatasetMetadataRead {
        metadata: serde_json::from_value(value)
            .map_err(|err| DataStoreError::Json(err.to_string()))?,
        quarantined,
    })
}

fn json_unknown_field(error: &DataStoreError, field: &str) -> bool {
    match error {
        DataStoreError::Json(message) => {
            message.contains("unknown field") && message.contains(field)
        }
        _ => false,
    }
}

fn verify_checksum_chain(
    record: &StoredDatasetRecord,
    artifacts: &VerifiedArtifacts,
) -> Result<(), DataStoreError> {
    let metadata_sha256 = metadata_sha256(&artifacts.metadata)?;
    let validation_sha256 =
        ValidationReport::content_hash(&artifacts.normalized_sha256, &artifacts.validation.checks);
    // Each link is named so a failure says which one broke. The message used to
    // carry only `dataset_id:version`, which told an operator — or a remediation
    // agent — nothing about whether to look at the data, the metadata, the
    // validation report or the manifest. Diagnosing one live mismatch meant
    // hand-hashing every file to find that `ohlcv.jsonl` disagreed with all
    // three recorded copies of its checksum.
    let checks: [(&str, bool); 14] = [
        (
            "manifest record differs from manifest.json on disk",
            artifacts.metadata_was_quarantined || record == &artifacts.manifest,
        ),
        (
            "manifest.dataset_id != metadata.dataset_id",
            record.dataset_id == artifacts.metadata.dataset_id,
        ),
        (
            "manifest.version != metadata.version",
            record.version == artifacts.metadata.version,
        ),
        (
            "validation.dataset_id != manifest.dataset_id",
            artifacts.validation.dataset_id == record.dataset_id,
        ),
        (
            "validation.version != manifest.version",
            artifacts.validation.version == record.version,
        ),
        (
            "manifest.checksum != actual normalized data hash",
            artifacts.metadata_was_quarantined || record.checksum == artifacts.normalized_sha256,
        ),
        (
            "metadata.checksum != actual normalized data hash",
            artifacts.metadata_was_quarantined
                || artifacts.metadata.checksum == artifacts.normalized_sha256,
        ),
        (
            "metadata.checksums.normalized_sha256 != actual normalized data hash",
            artifacts.metadata_was_quarantined
                || artifacts.metadata.checksums.normalized_sha256 == artifacts.normalized_sha256,
        ),
        (
            "manifest.raw_checksum != actual raw hash",
            artifacts.metadata_was_quarantined || record.raw_checksum == artifacts.raw_sha256,
        ),
        (
            "metadata.checksums.raw_sha256 != actual raw hash",
            artifacts.metadata_was_quarantined
                || artifacts.metadata.checksums.raw_sha256 == artifacts.raw_sha256,
        ),
        (
            "manifest.metadata_checksum != actual metadata hash",
            artifacts.metadata_was_quarantined || record.metadata_checksum == metadata_sha256,
        ),
        (
            "metadata.checksums.metadata_sha256 != actual metadata hash",
            artifacts.metadata_was_quarantined
                || artifacts.metadata.checksums.metadata_sha256 == metadata_sha256,
        ),
        (
            "manifest.validation_checksum != recomputed validation hash",
            artifacts.metadata_was_quarantined || record.validation_checksum == validation_sha256,
        ),
        (
            "validation.content_sha256 != recomputed validation hash",
            artifacts.metadata_was_quarantined
                || artifacts.validation.content_sha256 == validation_sha256,
        ),
    ];
    let broken: Vec<&str> = checks
        .into_iter()
        .filter(|(_, valid)| !valid)
        .map(|(label, _)| label)
        .collect();
    if broken.is_empty() {
        Ok(())
    } else {
        Err(checksum_chain_mismatch(record, &broken))
    }
}

fn checksum_chain_mismatch(record: &StoredDatasetRecord, broken: &[&str]) -> DataStoreError {
    DataStoreError::IncompleteArtifactContract(format!(
        "checksum chain mismatch for {}:{} ({} of 14 links broken: {})",
        record.dataset_id,
        record.version,
        broken.len(),
        broken.join("; ")
    ))
}

pub(super) fn project_root_for_artifact(path: &Path) -> Result<PathBuf, DataStoreError> {
    path.ancestors()
        .find(|ancestor| ancestor.file_name().is_some_and(|name| name == ".archon"))
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or(DataStoreError::InvalidPath)
}

pub(super) fn required_raw_artifact_paths(record: &StoredDatasetRecord) -> Vec<String> {
    [
        raw_response_path(record),
        "request.json",
        "headers.redacted.json",
        "provider-notes.md",
    ]
    .into_iter()
    .map(|file| dataset_raw_path(record, file))
    .collect()
}

fn raw_response_path(record: &StoredDatasetRecord) -> &'static str {
    if record.raw_path.ends_with(".csv") {
        "response.csv"
    } else if record.raw_path.ends_with(".zip") {
        "response.zip"
    } else if record.raw_path.ends_with(".txt") {
        "response.txt"
    } else {
        "response.json"
    }
}

pub(super) fn dataset_raw_path(record: &StoredDatasetRecord, raw_file: &str) -> String {
    let Some((dir, _)) = record.raw_path.rsplit_once('/') else {
        return raw_file.into();
    };
    format!("{dir}/{raw_file}")
}

pub(super) fn capability_key(provider: &str, symbol: &str, timeframe: &str) -> String {
    format!(
        "{}:{}:{}",
        provider.trim().to_ascii_lowercase(),
        symbol.trim(),
        timeframe.trim()
    )
}

pub(super) fn registry_key(dataset_id: &str, version: &str) -> String {
    format!("{dataset_id}:{version}")
}

pub(super) fn raw_filename(format: OhlcvFormat) -> &'static str {
    match format {
        OhlcvFormat::Csv => "response.csv",
        OhlcvFormat::Json => "response.json",
        OhlcvFormat::Txt => "response.txt",
        OhlcvFormat::Zip => "response.zip",
    }
}

pub(super) fn safe_path(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Stored dataset paths, always `/`-separated regardless of host.
///
/// `to_string_lossy` alone emits platform-native separators, so on Windows
/// every path in a registry record came back with backslashes while every
/// consumer split it on `/` and rebuilt it with `/` — `dataset_raw_path` and
/// the two `metadata_path` splits in `migration.rs`. Those splits then cut at
/// the wrong component and produced paths that do not exist, surfacing as
/// `IncompleteArtifactContract` on ~64 Windows tests.
///
/// Joining components explicitly rather than string-replacing `\` keeps this
/// correct on Unix, where a backslash is a legal character *inside* a file
/// name and must not be treated as a separator. `Path::join` accepts
/// `/`-separated input on Windows, so reading these back works on both.
///
/// It also makes the on-disk records portable: a data lake written on one
/// platform now reads identically on the other.
pub(super) fn relative(root: &Path, path: &Path) -> Result<String, DataStoreError> {
    path.strip_prefix(root)
        .map(|relative| {
            relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        })
        .map_err(|_| DataStoreError::InvalidPath)
}

pub(super) fn io_error(error: std::io::Error) -> DataStoreError {
    DataStoreError::Io(error.to_string())
}

pub(super) fn contains_secret_material(value: &serde_json::Value) -> bool {
    let text = value.to_string().to_ascii_lowercase();
    [
        "secret",
        "token",
        "api_key",
        "apikey",
        "authorization",
        "bearer ",
        "cookie",
        "password",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}
