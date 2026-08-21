use super::{passed_checks, report};
use crate::data_lake::validation_gate::{
    ProductionDenialCode, ProductionUse, ValidationReportIoError,
    validation_report_allows_production,
};
use crate::data_lake::{ValidationReport, ValidationStatus};

type Fault = fn(&mut ValidationReport);

const FAULTS: [(Fault, ProductionDenialCode); 7] = [
    (schema_fault, ProductionDenialCode::SchemaMismatch),
    (identity_fault, ProductionDenialCode::IdentityMismatch),
    (status_fault, ProductionDenialCode::StatusNotPassed),
    (native_fault, ProductionDenialCode::NativeIntervalRequired),
    (lineage_fault, ProductionDenialCode::DerivedLineage),
    (checksum_fault, ProductionDenialCode::ChecksumMismatch),
    (
        artifact_fault,
        ProductionDenialCode::RequiredArtifactEvidenceMissing,
    ),
];

const COMPLETE_FAULT_COUNT: usize = FAULTS.len() + 3;

pub(super) fn run() {
    for production_use in [
        ProductionUse::Backtest,
        ProductionUse::StrategySpecPromotion,
    ] {
        assert!(gate(&report(passed_checks()), production_use).is_none());
        for (apply_fault, expected) in FAULTS {
            let mut candidate = report(passed_checks());
            apply_fault(&mut candidate);
            assert_eq!(gate(&candidate, production_use), Some(expected));
        }
        assert_eq!(
            gate(&without_lineage(), production_use),
            Some(ProductionDenialCode::NativeLineageMissing)
        );
        assert_required_check_failure(production_use);
        let io_failure: Result<&ValidationReport, ValidationReportIoError> =
            Err(ValidationReportIoError);
        let decision = validation_report_allows_production(io_failure, production_use);
        assert_eq!(
            decision.denial_code,
            Some(ProductionDenialCode::ValidationReportIo)
        );
        assert!(!decision.allowed());
        assert_eq!(COMPLETE_FAULT_COUNT, 10);
    }
}

fn gate(report: &ValidationReport, production_use: ProductionUse) -> Option<ProductionDenialCode> {
    let decision = validation_report_allows_production(report, production_use);
    assert_eq!(decision.production_use, production_use);
    assert_eq!(decision.allowed(), decision.denial_code.is_none());
    decision.denial_code
}

fn schema_fault(report: &mut ValidationReport) {
    report.schema_version = "unsupported-validation-schema".into();
}

fn identity_fault(report: &mut ValidationReport) {
    report
        .native_lineage
        .as_mut()
        .unwrap()
        .observation
        .dataset_id = "other-dataset".into();
}

fn status_fault(report: &mut ValidationReport) {
    report.status = ValidationStatus::Failed;
    report.production_eligible = false;
}

fn native_fault(report: &mut ValidationReport) {
    report.native_interval = false;
}

fn lineage_fault(report: &mut ValidationReport) {
    report.native_lineage.as_mut().unwrap().lineage.resampled = true;
}

fn checksum_fault(report: &mut ValidationReport) {
    report.content_sha256 = "tampered".into();
}

fn artifact_fault(report: &mut ValidationReport) {
    let artifact = report
        .checks
        .iter_mut()
        .find(|check| check.id == "artifact.required_files")
        .unwrap();
    artifact.status = ValidationStatus::Failed;
    report.content_sha256 =
        ValidationReport::content_hash(&report.normalized_sha256, &report.checks, &report.summary);
}

fn without_lineage() -> ValidationReport {
    let mut report = report(passed_checks());
    report.native_lineage = None;
    report
}

fn assert_required_check_failure(production_use: ProductionUse) {
    let mut candidate = report(passed_checks());
    let check = candidate
        .checks
        .iter_mut()
        .find(|check| check.id != "artifact.required_files")
        .unwrap();
    check.status = ValidationStatus::Failed;
    candidate.content_sha256 = ValidationReport::content_hash(
        &candidate.normalized_sha256,
        &candidate.checks,
        &candidate.summary,
    );
    assert_eq!(
        gate(&candidate, production_use),
        Some(ProductionDenialCode::RequiredValidationCheckFailed)
    );
}
