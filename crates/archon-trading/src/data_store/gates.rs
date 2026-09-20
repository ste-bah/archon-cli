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

pub(super) fn missing_dataset_backtest_gate_report(
    dataset_id: &str,
    version: &str,
    key: String,
) -> BacktestDataGateReport {
    let issues = vec![format!("missing dataset registry entry: {key}")];
    BacktestDataGateReport {
        dataset_id: dataset_id.into(),
        version: version.into(),
        diagnostic: true,
        promotion_eligible: false,
        overridden_issues: issues.clone(),
        issues,
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
    if metadata_is_yfinance_degraded_fallback(&dataset.metadata) {
        issues.push("yfinance degraded fallback artifacts cannot satisfy production gates".into());
    }
    if dataset
        .metadata
        .provider
        .trim()
        .eq_ignore_ascii_case("manual")
    {
        issues.push("manual datasets cannot satisfy provider-native production gates".into());
    }
    append_backtest_history_issues(record, dataset, issues);
    append_live_fetch_provenance_issues(root, record, dataset, issues);
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

fn append_backtest_history_issues(
    record: &StoredDatasetRecord,
    dataset: &StoredOhlcvDataset,
    issues: &mut Vec<String>,
) {
    let required = AHDM_BACKTEST_MINIMUM_ROWS;
    if record.bars < required {
        issues.push(format!(
            "registry bars {} below required production backtest minimum {}",
            record.bars, required
        ));
    }
    if dataset.bars.len() < required {
        issues.push(format!(
            "normalized payload rows {} below required production backtest minimum {}",
            dataset.bars.len(),
            required
        ));
    }
    if dataset.metadata.coverage.observed_bars < required as u64 {
        issues.push(format!(
            "metadata observed bars {} below required production backtest minimum {}",
            dataset.metadata.coverage.observed_bars, required
        ));
    }
}

pub(super) const AHDM_BACKTEST_MINIMUM_ROWS: usize = COVERAGE_MINIMUM_ROWS * 2;

fn append_live_fetch_provenance_issues(
    root: &Path,
    record: &StoredDatasetRecord,
    dataset: &StoredOhlcvDataset,
    issues: &mut Vec<String>,
) {
    if dataset
        .metadata
        .provider
        .trim()
        .eq_ignore_ascii_case("manual")
    {
        return;
    }
    append_live_text_issues(root, &record.raw_request_path, "raw request", issues);
    append_live_text_issues(root, &record.raw_response_path, "raw response", issues);
    append_live_text_issues(root, &record.provider_notes_path, "provider notes", issues);
    if dataset.metadata.source.retrieved_at.trim().is_empty() {
        issues.push("dataset source retrieved_at is missing live-fetch provenance".into());
    }
    if bars_have_linear_shape(&dataset.bars) {
        issues.push("normalized bars have deterministic linear OHLCV shape; production AHDM backtest requires real live-fetch history".into());
    }
}

fn append_live_text_issues(root: &Path, path: &str, label: &str, issues: &mut Vec<String>) {
    let text = match std::fs::read_to_string(root.join(path)) {
        Ok(text) => text,
        Err(_) => {
            issues.push(format!("{label} live-fetch provenance unreadable: {path}"));
            return;
        }
    };
    if !contains_live_fetch_marker(&text) {
        issues.push(format!(
            "{label} does not record captured live-fetch provider provenance: {path}"
        ));
    }
    if contains_non_live_provenance_marker(&text) {
        issues.push(format!(
            "{label} provenance indicates fixture/mock/synthetic/placeholder source: {path}"
        ));
    }
}

fn contains_live_fetch_marker(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    ["live", "fetch", "provider"]
        .iter()
        .all(|marker| lower.contains(marker))
}

fn contains_non_live_provenance_marker(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    ["fixture", "mock", "synthetic", "placeholder"]
        .iter()
        .any(|marker| lower.contains(marker))
}

fn bars_have_linear_shape(bars: &[OhlcvBar]) -> bool {
    bars.len() >= COVERAGE_MINIMUM_ROWS && field_is_linear(bars, |bar| bar.close)
}

fn field_is_linear(bars: &[OhlcvBar], value: fn(&OhlcvBar) -> f64) -> bool {
    let first_delta = value(&bars[1]) - value(&bars[0]);
    first_delta.abs() > f64::EPSILON
        && bars.windows(2).all(|pair| {
            ((value(&pair[1]) - value(&pair[0])) - first_delta).abs()
                <= f64::EPSILON * first_delta.abs().max(1.0)
        })
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
