use crate::ohlcv::bytes_checksum;
use serde::{Deserialize, Deserializer, Serialize};

pub const VALIDATION_REPORT_SCHEMA: &str = "archon-trading-validation-v1";

/// Persisted provider evidence that a native payload omitted the volume field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VolumeAbsenceEvidence {
    pub provider: String,
    pub canonical_instrument: String,
    pub provider_symbol: String,
    pub timeframe: String,
    pub source_action: String,
    pub retrieved_at: String,
    pub evidence_path: String,
    pub volume_field_present: bool,
}

/// Identity-bound proof that observations came from the requested native series.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeObservationEvidence {
    pub dataset_id: String,
    pub version: String,
    pub provider: String,
    pub canonical_instrument: String,
    pub provider_symbol: String,
    pub timeframe: String,
    pub retrieved_at: String,
    pub exact_native_interval: bool,
    pub complete: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DerivationLineage {
    pub aggregated: bool,
    pub resampled: bool,
    pub downsampled: bool,
    pub upsampled: bool,
    pub interpolated: bool,
    pub synthesized: bool,
}

impl DerivationLineage {
    pub fn is_exact_native(self) -> bool {
        !self.aggregated
            && !self.resampled
            && !self.downsampled
            && !self.upsampled
            && !self.interpolated
            && !self.synthesized
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeLineageEvidence {
    pub observation: NativeObservationEvidence,
    pub lineage: DerivationLineage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ValidationStatus {
    Passed,
    Degraded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ValidationSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationCheck {
    pub id: String,
    pub status: ValidationStatus,
    pub severity: ValidationSeverity,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageValidationPolicy {
    pub minimum_bar_count: u64,
    pub large_gap_threshold_bps: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCalendarEvidence {
    pub session: String,
    pub calendar: String,
    pub timezone: String,
    pub coverage_start: String,
    pub coverage_end: String,
    pub first_observed_at: String,
    pub last_observed_at: String,
    pub expected_bar_count: u64,
    pub observed_bar_count: u64,
    pub derivation: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationSummary {
    pub row_count: u64,
    pub duplicate_timestamp_count: u64,
    pub gap_count: u64,
    pub bad_ohlc_count: u64,
    pub missing_volume_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValidationReport {
    pub schema_version: String,
    pub dataset_id: String,
    pub version: String,
    pub status: ValidationStatus,
    pub native_interval: bool,
    pub native_lineage: Option<NativeLineageEvidence>,
    pub production_eligible: bool,
    pub checks: Vec<ValidationCheck>,
    pub coverage_policy: CoverageValidationPolicy,
    pub session_calendar_evidence: SessionCalendarEvidence,
    pub normalized_sha256: String,
    pub content_sha256: String,
    pub summary: ValidationSummary,
    pub validated_at: String,
}

impl ValidationReport {
    pub fn status_from_checks(checks: &[ValidationCheck]) -> ValidationStatus {
        if checks.iter().any(|check| {
            check.status == ValidationStatus::Failed && check.severity == ValidationSeverity::Error
        }) {
            ValidationStatus::Failed
        } else if checks.iter().any(|check| {
            check.status == ValidationStatus::Failed
                && check.severity == ValidationSeverity::Warning
        }) {
            ValidationStatus::Degraded
        } else {
            ValidationStatus::Passed
        }
    }

    pub fn is_consistent(&self) -> bool {
        self.has_base_contract()
            && self.has_coverage_contract()
            && !self.normalized_sha256.trim().is_empty()
            && !self.content_sha256.trim().is_empty()
            && self.status == Self::status_from_checks(&self.checks)
            && self.production_eligible == self.recomputed_production_eligibility()
    }

    fn has_base_contract(&self) -> bool {
        self.schema_version == VALIDATION_REPORT_SCHEMA
            && !self.dataset_id.trim().is_empty()
            && !self.version.trim().is_empty()
            && chrono::DateTime::parse_from_rfc3339(&self.validated_at).is_ok()
    }

    fn has_coverage_contract(&self) -> bool {
        let evidence = &self.session_calendar_evidence;
        self.coverage_policy.minimum_bar_count > 0
            && self.coverage_policy.large_gap_threshold_bps > 0
            && self.coverage_policy.large_gap_threshold_bps <= 10_000
            && !evidence.session.trim().is_empty()
            && !evidence.calendar.trim().is_empty()
            && !evidence.timezone.trim().is_empty()
            && !evidence.derivation.trim().is_empty()
            && evidence.expected_bar_count >= self.coverage_policy.minimum_bar_count
            && evidence.observed_bar_count >= self.coverage_policy.minimum_bar_count
            && evidence.observed_bar_count <= evidence.expected_bar_count
            && chrono::DateTime::parse_from_rfc3339(&evidence.coverage_start).is_ok()
            && chrono::DateTime::parse_from_rfc3339(&evidence.coverage_end).is_ok()
            && chrono::DateTime::parse_from_rfc3339(&evidence.first_observed_at).is_ok()
            && chrono::DateTime::parse_from_rfc3339(&evidence.last_observed_at).is_ok()
            && Self::coverage_boundaries_match(evidence)
    }

    fn coverage_boundaries_match(evidence: &SessionCalendarEvidence) -> bool {
        let parse = |value: &str| chrono::DateTime::parse_from_rfc3339(value);
        matches!(
            (
                parse(&evidence.coverage_start),
                parse(&evidence.coverage_end),
                parse(&evidence.first_observed_at),
                parse(&evidence.last_observed_at),
            ),
            (Ok(start), Ok(end), Ok(first), Ok(last))
                if start == first && end == last && start <= end
        )
    }

    fn recomputed_production_eligibility(&self) -> bool {
        self.status == ValidationStatus::Passed
            && self.native_interval
            && self.has_matching_exact_native_lineage()
            && required_check_ids()
                .iter()
                .all(|id| self.checks.iter().any(|check| check.id == *id))
    }

    fn has_matching_exact_native_lineage(&self) -> bool {
        self.native_lineage.as_ref().is_some_and(|evidence| {
            let observation = &evidence.observation;
            observation.dataset_id == self.dataset_id
                && observation.version == self.version
                && non_empty(&[
                    &observation.provider,
                    &observation.canonical_instrument,
                    &observation.provider_symbol,
                    &observation.timeframe,
                ])
                && chrono::DateTime::parse_from_rfc3339(&observation.retrieved_at).is_ok()
                && observation.exact_native_interval
                && observation.complete
                && evidence.lineage.is_exact_native()
        })
    }

    pub fn content_hash(
        normalized_sha256: &str,
        checks: &[ValidationCheck],
        summary: &ValidationSummary,
    ) -> String {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "normalized_sha256": normalized_sha256,
            "checks": checks,
            "summary": summary,
        }))
        .unwrap_or_default();
        bytes_checksum(&bytes)
    }

    pub fn allows_production(&self) -> bool {
        self.is_consistent() && self.production_eligible
    }
}

pub fn required_check_ids() -> &'static [&'static str] {
    &[
        "metadata.complete",
        "metadata.production_contract",
        "metadata.coverage_minimum",
        "metadata.native_observation_evidence",
        "metadata.lineage.underived",
        "metadata.production_eligible",
        "ohlcv.required_fields",
        "ohlcv.rfc3339_timestamps",
        "ohlcv.monotonic_timestamps",
        "ohlcv.duplicate_timestamps",
        "ohlcv.finite_numbers",
        "ohlcv.nonnegative_prices",
        "ohlcv.volume_presence",
        "ohlcv.nonnegative_volume",
        "ohlcv.ohlc_sanity",
        "ohlcv.volume",
        "ohlcv.coverage_inputs",
        "ohlcv.large_gaps",
        "ohlcv.gaps",
        "artifact.normalized_checksum",
        "artifact.required_files",
        "ohlcv.valid_bars",
    ]
}

impl<'de> Deserialize<'de> for ValidationReport {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            #[serde(alias = "schema")]
            schema_version: String,
            dataset_id: String,
            version: String,
            status: ValidationStatus,
            native_interval: bool,
            #[serde(default)]
            native_lineage: Option<NativeLineageEvidence>,
            production_eligible: bool,
            checks: Vec<ValidationCheck>,
            coverage_policy: CoverageValidationPolicy,
            session_calendar_evidence: SessionCalendarEvidence,
            normalized_sha256: String,
            content_sha256: String,
            summary: ValidationSummary,
            validated_at: String,
            #[serde(default, rename = "duplicate_timestamp_check")]
            _duplicate_timestamp_check: Option<serde_json::Value>,
            #[serde(default, rename = "ohlc_check")]
            _ohlc_check: Option<serde_json::Value>,
            #[serde(default, rename = "volume_check")]
            _volume_check: Option<serde_json::Value>,
            #[serde(default, rename = "gap_check")]
            _gap_check: Option<serde_json::Value>,
            #[serde(default, rename = "timestamp_check")]
            _timestamp_check: Option<serde_json::Value>,
            #[serde(default, rename = "metadata_check")]
            _metadata_check: Option<serde_json::Value>,
        }

        let wire = Wire::deserialize(deserializer)?;
        let report = Self {
            schema_version: wire.schema_version,
            dataset_id: wire.dataset_id,
            version: wire.version,
            status: wire.status,
            native_interval: wire.native_interval,
            native_lineage: wire.native_lineage,
            production_eligible: wire.production_eligible,
            checks: wire.checks,
            coverage_policy: wire.coverage_policy,
            session_calendar_evidence: wire.session_calendar_evidence,
            normalized_sha256: wire.normalized_sha256,
            content_sha256: wire.content_sha256,
            summary: wire.summary,
            validated_at: wire.validated_at,
        };
        if !report.is_consistent() {
            return Err(serde::de::Error::custom(
                "validation report status/production eligibility contradicts recomputed values",
            ));
        }
        Ok(report)
    }
}

fn non_empty(values: &[&str]) -> bool {
    values.iter().all(|value| !value.trim().is_empty())
}
