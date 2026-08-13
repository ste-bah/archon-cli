use super::*;

fn zero_bars() -> Vec<OhlcvBar> {
    vec![
        bar("2026-01-01T00:00:00Z", 10.0, 0.0),
        bar("2026-01-02T00:00:00Z", 11.0, 0.0),
    ]
}

fn exact_evidence(metadata: &DatasetMetadata) -> VolumeAbsenceEvidence {
    VolumeAbsenceEvidence {
        provider: metadata.provider.clone(),
        canonical_instrument: metadata.canonical_instrument.clone(),
        provider_symbol: metadata.provider_symbol.clone(),
        timeframe: metadata.timeframe.clone(),
        source_action: "fetch_ohlcv_native".into(),
        retrieved_at: metadata.source.retrieved_at.clone(),
        evidence_path: metadata.paths.raw_request.clone(),
        volume_field_present: false,
    }
}

fn volume_checks_pass(report: &ValidationReport) -> bool {
    ["ohlcv.volume_presence", "ohlcv.volume"].iter().all(|id| {
        report
            .checks
            .iter()
            .any(|check| check.id == *id && check.status == ValidationStatus::Passed)
    })
}

#[test]
fn validation_volume_exemption_requires_exact_evidence() {
    let bars = zero_bars();
    let mut metadata = complete_metadata(&bars);
    metadata.source.retrieved_at = "2026-01-01T00:00:00Z".into();
    metadata.paths.raw_request = "raw/request.json".into();
    let evidence = exact_evidence(&metadata);

    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("raw")).unwrap();
    write_json(
        &temp.path().join(&metadata.paths.raw_request),
        &serde_json::json!({"volume_absence_evidence": evidence}),
    )
    .unwrap();
    assert_eq!(
        load_volume_absence_evidence(temp.path(), &metadata),
        Some(evidence.clone())
    );

    write_json(
        &temp.path().join(&metadata.paths.raw_request),
        &serde_json::json!({"provider_notes": "volume field absent"}),
    )
    .unwrap();
    assert_eq!(load_volume_absence_evidence(temp.path(), &metadata), None);

    let report = validation_report_with_volume_evidence(
        &metadata,
        &bars,
        metadata.created_at.clone(),
        Some(&evidence),
    );
    assert!(volume_checks_pass(&report));

    for mutate in [
        |value: &mut VolumeAbsenceEvidence| value.provider.push_str("-other"),
        |value: &mut VolumeAbsenceEvidence| value.canonical_instrument.push_str("-other"),
        |value: &mut VolumeAbsenceEvidence| value.provider_symbol.push_str("-other"),
        |value: &mut VolumeAbsenceEvidence| value.timeframe.push_str("-other"),
        |value: &mut VolumeAbsenceEvidence| value.source_action = "notes".into(),
        |value: &mut VolumeAbsenceEvidence| value.retrieved_at = "2025-01-01T00:00:00Z".into(),
        |value: &mut VolumeAbsenceEvidence| value.retrieved_at = "invalid".into(),
        |value: &mut VolumeAbsenceEvidence| value.evidence_path = "../unsafe.json".into(),
        |value: &mut VolumeAbsenceEvidence| value.volume_field_present = true,
    ] {
        let mut mismatched = evidence.clone();
        mutate(&mut mismatched);
        let report = validation_report_with_volume_evidence(
            &metadata,
            &bars,
            metadata.created_at.clone(),
            Some(&mismatched),
        );
        assert!(!volume_checks_pass(&report));
    }

    let report = validation_report(&metadata, &bars, metadata.created_at.clone());
    assert!(!volume_checks_pass(&report));

    let positive_zero = zero_bars();
    assert!(!volume_is_non_degenerate(&positive_zero));
    assert!(!volume_checks_pass(&validation_report(
        &metadata,
        &positive_zero,
        metadata.created_at.clone(),
    )));
}
