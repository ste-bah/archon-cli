//! Observation records retain floor failures and defer commands.
use super::*;

#[test]
fn failed_commandless_floor_writes_observe_only_shadow() {
    let fixture = frozen_fixture(vec![criterion("AC-X-001", floor("missing.json"))]);
    let run = fixture.store.create_run(finalizer_spec()).unwrap();
    let observer = FixedRunEndAcceptanceObserver::new(fixture.store.clone());
    let outcome = observer.observe(&context(&fixture, &run.id)).unwrap();

    assert_eq!(outcome.authority, ObserverAuthority::ObserveOnly);
    assert_eq!(outcome.evaluated_floor_count, 1);
    assert_eq!(outcome.policy_finding_count, 1);
    assert_eq!(outcome.operational_deferral_count, 0);
    let rows = records(&fixture, &run.id);
    assert_eq!(rows[0]["record_kind"], "policy_shadow");
    assert_eq!(rows[0]["acceptance_id"], "AC-X-001");
    assert_eq!(rows[0]["authority"], "observe_only");
}

#[test]
fn command_check_is_deferred_and_never_executed() {
    let project = tempfile::tempdir().unwrap();
    let sentinel = project.path().join("must-not-exist");
    let fixture = frozen_fixture(vec![criterion(
        "AC-X-001",
        command(format!("touch {}", sentinel.display())),
    )]);
    let run = fixture.store.create_run(finalizer_spec()).unwrap();
    let observer = FixedRunEndAcceptanceObserver::new(fixture.store.clone());

    let outcome = observer.observe(&context(&fixture, &run.id)).unwrap();

    assert_eq!(outcome.evaluated_floor_count, 0);
    assert_eq!(outcome.operational_deferral_count, 1);
    assert!(!sentinel.exists());
    let encoded = serde_json::to_string(&records(&fixture, &run.id)).unwrap();
    assert!(!encoded.contains("touch"));
    assert!(!encoded.contains("must-not-exist"));
}
