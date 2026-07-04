use super::*;

pub(super) fn record(
    root: &Path,
    versioned: &VersionedDataset,
    bars: &[OhlcvBar],
    paths: ArtifactPaths<'_>,
    created_at: String,
) -> Result<StoredDatasetRecord, DataStoreError> {
    Ok(StoredDatasetRecord {
        dataset_id: versioned.metadata.dataset_id.clone(),
        version: versioned.metadata.version.clone(),
        schema_version: registry_contract_schema(),
        dataset_path: relative(
            root,
            paths.metadata.parent().ok_or(DataStoreError::InvalidPath)?,
        )?,
        metadata_checksum: versioned.metadata.checksums.metadata_sha256.clone(),
        raw_checksum: versioned.metadata.checksums.raw_sha256.clone(),
        raw_response_path: relative(root, paths.raw)?,
        raw_request_path: dataset_raw_artifact_path(root, paths.raw, "request.json")?,
        redacted_headers_path: dataset_raw_artifact_path(root, paths.raw, "headers.redacted.json")?,
        provider_notes_path: dataset_raw_artifact_path(root, paths.raw, "provider-notes.md")?,
        provider: versioned.metadata.provider.clone(),
        data_type: format!("{:?}", versioned.metadata.data_type),
        symbol: versioned.metadata.canonical_instrument.clone(),
        timeframe: versioned.metadata.timeframe.clone(),
        native_interval: versioned.metadata.native_interval,
        production_eligible: versioned.metadata.production_eligible,
        status: versioned.status,
        checksum: versioned.metadata.checksum.clone(),
        bars: bars.len(),
        coverage_start: versioned.metadata.coverage.start.clone(),
        coverage_end: versioned.metadata.coverage.end.clone(),
        metadata_path: relative(root, paths.metadata)?,
        normalized_path: relative(root, paths.normalized)?,
        raw_path: relative(root, paths.raw)?,
        validation_path: relative(root, paths.validation)?,
        manifest_path: relative(root, paths.manifest)?,
        created_at,
    })
}
pub(super) fn enrich_metadata_artifacts(
    root: &Path,
    metadata: &mut DatasetMetadata,
    raw_body: &[u8],
    normalized_path: &Path,
    raw_path: &Path,
    validation_path: &Path,
    manifest_path: &Path,
    created_at: &str,
) -> Result<(), DataStoreError> {
    let normalized = std::fs::read(normalized_path).map_err(io_error)?;
    metadata.checksums = DatasetChecksums {
        raw_sha256: bytes_checksum(raw_body),
        normalized_sha256: bytes_checksum(&normalized),
        metadata_sha256: String::new(),
    };
    metadata.paths = DatasetArtifactPaths {
        raw: relative(root, raw_path)?,
        raw_response: relative(root, raw_path)?,
        raw_request: dataset_raw_artifact_path(root, raw_path, "request.json")?,
        redacted_headers: dataset_raw_artifact_path(root, raw_path, "headers.redacted.json")?,
        provider_notes: dataset_raw_artifact_path(root, raw_path, "provider-notes.md")?,
        normalized: relative(root, normalized_path)?,
        validation: relative(root, validation_path)?,
        manifest: relative(root, manifest_path)?,
    };
    metadata.source = DatasetSourceMetadata {
        license_notes: metadata.license.clone(),
        url_or_endpoint: metadata.provider.clone(),
        retrieved_at: created_at.to_string(),
        credential_required: false,
    };
    metadata.created_at = created_at.to_string();
    metadata.checksums.metadata_sha256 = metadata_sha256(metadata)?;
    metadata.checksum = metadata.checksums.normalized_sha256.clone();
    Ok(())
}

pub(super) fn registry_contract_schema() -> String {
    REGISTRY_SCHEMA_V2.into()
}

pub(super) fn fail_closed_non_native_production_metadata(metadata: &mut DatasetMetadata) {
    if metadata.production_eligible && !metadata.native_interval {
        metadata.production_eligible = false;
        metadata.quality_status = "degraded".into();
    }
}

pub(super) fn fail_closed_derived_or_resampled_metadata(metadata: &mut DatasetMetadata) {
    let derived_or_resampled = metadata.price_basis.eq_ignore_ascii_case("derived")
        || metadata.price_basis.eq_ignore_ascii_case("resampled")
        || metadata.dataset_id.to_ascii_lowercase().contains("derived")
        || metadata
            .dataset_id
            .to_ascii_lowercase()
            .contains("resampled");
    if metadata.production_eligible && derived_or_resampled {
        metadata.production_eligible = false;
        metadata.quality_status = "diagnostic".into();
    }
}

fn dataset_raw_artifact_path(
    root: &Path,
    raw_path: &Path,
    file_name: &str,
) -> Result<String, DataStoreError> {
    let raw_dir = raw_path.parent().ok_or(DataStoreError::InvalidPath)?;
    relative(root, &raw_dir.join(file_name))
}

fn metadata_sha256(metadata: &DatasetMetadata) -> Result<String, DataStoreError> {
    let mut canonical = metadata.clone();
    canonical.checksums.metadata_sha256.clear();
    serde_json::to_vec(&canonical)
        .map(|bytes| bytes_checksum(&bytes))
        .map_err(|err| DataStoreError::Json(err.to_string()))
}

pub(super) fn fail_closed_validation_record(
    root: &Path,
    record: &StoredDatasetRecord,
    report: &ValidationReport,
) -> Result<(), DataStoreError> {
    let mut registry = read_json::<PersistentDatasetRegistry>(
        &root.join(".archon/trading-lab/data/registry.json"),
    )?;
    if let Some(stored) = registry
        .datasets
        .get_mut(&registry_key(&record.dataset_id, &record.version))
    {
        stored.production_eligible = false;
        stored.status = DatasetStatus::Degraded;
    }
    write_json(
        &root.join(".archon/trading-lab/data/registry.json"),
        &registry,
    )?;
    update_metadata_from_validation(root, record, report)
}

pub(super) fn sync_validation_record(
    root: &Path,
    record: &StoredDatasetRecord,
    report: &ValidationReport,
) -> Result<(), DataStoreError> {
    let mut registry = read_json::<PersistentDatasetRegistry>(
        &root.join(".archon/trading-lab/data/registry.json"),
    )?;
    if let Some(stored) = registry
        .datasets
        .get_mut(&registry_key(&record.dataset_id, &record.version))
    {
        stored.production_eligible = report.production_eligible;
        stored.status = if report.production_eligible {
            DatasetStatus::Healthy
        } else {
            DatasetStatus::Degraded
        };
    }
    write_json(
        &root.join(".archon/trading-lab/data/registry.json"),
        &registry,
    )?;
    update_metadata_from_validation(root, record, report)
}

fn update_metadata_from_validation(
    root: &Path,
    record: &StoredDatasetRecord,
    report: &ValidationReport,
) -> Result<(), DataStoreError> {
    let metadata_path = root.join(&record.metadata_path);
    let mut metadata: DatasetMetadata = read_json(&metadata_path)?;
    metadata.production_eligible = report.production_eligible;
    if !report.production_eligible {
        metadata.quality_status = "degraded".into();
    }
    write_json(&metadata_path, &metadata)
}
