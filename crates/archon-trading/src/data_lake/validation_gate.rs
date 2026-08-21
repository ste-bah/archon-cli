use super::{ValidationReport, ValidationStatus, required_check_ids};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductionUse {
    Backtest,
    StrategySpecPromotion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductionDenialCode {
    ValidationReportIo,
    SchemaMismatch,
    IdentityMismatch,
    StatusNotPassed,
    NativeIntervalRequired,
    NativeLineageMissing,
    DerivedLineage,
    ChecksumMismatch,
    RequiredArtifactEvidenceMissing,
    RequiredValidationCheckFailed,
    InconsistentReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProductionGateDecision {
    pub production_use: ProductionUse,
    pub denial_code: Option<ProductionDenialCode>,
}

impl ProductionGateDecision {
    pub fn allowed(self) -> bool {
        self.denial_code.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidationReportIoError;

pub enum ValidationReportEvidence<'a> {
    Report(&'a ValidationReport),
    IoFailure,
}

impl<'a> From<&'a ValidationReport> for ValidationReportEvidence<'a> {
    fn from(report: &'a ValidationReport) -> Self {
        Self::Report(report)
    }
}

impl<'a> From<Result<&'a ValidationReport, ValidationReportIoError>>
    for ValidationReportEvidence<'a>
{
    fn from(result: Result<&'a ValidationReport, ValidationReportIoError>) -> Self {
        match result {
            Ok(report) => Self::Report(report),
            Err(_) => Self::IoFailure,
        }
    }
}

/// Applies one fail-closed policy to every production use and returns a stable denial code.
pub fn validation_report_allows_production<'a>(
    evidence: impl Into<ValidationReportEvidence<'a>>,
    production_use: ProductionUse,
) -> ProductionGateDecision {
    let denial_code = match evidence.into() {
        ValidationReportEvidence::Report(report) => denial_code(report),
        ValidationReportEvidence::IoFailure => Some(ProductionDenialCode::ValidationReportIo),
    };
    ProductionGateDecision {
        production_use,
        denial_code,
    }
}

/// Compatibility wrapper for callers that only need the legacy boolean answer.
pub fn allows_production(report: &ValidationReport) -> bool {
    validation_report_allows_production(report, ProductionUse::Backtest).allowed()
}

fn denial_code(report: &ValidationReport) -> Option<ProductionDenialCode> {
    if report.schema_version != super::VALIDATION_REPORT_SCHEMA {
        return Some(ProductionDenialCode::SchemaMismatch);
    }
    if !identity_matches(report) {
        return Some(ProductionDenialCode::IdentityMismatch);
    }
    if report.status != ValidationStatus::Passed {
        return Some(ProductionDenialCode::StatusNotPassed);
    }
    let Some(lineage) = report.native_lineage.as_ref() else {
        return Some(ProductionDenialCode::NativeLineageMissing);
    };
    if !report.native_interval || !observation_is_exact_native(report) {
        return Some(ProductionDenialCode::NativeIntervalRequired);
    }
    if !lineage.lineage.is_exact_native() {
        return Some(ProductionDenialCode::DerivedLineage);
    }
    if !checksums_match(report) {
        return Some(ProductionDenialCode::ChecksumMismatch);
    }
    if !passed_check(report, "artifact.required_files") {
        return Some(ProductionDenialCode::RequiredArtifactEvidenceMissing);
    }
    if !required_check_ids()
        .iter()
        .all(|id| passed_check(report, id))
    {
        return Some(ProductionDenialCode::RequiredValidationCheckFailed);
    }
    if !report.is_consistent() || !report.production_eligible {
        return Some(ProductionDenialCode::InconsistentReport);
    }
    None
}

fn identity_matches(report: &ValidationReport) -> bool {
    if report.dataset_id.trim().is_empty() || report.version.trim().is_empty() {
        return false;
    }
    report.native_lineage.as_ref().is_none_or(|lineage| {
        let observation = &lineage.observation;
        observation.dataset_id == report.dataset_id && observation.version == report.version
    })
}

fn observation_is_exact_native(report: &ValidationReport) -> bool {
    report.native_lineage.as_ref().is_some_and(|lineage| {
        lineage.observation.exact_native_interval && lineage.observation.complete
    })
}

fn checksums_match(report: &ValidationReport) -> bool {
    !report.normalized_sha256.trim().is_empty()
        && !report.content_sha256.trim().is_empty()
        && report.content_sha256
            == ValidationReport::content_hash(
                &report.normalized_sha256,
                &report.checks,
                &report.summary,
            )
}

fn passed_check(report: &ValidationReport, id: &str) -> bool {
    report
        .checks
        .iter()
        .any(|check| check.id == id && check.status == ValidationStatus::Passed)
}
