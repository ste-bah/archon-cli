use super::super::*;
#[cfg(test)]
use crate::data_lake::{DerivationLineage, NativeObservationEvidence};

const VOLUME_ABSENCE_ACTION: &str = "fetch_ohlcv_native";

pub(crate) fn load_volume_absence_evidence(
    root: &std::path::Path,
    metadata: &DatasetMetadata,
) -> Option<VolumeAbsenceEvidence> {
    if !safe_relative_evidence_path(&metadata.paths.raw_request) {
        return None;
    }
    let artifact: serde_json::Value = read_json(&root.join(&metadata.paths.raw_request)).ok()?;
    let evidence = artifact.get("volume_absence_evidence")?.clone();
    let evidence: VolumeAbsenceEvidence = serde_json::from_value(evidence).ok()?;
    volume_absence_evidence_matches(metadata, &evidence).then_some(evidence)
}

pub(crate) fn volume_absence_evidence_matches(
    metadata: &DatasetMetadata,
    evidence: &VolumeAbsenceEvidence,
) -> bool {
    !evidence.volume_field_present
        && evidence.provider == metadata.provider
        && evidence.canonical_instrument == metadata.canonical_instrument
        && evidence.provider_symbol == metadata.provider_symbol
        && evidence.timeframe == metadata.timeframe
        && evidence.source_action == VOLUME_ABSENCE_ACTION
        && evidence.retrieved_at == metadata.source.retrieved_at
        && chrono::DateTime::parse_from_rfc3339(&evidence.retrieved_at).is_ok()
        && evidence.evidence_path == metadata.paths.raw_request
        && safe_relative_evidence_path(&evidence.evidence_path)
}

