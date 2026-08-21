use super::*;

pub(super) fn run() {
    rejects_unavailable_malformed_mismatched_and_contradictory_evidence();
    rejects_every_derived_lineage_independently();
    ignores_descriptive_metadata_when_typed_evidence_is_valid();
}

fn rejects_unavailable_malformed_mismatched_and_contradictory_evidence() {
    let mut value = serde_json::to_value(report(passed_checks())).unwrap();
    value.as_object_mut().unwrap().remove("native_lineage");
    assert!(serde_json::from_value::<ValidationReport>(value).is_err());

    let mut malformed = serde_json::to_value(report(passed_checks())).unwrap();
    malformed["native_lineage"]["observation"]["retrieved_at"] = "not-a-time".into();
    assert!(serde_json::from_value::<ValidationReport>(malformed).is_err());

    let mut mismatched = report(passed_checks());
    mismatched
        .native_lineage
        .as_mut()
        .unwrap()
        .observation
        .dataset_id = "other-dataset".into();
    assert!(!mismatched.allows_production());

    let mut contradictory = report(passed_checks());
    contradictory
        .native_lineage
        .as_mut()
        .unwrap()
        .observation
        .exact_native_interval = false;
    assert!(!contradictory.allows_production());
}

fn rejects_every_derived_lineage_independently() {
    let mutations: &[fn(&mut DerivationLineage)] = &[
        |lineage| lineage.aggregated = true,
        |lineage| lineage.resampled = true,
        |lineage| lineage.downsampled = true,
        |lineage| lineage.upsampled = true,
        |lineage| lineage.interpolated = true,
        |lineage| lineage.synthesized = true,
    ];
    for mutate in mutations {
        let mut candidate = report(passed_checks());
        mutate(&mut candidate.native_lineage.as_mut().unwrap().lineage);
        assert!(!candidate.allows_production());
    }
}

fn ignores_descriptive_metadata_when_typed_evidence_is_valid() {
    let bars = vec![
        bar("2026-01-01T00:00:00Z", 10.0, 100.0),
        bar("2026-01-02T00:00:00Z", 11.0, 110.0),
    ];
    let mut metadata = complete_metadata(&bars);
    metadata.dataset_id = "manual-BTCUSD-1D-resampled".into();
    metadata.price_basis = "resampled".into();
    metadata.quality_status = "passed".into();
    let candidate = validation_report(&metadata, &bars, metadata.created_at.clone());
    assert!(candidate.allows_production(), "{candidate:#?}");

    metadata.native_interval = false;
    let candidate = validation_report(&metadata, &bars, metadata.created_at.clone());
    assert!(!candidate.allows_production());
    assert_failed_check(&candidate, "metadata.native_observation_evidence");
}
