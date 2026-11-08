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

fn advanced_fixture(typed: bool, constant_steps: bool) -> FrozenFixture {
    let mut contract: WorkflowV2DeliverableContract = serde_json::from_value(serde_json::json!({
        "kind":"evidence", "artifact_path":"matrix.json", "required_universe":true,
        "universe_fields":["axis"], "cells_field":"cells", "cell_identity_fields":["id"],
        "registry_path":"registry.json", "registry_records_field":"records", "registry_key_fields":["id"],
        "data_kind":"record_series", "payload_path_field":"payload", "required_fields":["x"],
        "non_constant_fields":["x"], "series_value_fields":["x"]
    })).unwrap();
    if typed {
        contract.typed_verifier_command = Some("test -s {artifact_path}".into());
    }
    let fixture = frozen_fixture(vec![criterion(
        "AC-X-001",
        AcceptanceCheck::Floor {
            contract: Box::new(contract),
        },
    )]);
    for (path, value) in [
        (
            "matrix.json",
            serde_json::json!({"axis":["a"],"cells":[{"id":"a"}]}),
        ),
        (
            "registry.json",
            serde_json::json!({"records":{"a":{"payload":"payload.json"}}}),
        ),
        (
            "payload.json",
            if constant_steps {
                serde_json::json!([{"x":1},{"x":2},{"x":3}])
            } else {
                serde_json::json!([{"x":1},{"x":2},{"x":4}])
            },
        ),
    ] {
        std::fs::write(fixture.project.path().join(path), value.to_string()).unwrap();
    }
    fixture
}

#[test]
fn advanced_payload_and_series_floor_passes_natively() {
    let fixture = advanced_fixture(false, false);
    let run = fixture.store.create_run(finalizer_spec()).unwrap();
    let outcome = FixedRunEndAcceptanceObserver::new(fixture.store.clone())
        .observe(&context(&fixture, &run.id))
        .unwrap();
    assert_eq!(outcome.evaluated_floor_count, 1);
    assert_eq!(outcome.operational_deferral_count, 0);
    assert_eq!(outcome.policy_finding_count, 0);
}

#[test]
fn advanced_payload_and_series_floor_fails_natively() {
    let fixture = advanced_fixture(false, true);
    let run = fixture.store.create_run(finalizer_spec()).unwrap();
    let outcome = FixedRunEndAcceptanceObserver::new(fixture.store.clone())
        .observe(&context(&fixture, &run.id))
        .unwrap();
    assert_eq!(outcome.evaluated_floor_count, 1);
    assert_eq!(outcome.operational_deferral_count, 0);
    assert_eq!(outcome.policy_finding_count, 1);
    assert!(
        serde_json::to_string(&records(&fixture, &run.id))
            .unwrap()
            .contains("constant first difference")
    );
}

#[test]
fn advanced_floor_with_typed_command_still_defers() {
    let fixture = advanced_fixture(true, false);
    let run = fixture.store.create_run(finalizer_spec()).unwrap();
    let outcome = FixedRunEndAcceptanceObserver::new(fixture.store.clone())
        .observe(&context(&fixture, &run.id))
        .unwrap();
    assert_eq!(outcome.evaluated_floor_count, 0);
    assert_eq!(outcome.operational_deferral_count, 1);
    assert!(serde_json::to_string(&records(&fixture,&run.id)).unwrap().contains("floor with a typed verifier command deferred: the run-end observer never runs commands"));
}
