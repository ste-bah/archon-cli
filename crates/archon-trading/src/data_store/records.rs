use super::*;

pub(super) fn record(
    root: &Path,
    versioned: &VersionedDataset,
    bars: &[OhlcvBar],
    paths: ArtifactPaths<'_>,
    validation: &ValidationReport,
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
        validation_checksum: validation.content_sha256.clone(),
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
    (normalized_path, raw_path): (&Path, &Path),
    (validation_path, manifest_path): (&Path, &Path),
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
    let source = metadata.source.clone();
    metadata.source = DatasetSourceMetadata {
        license_notes: non_empty_or(source.license_notes, metadata.license.clone()),
        url_or_endpoint: non_empty_or(source.url_or_endpoint, metadata.provider.clone()),
        retrieved_at: non_empty_or(source.retrieved_at, created_at.to_string()),
        credential_required: source.credential_required
            || metadata_license_requires_credentials(metadata),
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

pub(super) fn fail_closed_yfinance_fallback_metadata(metadata: &mut DatasetMetadata) {
    if metadata.provider.trim().eq_ignore_ascii_case("yfinance") {
        metadata.production_eligible = false;
        metadata.quality_status = "degraded".into();
    }
}

fn non_empty_or(value: String, fallback: String) -> String {
    if value.trim().is_empty() {
        fallback
    } else {
        value
    }
}

fn metadata_license_requires_credentials(metadata: &DatasetMetadata) -> bool {
    metadata
        .license
        .to_ascii_lowercase()
        .contains("credentials supplied")
}

fn dataset_raw_artifact_path(
    root: &Path,
    raw_path: &Path,
    file_name: &str,
) -> Result<String, DataStoreError> {
    let raw_dir = raw_path.parent().ok_or(DataStoreError::InvalidPath)?;
    relative(root, &raw_dir.join(file_name))
}

pub(super) fn metadata_sha256(metadata: &DatasetMetadata) -> Result<String, DataStoreError> {
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
    sync_validation_record(root, record, report)
}

pub(super) fn sync_validation_record(
    root: &Path,
    record: &StoredDatasetRecord,
    report: &ValidationReport,
) -> Result<(), DataStoreError> {
    let metadata = read_dataset_metadata(root, record)?;
    let mut registry = read_json::<PersistentDatasetRegistry>(
        &root.join(".archon/trading-lab/data/registry.json"),
    )?;
    if let Some(stored) = registry
        .datasets
        .get_mut(&registry_key(&record.dataset_id, &record.version))
    {
        stored.production_eligible = validation_is_production_eligible(report, &metadata);
        stored.status = if stored.production_eligible {
            DatasetStatus::Healthy
        } else {
            DatasetStatus::Degraded
        };
    }
    write_schema_json(
        &root.join(".archon/trading-lab/data/registry.json"),
        &registry,
    )?;
    update_metadata_from_validation(root, record, report)
}

pub(super) fn reconcile_versioned_from_validation(
    versioned: &mut VersionedDataset,
    report: &ValidationReport,
) {
    versioned.metadata.production_eligible =
        validation_is_production_eligible(report, &versioned.metadata);
    if !versioned.metadata.production_eligible
        && !versioned
            .metadata
            .quality_status
            .eq_ignore_ascii_case("diagnostic")
        && !metadata_is_yfinance_degraded_fallback(&versioned.metadata)
    {
        versioned.metadata.quality_status = "degraded".into();
    }
    versioned.status = if versioned.metadata.production_eligible {
        DatasetStatus::Healthy
    } else {
        DatasetStatus::Degraded
    };
}

fn validation_is_production_eligible(
    report: &ValidationReport,
    metadata: &DatasetMetadata,
) -> bool {
    report.allows_production() && metadata_can_satisfy_production(metadata)
}

fn update_metadata_from_validation(
    root: &Path,
    record: &StoredDatasetRecord,
    report: &ValidationReport,
) -> Result<(), DataStoreError> {
    let metadata_path = root.join(&record.metadata_path);
    let mut metadata = read_dataset_metadata(root, record)?;
    metadata.production_eligible = validation_is_production_eligible(report, &metadata);
    if !metadata.production_eligible && !metadata_is_yfinance_degraded_fallback(&metadata) {
        metadata.quality_status = "degraded".into();
    }
    write_schema_json(&metadata_path, &metadata)
}