fn safe_relative_evidence_path(path: &str) -> bool {
    let path = std::path::Path::new(path);
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path.components().all(|component| {
            matches!(
                component,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
}

pub(crate) fn load_native_lineage_evidence(
    root: &std::path::Path,
    metadata: &DatasetMetadata,
) -> Option<NativeLineageEvidence> {
    if !safe_relative_evidence_path(&metadata.paths.raw_request) {
        return None;
    }
    let artifact: serde_json::Value = read_json(&root.join(&metadata.paths.raw_request)).ok()?;
    let evidence = artifact.get("native_lineage_evidence")?.clone();
    serde_json::from_value(evidence).ok()
}

#[cfg(test)]
pub(crate) fn fixture_native_lineage(metadata: &DatasetMetadata) -> NativeLineageEvidence {
    NativeLineageEvidence {
        observation: NativeObservationEvidence {
            dataset_id: metadata.dataset_id.clone(),
            version: metadata.version.clone(),
            provider: metadata.provider.clone(),
            canonical_instrument: metadata.canonical_instrument.clone(),
            provider_symbol: metadata.provider_symbol.clone(),
            timeframe: metadata.timeframe.clone(),
            retrieved_at: metadata.source.retrieved_at.clone(),
            exact_native_interval: metadata.native_interval,
            complete: true,
        },
        lineage: DerivationLineage::default(),
    }
}

pub(crate) fn native_observation_matches(
    metadata: &DatasetMetadata,
    evidence: &NativeLineageEvidence,
) -> bool {
    let observation = &evidence.observation;
    observation.dataset_id == metadata.dataset_id
        && observation.version == metadata.version
        && observation.provider == metadata.provider
        && observation.canonical_instrument == metadata.canonical_instrument
        && observation.provider_symbol == metadata.provider_symbol
        && observation.timeframe == metadata.timeframe
        && observation.retrieved_at == metadata.source.retrieved_at
        && chrono::DateTime::parse_from_rfc3339(&observation.retrieved_at).is_ok()
        && observation.exact_native_interval
        && observation.complete
}

pub(crate) fn native_lineage_matches(
    metadata: &DatasetMetadata,
    evidence: &NativeLineageEvidence,
) -> bool {
    native_observation_matches(metadata, evidence) && evidence.lineage.is_exact_native()
}

pub(crate) fn volume_is_non_degenerate(bars: &[OhlcvBar]) -> bool {
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

pub(crate) fn validation_status(checks: &[ValidationCheck]) -> ValidationStatus {
    ValidationReport::status_from_checks(checks)
}

pub(crate) fn validation_summary(
    metadata: &DatasetMetadata,
    bars: &[OhlcvBar],
) -> ValidationSummary {
    let mut seen = std::collections::BTreeSet::new();
    let mut duplicate_timestamp_count = 0;
    let mut bad_ohlc_count = 0;
    let mut missing_volume_count = 0;
    for bar in bars {
        let instant = parsed_timestamp(&bar.timestamp);
        if instant.is_some() && !seen.insert(instant) {
            duplicate_timestamp_count += 1;
        }
        if bad_ohlc_bar(bar) {
            bad_ohlc_count += 1;
        }
        if !bar.volume.is_finite() || bar.volume < 0.0 {
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

pub(crate) fn has_unsorted_timestamps(bars: &[OhlcvBar]) -> bool {
    bars.windows(2).any(|pair| {
        let Some(previous) = parsed_timestamp(&pair[0].timestamp) else {
            return true;
        };
        let Some(current) = parsed_timestamp(&pair[1].timestamp) else {
            return true;
        };
        current <= previous
    })
}

pub(crate) fn timestamp_values_are_rfc3339(bars: &[OhlcvBar]) -> bool {
    bars.iter().all(|bar| {
        timestamp_has_timezone(&bar.timestamp) && parsed_timestamp(&bar.timestamp).is_some()
    })
}

fn parsed_timestamp(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&chrono::Utc))
}

pub(crate) fn timestamp_has_timezone(value: &str) -> bool {
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

pub(crate) fn numbers_are_finite(bars: &[OhlcvBar]) -> bool {
    bars.iter().all(|bar| {
        [bar.open, bar.high, bar.low, bar.close, bar.volume]
            .iter()
            .all(|value| value.is_finite())
    })
}

pub(crate) fn prices_are_nonnegative(bars: &[OhlcvBar]) -> bool {
    bars.iter().all(|bar| {
        [bar.open, bar.high, bar.low, bar.close]
            .iter()
            .all(|value| *value >= 0.0)
    })
}

pub(crate) fn volume_is_present(bars: &[OhlcvBar]) -> bool {
    !bars.is_empty() && bars.iter().any(|bar| bar.volume > 0.0)
}

pub(crate) fn volume_is_nonnegative(bars: &[OhlcvBar]) -> bool {
    bars.iter().all(|bar| bar.volume >= 0.0)
}

pub(crate) fn bad_ohlc_bar(bar: &OhlcvBar) -> bool {
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

pub(crate) fn metadata_is_yfinance_degraded_fallback(metadata: &DatasetMetadata) -> bool {
    metadata.provider.trim().eq_ignore_ascii_case("yfinance")
        || metadata
            .quality_status
            .eq_ignore_ascii_case("degraded_fallback")
}

fn all_non_empty(values: &[&str]) -> bool {
    values.iter().all(|value| !value.trim().is_empty())
}

fn metadata_identity_fields_present(metadata: &DatasetMetadata) -> bool {
    all_non_empty(&[
        &metadata.schema_version,
        &metadata.dataset_id,
        &metadata.version,
        &metadata.canonical_instrument,
        &metadata.asset_class,
        &metadata.provider,
        &metadata.provider_symbol,
        &metadata.timeframe,
        &metadata.price_basis,
        &metadata.session,
        &metadata.timezone,
        &metadata.adjustment,
        &metadata.license,
        &metadata.quality_status,
        &metadata.created_at,
    ]) && !metadata.symbol_map.is_empty()
        && all_non_empty(&[
            &metadata.source.license_notes,
            &metadata.source.url_or_endpoint,
            &metadata.source.retrieved_at,
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

fn artifact_paths(metadata: &DatasetMetadata) -> [&str; 8] {
    [
        &metadata.paths.raw,
        &metadata.paths.raw_response,
        &metadata.paths.raw_request,
        &metadata.paths.redacted_headers,
        &metadata.paths.provider_notes,
        &metadata.paths.normalized,
        &metadata.paths.validation,
        &metadata.paths.manifest,
    ]
}

fn required_input_artifact_paths(metadata: &DatasetMetadata) -> [&str; 6] {
    [
        &metadata.paths.raw,
        &metadata.paths.raw_response,
        &metadata.paths.raw_request,
        &metadata.paths.redacted_headers,
        &metadata.paths.provider_notes,
        &metadata.paths.normalized,
    ]
}

fn safe_relative_artifact_path(value: &str) -> bool {
    let path = std::path::Path::new(value);
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

pub(crate) fn metadata_artifact_paths_are_safe(metadata: &DatasetMetadata) -> bool {
    artifact_paths(metadata)
        .iter()
        .all(|path| safe_relative_artifact_path(path))
}

pub(crate) fn required_artifact_files_present(
    root: &std::path::Path,
    metadata: &DatasetMetadata,
) -> bool {
    let Ok(canonical_root) = root.canonicalize() else {
        return false;
    };
    metadata_artifact_paths_are_safe(metadata)
        && required_input_artifact_paths(metadata)
            .iter()
            .all(|relative| regular_file_within_root(&canonical_root, relative))
}

fn regular_file_within_root(canonical_root: &std::path::Path, relative: &str) -> bool {
    let Ok(canonical_path) = canonical_root.join(relative).canonicalize() else {
        return false;
    };
    canonical_path.starts_with(canonical_root)
        && std::fs::metadata(canonical_path).is_ok_and(|metadata| metadata.is_file())
}

pub(crate) fn coverage_inputs_are_consistent(
    metadata: &DatasetMetadata,
    bars: &[OhlcvBar],
    policy: &CoverageValidationPolicy,
) -> bool {
    let Some(evidence) = session_calendar_evidence(metadata, bars) else {
        return false;
    };
    coverage_policy_is_valid(policy)
        && metadata.coverage.expected_bars > 0
        && metadata.coverage.observed_bars == bars.len() as u64
        && metadata.coverage.observed_bars >= policy.minimum_bar_count
        && metadata.gaps.expected_bars == metadata.coverage.expected_bars
        && metadata.gaps.missing_bars
            == metadata
                .coverage
                .expected_bars
                .saturating_sub(metadata.coverage.observed_bars)
        && evidence.expected_bar_count == metadata.coverage.expected_bars
        && evidence.observed_bar_count == metadata.coverage.observed_bars
}

pub(crate) fn coverage_policy() -> CoverageValidationPolicy {
    CoverageValidationPolicy {
        minimum_bar_count: 1,
        large_gap_threshold_bps: 100,
    }
}

pub(crate) fn coverage_policy_is_valid(policy: &CoverageValidationPolicy) -> bool {
    policy.minimum_bar_count > 0
        && policy.large_gap_threshold_bps > 0
        && policy.large_gap_threshold_bps <= 10_000
}

pub(crate) fn session_calendar_evidence(
    metadata: &DatasetMetadata,
    bars: &[OhlcvBar],
) -> Option<SessionCalendarEvidence> {
    let first = bars.first()?;
    let last = bars.last()?;
    let start = parsed_timestamp(&metadata.coverage.start)?;
    let end = parsed_timestamp(&metadata.coverage.end)?;
    if start != parsed_timestamp(&first.timestamp)?
        || end != parsed_timestamp(&last.timestamp)?
        || start > end
        || metadata.session.trim().is_empty()
        || metadata.timezone.trim().is_empty()
    {
        return None;
    }
    Some(SessionCalendarEvidence {
        session: metadata.session.clone(),
        calendar: coverage_calendar(metadata),
        timezone: metadata.timezone.clone(),
        coverage_start: metadata.coverage.start.clone(),
        coverage_end: metadata.coverage.end.clone(),
        first_observed_at: first.timestamp.clone(),
        last_observed_at: last.timestamp.clone(),
        expected_bar_count: metadata.coverage.expected_bars,
        observed_bar_count: bars.len() as u64,
        derivation: "persisted metadata session/calendar and parsed normalized endpoints".into(),
    })
}

fn coverage_calendar(metadata: &DatasetMetadata) -> String {
    if metadata.session.eq_ignore_ascii_case("24x7") {
        "continuous_24x7".into()
    } else {
        format!("provider_session:{}", metadata.session)
    }
}

pub(crate) fn large_gap_within_policy(
    metadata: &DatasetMetadata,
    policy: &CoverageValidationPolicy,
) -> bool {
    metadata.gaps.missing_bars.saturating_mul(10_000)
        <= metadata
            .gaps
            .expected_bars
            .saturating_mul(u64::from(policy.large_gap_threshold_bps))
}

pub(crate) fn normalized_checksum_matches(metadata: &DatasetMetadata, bars: &[OhlcvBar]) -> bool {
    normalized_bars_checksum(bars).is_ok_and(|actual| {
        actual == metadata.checksum && actual == metadata.checksums.normalized_sha256
    })
}

pub(crate) fn metadata_required_fields_present(metadata: &DatasetMetadata) -> bool {
    metadata_identity_fields_present(metadata)
        && metadata_coverage_fields_present(metadata)
        && metadata_checksum_fields_present(metadata)
        && metadata_artifact_paths_are_safe(metadata)
}

pub(crate) fn push_check(checks: &mut Vec<ValidationCheck>, id: &str, passed: bool, message: &str) {
    push_check_with_severity(checks, id, passed, ValidationSeverity::Error, message);
}

pub(crate) fn push_warning_check(
    checks: &mut Vec<ValidationCheck>,
    id: &str,
    passed: bool,
    message: &str,
) {
    push_check_with_severity(checks, id, passed, ValidationSeverity::Warning, message);
}

fn push_check_with_severity(
    checks: &mut Vec<ValidationCheck>,
    id: &str,
    passed: bool,
    severity: ValidationSeverity,
    message: &str,
) {
    checks.push(ValidationCheck {
        id: id.into(),
        status: if passed {
            ValidationStatus::Passed
        } else {
            ValidationStatus::Failed
        },
        severity,
        message: message.into(),
    });
}
