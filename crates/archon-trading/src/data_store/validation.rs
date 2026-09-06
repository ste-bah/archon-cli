use super::*;
mod checks;
pub(super) use checks::*;

#[cfg(test)]
pub(super) fn validation_report(
    metadata: &DatasetMetadata,
    bars: &[OhlcvBar],
    validated_at: String,
) -> ValidationReport {
    let native_lineage = fixture_native_lineage(metadata);
    validation_report_with_context(
        metadata,
        bars,
        validated_at,
        None,
        Some(&native_lineage),
        None,
    )
}

pub(super) fn validation_report_at_root(
    root: &Path,
    metadata: &DatasetMetadata,
    bars: &[OhlcvBar],
    validated_at: String,
) -> ValidationReport {
    let native_lineage = load_native_lineage_evidence(root, metadata);
    validation_report_with_context(
        metadata,
        bars,
        validated_at,
        None,
        native_lineage.as_ref(),
        Some(root),
    )
}

pub(super) fn validation_report_at_root_with_volume_evidence(
    root: &Path,
    metadata: &DatasetMetadata,
    bars: &[OhlcvBar],
    validated_at: String,
    volume_evidence: Option<&VolumeAbsenceEvidence>,
) -> ValidationReport {
    let native_lineage = load_native_lineage_evidence(root, metadata);
    validation_report_with_context(
        metadata,
        bars,
        validated_at,
        volume_evidence,
        native_lineage.as_ref(),
        Some(root),
    )
}

#[cfg(test)]
pub(super) fn validation_report_with_volume_evidence(
    metadata: &DatasetMetadata,
    bars: &[OhlcvBar],
    validated_at: String,
    volume_evidence: Option<&VolumeAbsenceEvidence>,
) -> ValidationReport {
    validation_report_with_context(metadata, bars, validated_at, volume_evidence, None, None)
}

fn validation_report_with_context(
    metadata: &DatasetMetadata,
    bars: &[OhlcvBar],
    validated_at: String,
    volume_evidence: Option<&VolumeAbsenceEvidence>,
    native_lineage: Option<&NativeLineageEvidence>,
    artifact_root: Option<&Path>,
) -> ValidationReport {
    let coverage_policy = coverage_policy();
    let session_calendar_evidence = session_calendar_evidence(metadata, bars)
        .unwrap_or_else(|| unavailable_session_calendar_evidence(metadata, bars));
    let summary = validation_summary(metadata, bars);
    let mut checks = Vec::new();
    push_metadata_checks(&mut checks, metadata, native_lineage);
    push_timestamp_checks(&mut checks, bars, &summary);
    push_numeric_checks(&mut checks, metadata, bars, &summary, volume_evidence);
    push_coverage_and_artifact_checks(
        &mut checks,
        metadata,
        bars,
        &summary,
        &coverage_policy,
        artifact_root,
    );
    build_report(
        metadata,
        checks,
        summary,
        coverage_policy,
        session_calendar_evidence,
        native_lineage.cloned(),
        validated_at,
    )
}

fn push_metadata_checks(
    checks: &mut Vec<ValidationCheck>,
    metadata: &DatasetMetadata,
    native_lineage: Option<&NativeLineageEvidence>,
) {
    push_check(
        checks,
        "metadata.complete",
        metadata_required_fields_present(metadata),
        "metadata contains required dataset identity, coverage, checksum, and artifact path fields",
    );
    push_check(
        checks,
        "metadata.production_contract",
        validate_metadata(metadata).is_ok(),
        "metadata is complete for production gate evaluation",
    );
    push_check(
        checks,
        "metadata.native_interval",
        native_interval_claim_is_provider_supported(metadata),
        "native_interval metadata claim is consistent with provider-native timeframe support",
    );
    push_check(
        checks,
        "metadata.native_observation_evidence",
        native_lineage.is_some_and(|evidence| native_observation_matches(metadata, evidence)),
        "typed native observation evidence is complete and matches the dataset identity",
    );
    push_check(
        checks,
        "metadata.lineage.underived",
        native_lineage.is_some_and(|evidence| evidence.lineage.is_exact_native()),
        "lineage independently denies aggregation, resampling, sampling, interpolation, and synthesis",
    );
    push_check(
        checks,
        "metadata.production_eligible",
        metadata.production_eligible
            && native_lineage.is_some_and(|evidence| native_lineage_matches(metadata, evidence)),
        "production eligibility is backed by matching exact-native lineage evidence",
    );
}

fn native_interval_claim_is_provider_supported(metadata: &DatasetMetadata) -> bool {
    if !metadata.native_interval {
        return false;
    }
    let provider = metadata.provider.trim().to_ascii_lowercase();
    let is_recognized = matches!(
        provider.as_str(),
        "tradingview" | "openbb" | "polygon" | "stooq" | "yfinance"
    );
    if !is_recognized {
        // Unrecognised provider: trust the metadata claim rather than fail closed on
        // something we cannot verify (e.g. "manual" fixtures in tests).
        return true;
    }
    crate::data_lake::provider_supports_native_timeframe(&provider, &metadata.timeframe)
}

fn push_timestamp_checks(
    checks: &mut Vec<ValidationCheck>,
    bars: &[OhlcvBar],
    summary: &ValidationSummary,
) {
    push_check(
        checks,
        "ohlcv.required_fields",
        !bars.is_empty(),
        "normalized OHLCV contains at least one complete row",
    );
    push_check(
        checks,
        "ohlcv.rfc3339_timestamps",
        !bars.is_empty() && timestamp_values_are_rfc3339(bars),
        "timestamps are normalized RFC3339 values with timezone",
    );
    push_check(
        checks,
        "ohlcv.monotonic_timestamps",
        !bars.is_empty() && !has_unsorted_timestamps(bars),
        "parsed timestamp instants are strictly ascending",
    );
    push_check(
        checks,
        "ohlcv.duplicate_timestamps",
        summary.duplicate_timestamp_count == 0,
        "parsed timestamp instants are unique",
    );
}

