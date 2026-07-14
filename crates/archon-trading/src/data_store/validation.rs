use super::*;
pub(super) fn validation_report(
    metadata: &DatasetMetadata,
    bars: &[OhlcvBar],
    validated_at: String,
) -> ValidationReport {
    let mut checks = Vec::new();
    let summary = validation_summary(metadata, bars);
    push_check(
        &mut checks,
        "metadata.complete",
        metadata_required_fields_present(metadata),
        "metadata contains required dataset identity, coverage, checksum, and artifact path fields",
    );
    push_check(
        &mut checks,
        "metadata.production_contract",
        validate_metadata(metadata).is_ok(),
        "metadata is complete for production gate evaluation",
    );
    push_check(
        &mut checks,
        "metadata.coverage_minimum",
        metadata.coverage.observed_bars > 0 && metadata.coverage.observed_bars == bars.len() as u64,
        "metadata observed row count matches normalized OHLCV rows",
    );
    push_check(
        &mut checks,
        "metadata.native_interval",
        metadata_has_expected_native_interval(metadata),
        "dataset uses expected provider-native interval metadata",
    );
    push_check(
        &mut checks,
        "metadata.not_derived_or_resampled",
        !metadata_is_derived_or_resampled_diagnostic(metadata),
        "derived or resampled diagnostic candles are not production eligible",
    );
    push_check(
        &mut checks,
        "metadata.production_eligible",
        metadata.production_eligible && !metadata_is_derived_or_resampled_diagnostic(metadata),
        "dataset is marked production eligible only for native production data",
    );
    push_check(
        &mut checks,
        "ohlcv.rfc3339_timestamps",
        !bars.is_empty() && timestamp_values_are_rfc3339(bars),
        "timestamps are normalized RFC3339 values with timezone",
    );
    push_check(
        &mut checks,
        "ohlcv.monotonic_timestamps",
        !has_unsorted_timestamps(bars),
        "timestamps are strictly ascending",
    );
    push_check(
        &mut checks,
        "ohlcv.duplicate_timestamps",
        summary.duplicate_timestamp_count == 0,
        "timestamps are unique",
    );
    push_check(
        &mut checks,
        "ohlcv.ohlc_sanity",
        summary.bad_ohlc_count == 0,
        "OHLC prices are finite, positive, and internally consistent",
    );
    push_check(
        &mut checks,
        "ohlcv.volume",
        summary.missing_volume_count == 0 && volume_is_non_degenerate(bars),
        "volume is present, positive, and non-degenerate for production datasets",
    );
    push_check(
        &mut checks,
        "ohlcv.gaps",
        summary.gap_count == 0,
        "metadata gap count is zero",
    );
    push_check(
        &mut checks,
        "ohlcv.valid_bars",
        validate_bars(bars).is_ok(),
        "OHLCV bars are sorted, unique, finite, and sane",
    );
    let status = validation_status(&checks);
    ValidationReport {
        schema_version: "archon-trading-validation-v1".into(),
        dataset_id: metadata.dataset_id.clone(),
        version: metadata.version.clone(),
        status,
        native_interval: metadata.native_interval,
        production_eligible: metadata.production_eligible && status == ValidationStatus::Passed,
        checks,
        summary,
        validated_at,
    }
}

fn volume_is_non_degenerate(bars: &[OhlcvBar]) -> bool {
    let Some(first) = bars.first().map(|bar| bar.volume) else {
        return false;
    };
    bars.len() > 1
        && first.is_finite()
        && first > 0.0
        && bars.iter().skip(1).any(|bar| {
            bar.volume.is_finite()
                && bar.volume > 0.0
                && (bar.volume - first).abs() > f64::EPSILON * first.abs().max(1.0)
        })
}

pub(super) fn validation_status(checks: &[ValidationCheck]) -> ValidationStatus {
    if checks.iter().any(|check| {
        check.status == ValidationStatus::Failed && check.severity == ValidationSeverity::Error
    }) {
        ValidationStatus::Failed
    } else if checks.iter().any(|check| {
        check.status == ValidationStatus::Failed && check.severity == ValidationSeverity::Warning
    }) {
        ValidationStatus::Degraded
    } else {
        ValidationStatus::Passed
    }
}

pub(super) fn validation_summary(
    metadata: &DatasetMetadata,
    bars: &[OhlcvBar],
) -> ValidationSummary {
    let mut seen = std::collections::BTreeSet::new();
    let mut duplicate_timestamp_count = 0;
    let mut bad_ohlc_count = 0;
    let mut missing_volume_count = 0;
    for bar in bars {
        if !seen.insert(bar.timestamp.clone()) {
            duplicate_timestamp_count += 1;
        }
        if bad_ohlc_bar(bar) {
            bad_ohlc_count += 1;
        }
        if bar.volume <= 0.0 || !bar.volume.is_finite() {
            missing_volume_count += 1;
        }
    }
    ValidationSummary {
        row_count: bars.len() as u64,
        duplicate_timestamp_count,
        gap_count: metadata.gaps.missing_bars,
        bad_ohlc_count,
        missing_volume_count,
    }
}

