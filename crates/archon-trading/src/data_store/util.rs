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
    {
        return Err(DataStoreError::IncompleteArtifactContract(
            record.dataset_id.clone(),
        ));
    }
    Ok(())
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

pub(super) fn relative(root: &Path, path: &Path) -> Result<String, DataStoreError> {
    path.strip_prefix(root)
        .map(|path| path.to_string_lossy().to_string())
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