fn push_numeric_checks(
    checks: &mut Vec<ValidationCheck>,
    metadata: &DatasetMetadata,
    bars: &[OhlcvBar],
    summary: &ValidationSummary,
    volume_evidence: Option<&VolumeAbsenceEvidence>,
) {
    let volume_exempt =
        volume_evidence.is_some_and(|evidence| volume_absence_evidence_matches(metadata, evidence));
    push_check(
        checks,
        "ohlcv.finite_numbers",
        numbers_are_finite(bars),
        "every OHLCV numeric value is finite",
    );
    push_check(
        checks,
        "ohlcv.nonnegative_prices",
        prices_are_nonnegative(bars),
        "every OHLC price is nonnegative",
    );
    push_check(
        checks,
        "ohlcv.volume_presence",
        volume_is_present(bars) || volume_exempt,
        "volume is present or exact persisted provider evidence proves field absence",
    );
    push_check(
        checks,
        "ohlcv.nonnegative_volume",
        volume_is_nonnegative(bars),
        "volume is nonnegative",
    );
    push_check(
        checks,
        "ohlcv.ohlc_sanity",
        summary.bad_ohlc_count == 0,
        "OHLC prices are finite, positive, and internally consistent",
    );
    push_check(
        checks,
        "ohlcv.volume",
        summary.missing_volume_count == 0 && (volume_is_non_degenerate(bars) || volume_exempt),
        "volume is non-degenerate or covered by exact persisted field-absence evidence",
    );
}

fn push_coverage_and_artifact_checks(
    checks: &mut Vec<ValidationCheck>,
    metadata: &DatasetMetadata,
    bars: &[OhlcvBar],
    summary: &ValidationSummary,
    policy: &CoverageValidationPolicy,
    artifact_root: Option<&Path>,
) {
    let coverage_ok = coverage_inputs_are_consistent(metadata, bars, policy);
    push_check(
        checks,
        "metadata.coverage_minimum",
        coverage_ok,
        "metadata coverage and observed row counts are internally consistent",
    );
    push_check(
        checks,
        "ohlcv.coverage_inputs",
        coverage_ok,
        "coverage expected, observed, and missing counts reconcile",
    );
    push_warning_check(
        checks,
        "ohlcv.large_gaps",
        large_gap_within_policy(metadata, policy),
        "missing bars do not exceed the configured production threshold",
    );
    push_warning_check(
        checks,
        "ohlcv.gaps",
        summary.gap_count == 0,
        "metadata gap count is zero",
    );
    push_check(
        checks,
        "artifact.normalized_checksum",
        normalized_checksum_matches(metadata, bars),
        "normalized bars match metadata checksums",
    );
    push_check(
        checks,
        "artifact.required_files",
        artifact_root.map_or_else(
            || metadata_artifact_paths_are_safe(metadata),
            |root| required_artifact_files_present(root, metadata),
        ),
        "required artifact paths are root-confined regular files",
    );
    push_check(
        checks,
        "ohlcv.valid_bars",
        validate_bars(bars).is_ok(),
        "OHLCV bars are sorted, unique, finite, and sane",
    );
}

fn build_report(
    metadata: &DatasetMetadata,
    checks: Vec<ValidationCheck>,
    summary: ValidationSummary,
    coverage_policy: CoverageValidationPolicy,
    session_calendar_evidence: SessionCalendarEvidence,
    native_lineage: Option<NativeLineageEvidence>,
    validated_at: String,
) -> ValidationReport {
    let status = validation_status(&checks);
    let production_eligible = metadata.production_eligible
        && metadata.native_interval
        && status == ValidationStatus::Passed
        && native_lineage
            .as_ref()
            .is_some_and(|evidence| native_lineage_matches(metadata, evidence));
    let normalized_sha256 = metadata.checksums.normalized_sha256.clone();
    let content_sha256 = ValidationReport::content_hash(&normalized_sha256, &checks, &summary);
    ValidationReport {
        schema_version: crate::data_lake::VALIDATION_REPORT_SCHEMA.into(),
        dataset_id: metadata.dataset_id.clone(),
        version: metadata.version.clone(),
        status,
        native_interval: metadata.native_interval,
        native_lineage,
        production_eligible,
        checks,
        coverage_policy,
        session_calendar_evidence,
        normalized_sha256,
        content_sha256,
        summary,
        validated_at,
    }
}

fn unavailable_session_calendar_evidence(
    metadata: &DatasetMetadata,
    bars: &[OhlcvBar],
) -> SessionCalendarEvidence {
    SessionCalendarEvidence {
        session: metadata.session.clone(),
        calendar: String::new(),
        timezone: metadata.timezone.clone(),
        coverage_start: metadata.coverage.start.clone(),
        coverage_end: metadata.coverage.end.clone(),
        first_observed_at: bars
            .first()
            .map_or_else(String::new, |bar| bar.timestamp.clone()),
        last_observed_at: bars
            .last()
            .map_or_else(String::new, |bar| bar.timestamp.clone()),
        expected_bar_count: metadata.coverage.expected_bars,
        observed_bar_count: bars.len() as u64,
        derivation: String::new(),
    }
}