fn has_unsorted_timestamps(bars: &[OhlcvBar]) -> bool {
    let mut previous = "";
    for bar in bars {
        if !previous.is_empty() && bar.timestamp.as_str() < previous {
            return true;
        }
        previous = &bar.timestamp;
    }
    false
}

pub(super) fn timestamp_values_are_rfc3339(bars: &[OhlcvBar]) -> bool {
    bars.iter().all(|bar| {
        timestamp_has_timezone(&bar.timestamp)
            && chrono::DateTime::parse_from_rfc3339(&bar.timestamp).is_ok()
    })
}

pub(super) fn timestamp_has_timezone(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20 {
        return false;
    }
    if value.ends_with('Z') {
        return true;
    }
    value
        .rsplit_once(['+', '-'])
        .is_some_and(|(_, offset)| offset.len() == 5 && offset.as_bytes()[2] == b':')
}

pub(super) fn bad_ohlc_bar(bar: &OhlcvBar) -> bool {
    !bar.open.is_finite()
        || !bar.high.is_finite()
        || !bar.low.is_finite()
        || !bar.close.is_finite()
        || bar.open <= 0.0
        || bar.high <= 0.0
        || bar.low <= 0.0
        || bar.close <= 0.0
        || bar.high < bar.low
        || bar.high < bar.open
        || bar.high < bar.close
        || bar.low > bar.open
        || bar.low > bar.close
}

pub(super) fn metadata_has_expected_native_interval(metadata: &DatasetMetadata) -> bool {
    let provider = metadata.provider.trim().to_ascii_lowercase();
    metadata.native_interval
        && (provider == "manual"
            || provider_supports_native_timeframe(
                &provider,
                &normalize_timeframe(&metadata.timeframe),
            ))
}

pub(super) fn metadata_is_derived_or_resampled_diagnostic(metadata: &DatasetMetadata) -> bool {
    metadata.quality_status.eq_ignore_ascii_case("diagnostic")
        || metadata.price_basis.eq_ignore_ascii_case("derived")
        || metadata.price_basis.eq_ignore_ascii_case("resampled")
        || metadata
            .dataset_id
            .to_ascii_lowercase()
            .contains("resampled")
        || metadata.dataset_id.to_ascii_lowercase().contains("derived")
}

fn all_non_empty(values: &[&str]) -> bool {
    values.iter().all(|value| !value.trim().is_empty())
}

fn metadata_identity_fields_present(metadata: &DatasetMetadata) -> bool {
    all_non_empty(&[
        &metadata.dataset_id,
        &metadata.version,
        &metadata.canonical_instrument,
        &metadata.asset_class,
        &metadata.provider,
        &metadata.provider_symbol,
        &metadata.timeframe,
        &metadata.timezone,
        &metadata.adjustment,
        &metadata.license,
    ])
}

fn metadata_coverage_fields_present(metadata: &DatasetMetadata) -> bool {
    all_non_empty(&[&metadata.coverage.start, &metadata.coverage.end])
        && metadata.coverage.expected_bars > 0
        && metadata.gaps.expected_bars > 0
}

fn metadata_checksum_fields_present(metadata: &DatasetMetadata) -> bool {
    all_non_empty(&[
        &metadata.checksum,
        &metadata.checksums.raw_sha256,
        &metadata.checksums.normalized_sha256,
        &metadata.checksums.metadata_sha256,
    ])
}

fn metadata_path_fields_present(metadata: &DatasetMetadata) -> bool {
    all_non_empty(&[
        &metadata.paths.raw,
        &metadata.paths.raw_response,
        &metadata.paths.raw_request,
        &metadata.paths.redacted_headers,
        &metadata.paths.provider_notes,
        &metadata.paths.normalized,
        &metadata.paths.validation,
        &metadata.paths.manifest,
    ])
}

fn metadata_required_fields_present(metadata: &DatasetMetadata) -> bool {
    metadata_identity_fields_present(metadata)
        && metadata_coverage_fields_present(metadata)
        && metadata_checksum_fields_present(metadata)
        && metadata_path_fields_present(metadata)
}

pub(super) fn push_check(checks: &mut Vec<ValidationCheck>, id: &str, passed: bool, message: &str) {
    checks.push(ValidationCheck {
        id: id.into(),
        status: if passed {
            ValidationStatus::Passed
        } else {
            ValidationStatus::Failed
        },
        severity: ValidationSeverity::Error,
        message: message.into(),
    });
}
