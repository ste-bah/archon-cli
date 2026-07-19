use super::*;

pub(super) fn append_missing_artifact_issues(
    root: &Path,
    record: &StoredDatasetRecord,
    issues: &mut Vec<String>,
) {
    for path in [record.metadata_path.clone(), record.normalized_path.clone()]
        .into_iter()
        .chain(required_raw_artifact_paths(record))
        .chain([record.validation_path.clone(), record.manifest_path.clone()])
    {
        if path.trim().is_empty() || !root.join(&path).exists() {
            issues.push(format!("missing artifact: {path}"));
        }
    }
}

pub(super) fn append_dataset_gate_issues(
    root: &Path,
    record: &StoredDatasetRecord,
    dataset: &StoredOhlcvDataset,
    issues: &mut Vec<String>,
) {
    if validate_metadata(&dataset.metadata).is_err() {
        issues.push("metadata incomplete for production backtest".into());
    }
    if !metadata_has_expected_native_interval(&dataset.metadata) {
        issues.push("dataset does not match expected provider-native interval metadata".into());
    }
    if metadata_is_derived_or_resampled_diagnostic(&dataset.metadata) {
        issues.push("derived/resampled diagnostic candles cannot satisfy production gates".into());
    }
    if dataset
        .metadata
        .provider
        .trim()
        .eq_ignore_ascii_case("manual")
    {
        issues.push("manual datasets cannot satisfy provider-native production gates".into());
    }
    if dataset.bars.len() < 2 {
        issues.push("dataset has insufficient bar substance for a production backtest".into());
    }
    if !dataset.metadata.production_eligible {
        issues.push("dataset is not production eligible".into());
    }
    if record.status != DatasetStatus::Healthy {
        issues.push("dataset registry status is degraded".into());
    }
    let bars_hash = normalized_bars_checksum(&dataset.bars).unwrap_or_default();
    let normalized_hash = dataset.metadata.checksums.normalized_sha256.as_str();
    let registry_matches = record.checksum == bars_hash || record.checksum == normalized_hash;
    let metadata_matches =
        dataset.metadata.checksum == bars_hash || dataset.metadata.checksum == normalized_hash;
    if !registry_matches || !metadata_matches {
        issues.push("checksum mismatch between registry, metadata, and normalized bars".into());
    }
    match read_json::<ValidationReport>(&root.join(&record.validation_path)) {
        Ok(report) if validation_report_allows_production(&report) => {}
        Ok(_) => {
            issues.push("validation status is not passed or production eligibility is false".into())
        }
        Err(_) => issues.push("validation report missing or unreadable".into()),
    }
    if dataset.metadata.checksums.raw_sha256.is_empty()
        || dataset.metadata.checksums.normalized_sha256.is_empty()
        || dataset.metadata.checksums.metadata_sha256.is_empty()
    {
        issues.push("metadata checksums object is incomplete".into());
    }
    if dataset.metadata.paths.raw.is_empty()
        || dataset.metadata.paths.normalized.is_empty()
        || dataset.metadata.paths.validation.is_empty()
        || dataset.metadata.paths.manifest.is_empty()
    {
        issues.push("metadata paths object is incomplete".into());
    }
}

fn validation_report_allows_production(report: &ValidationReport) -> bool {
    report.allows_production()
}

pub(super) struct ArtifactPaths<'a> {
    pub(super) metadata: &'a Path,
    pub(super) normalized: &'a Path,
    pub(super) raw: &'a Path,
    pub(super) validation: &'a Path,
    pub(super) manifest: &'a Path,
}
