//! A pin move no recorded republish explains is proven from the launch
//! contract only for a run whose launch predates lineage recording; a run
//! launched with the marker refuses it as tampering, renewed judgment or not.

use super::*;

#[test]
fn a_legacy_run_proves_an_unrecorded_change_through_the_imported_launch() {
    let fixture = fixture();
    let (launch, current) = launch_and_repair(&fixture);
    fixture.archive(&launch);
    let proof = fixture
        .verify_as(LaunchLineage::Predates, &launch, &current)
        .expect("an unmarked launch may derive the change from the two contracts");
    assert_eq!(proof.recorded_hops, None);
    assert_eq!(proof.changed_ids, BTreeSet::from(["B".to_string()]));
}

#[test]
fn a_marked_run_refuses_an_unrecorded_change_even_with_a_renewed_judgment() {
    let fixture = fixture();
    let (launch, current) = launch_and_repair(&fixture);
    // The launch chain is filed and B carries a renewed, accepted judgment:
    // everything the legacy fallback asks for is present.
    fixture.archive(&launch);
    assert!(fixture.verify(&launch, &current).is_ok());
    let refusal = fixture
        .verify_as(LaunchLineage::Recorded, &launch, &current)
        .unwrap_err();
    assert_eq!(refusal.check, ChainCheck::UnrecordedChange);
    assert!(
        refusal
            .to_string()
            .starts_with("chain check unrecorded_change failed:"),
        "{refusal}"
    );
    assert!(refusal.detail.contains(REAUTHOR_COMMAND), "{refusal}");
    assert!(refusal.detail.contains("--reauthor"), "{refusal}");
}

#[test]
fn a_marked_run_refuses_a_lineage_that_does_not_start_at_its_launch_pin() {
    let fixture = fixture();
    let (launch, mut current) = launch_and_repair(&fixture);
    fixture.archive(&launch);
    let earlier = version(&fixture.tasks, vec![entry("A", "old", "accepted")], "old");
    current.pin.lineage = vec![link(&[], &earlier, &current, &["B"])];
    assert_eq!(
        check_of(fixture.verify_as(LaunchLineage::Recorded, &launch, &current)),
        ChainCheck::UnrecordedChange
    );
}

#[test]
fn a_marked_run_adopts_a_recorded_republish_and_its_own_launch_pin() {
    let fixture = fixture();
    let (launch, mut current) = launch_and_repair(&fixture);
    fixture.publish(&launch);
    assert!(
        fixture
            .verify_as(LaunchLineage::Recorded, &launch, &launch)
            .unwrap()
            .identical
    );
    fixture.publish(&current);
    fixture.archive(&launch);
    current.pin.lineage = vec![link(&[], &launch, &current, &["B"])];
    let proof = fixture
        .verify_as(LaunchLineage::Recorded, &launch, &current)
        .unwrap();
    assert_eq!(proof.recorded_hops, Some(1));
}

#[test]
fn any_marker_binds_the_run_to_recorded_lineage() {
    assert_eq!(LaunchLineage::from_marker(None), LaunchLineage::Predates);
    for marker in [LINEAGE_RECORDING_V1, LINEAGE_RECORDING_V1 + 1] {
        assert_eq!(
            LaunchLineage::from_marker(Some(marker)),
            LaunchLineage::Recorded
        );
    }
}

#[test]
fn the_markers_are_optional_on_the_wire() {
    let fixture = fixture();
    let (launch, _) = launch_and_repair(&fixture);
    let legacy = serde_json::to_value(&launch.pin).unwrap();
    assert!(legacy.get("lineage_recording").is_none(), "{legacy:#}");
    let read: AcceptancePin = serde_json::from_value(legacy).unwrap();
    assert_eq!(read.lineage_recording, None);
    let mut marked = launch.pin.clone();
    marked.lineage_recording = Some(LINEAGE_RECORDING_V1);
    let wire = serde_json::to_value(&marked).unwrap();
    assert_eq!(wire["lineage_recording"], LINEAGE_RECORDING_V1);
    assert_eq!(
        serde_json::from_value::<AcceptancePin>(wire).unwrap(),
        marked
    );
    assert_eq!(marked.identity(), launch.pin.identity());
}
