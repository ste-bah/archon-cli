use super::*;

fn failed_at(severity: ValidationSeverity) -> Vec<ValidationCheck> {
    let mut checks = passed_checks();
    checks[0].status = ValidationStatus::Failed;
    checks[0].severity = severity;
    checks
}

fn assert_wire_rejected(mut value: serde_json::Value, field: &str, replacement: serde_json::Value) {
    value[field] = replacement;
    assert!(
        serde_json::from_value::<ValidationReport>(value).is_err(),
        "contradictory {field} must be rejected"
    );
}

#[test]
fn validation_status_precedence_is_fail_closed() {
    for (checks, expected_status, expected_wire_status, expected_eligible) in [
        (
            failed_at(ValidationSeverity::Error),
            ValidationStatus::Failed,
            "failed",
            false,
        ),
        (
            failed_at(ValidationSeverity::Warning),
            ValidationStatus::Degraded,
            "degraded",
            false,
        ),
        (
            failed_at(ValidationSeverity::Info),
            ValidationStatus::Passed,
            "passed",
            true,
        ),
        (passed_checks(), ValidationStatus::Passed, "passed", true),
    ] {
        assert_eq!(
            ValidationReport::status_from_checks(&checks),
            expected_status
        );
        let wire = serde_json::to_value(report(checks)).unwrap();
        assert_eq!(wire["status"], expected_wire_status);
        assert_eq!(wire["production_eligible"], expected_eligible);
        let parsed: ValidationReport = serde_json::from_value(wire).unwrap();
        assert_eq!(parsed.status, expected_status);
        assert_eq!(parsed.production_eligible, expected_eligible);
    }

    let passing = serde_json::to_value(report(passed_checks())).unwrap();
    assert_wire_rejected(passing.clone(), "status", serde_json::json!("degraded"));
    assert_wire_rejected(passing, "production_eligible", serde_json::json!(false));

    let failed = serde_json::to_value(report(failed_at(ValidationSeverity::Error))).unwrap();
    assert_wire_rejected(failed.clone(), "status", serde_json::json!("passed"));
    assert_wire_rejected(failed, "production_eligible", serde_json::json!(true));
}
