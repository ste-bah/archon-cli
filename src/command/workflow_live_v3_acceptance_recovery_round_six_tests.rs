//! Recovery adoption after incomplete run discovery and multiple launches.
use super::*;
use archon_workflow::PortableAcceptanceIdentityV1;
use archon_workflow::task_set_lineage::LaunchLineage;
const TXN: &str = "aabbccddeeff00112233445566778899";

fn metadata(run: &Run, id: &str, launch: &PortableAcceptanceIdentityV1) -> std::path::PathBuf {
    let path = run.store.run_dir(id).join("v2/generated-metadata.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, serde_json::json!({"observer_snapshot": {
        "schema_version": 1,
        "canonical_task_root_identity": run.set.tasks.canonicalize().map(archon_shell::paths::plain).unwrap(),
        "expected_artifact_paths": [], "portable_acceptance_identity": launch,
        "lineage_recording": 1
    }}).to_string()).unwrap();
    path
}

fn recover(run: &Run) -> serde_json::Value {
    let lock = run.set.tasks.join(ACCEPTANCE_LOCK_FILE);
    std::fs::write(
        lock.with_file_name(format!(".{ACCEPTANCE_LOCK_FILE}.{TXN}.old")),
        b"invalid backup",
    )
    .unwrap();
    std::fs::write(&lock, b"invalid live lock").unwrap();
    crate::command::workflow_task_set::recover_interrupted_publish(
        &run.set.pin_path(),
        &run.set.tasks,
    )
    .unwrap();
    assert!(
        !run.set.pin_path().exists(),
        "recovery must actually unfreeze"
    );
    let path = crate::command::workflow_task_set::recovery_lineage::path(&run.set.pin_path());
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn refreeze(run: &Run) {
    let mut contract = run.set.contract();
    let original = contract.clone();
    if let archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } =
        &mut contract.acceptance[0].check
    {
        command.push_str(" && test -s present");
    }
    let mut prepared = crate::command::workflow_task_set::prepare_from_judged(
        run.set.project.path(),
        &run.set.tasks,
        &run.set.prd,
        archon_core::config::GateMode::Observe,
        &contract,
    )
    .unwrap();
    prepared.record_recovery_refreeze().unwrap();
    let findings = prepared.findings.clone();
    let identity = prepared.publication_identity();
    let mut gate = crate::command::workflow_gate::run_sync_gate(
        run.set.project.path(),
        archon_core::config::GateMode::Observe,
        crate::command::workflow_gate::GateId::FreezeAcceptance,
        || {
            Ok(
                crate::command::workflow_gate::GateEvaluation::new("", findings)
                    .with_publication_identity(identity),
            )
        },
    )
    .unwrap();
    crate::command::workflow_task_set::publish_acceptance_freeze(
        prepared,
        gate.take_publication_permit().unwrap(),
    )
    .unwrap();
    assert!(
        !run.set.pin().lineage.is_empty(),
        "re-freeze must publish recovery lineage"
    );
    assert_ne!(
        run.set.contract(),
        original,
        "re-freeze must change the contract"
    );
}

fn verify(
    run: &Run,
    id: &str,
    launch: &PortableAcceptanceIdentityV1,
) -> Result<archon_workflow::task_set_lineage::ChainProof, String> {
    crate::command::acceptance_chain::verify_launch_chain(
        launch,
        LaunchLineage::Recorded,
        &run.set.pin(),
        &run.set.pin_path(),
        &run.set.tasks,
        id,
    )
}

#[test]
fn skipped_unfreeze_run_with_consistent_launch_evidence_is_adopted() {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    let launch = run.set.pin().identity();
    let path = metadata(&run, &run.run_id, &launch);
    let original = std::fs::read(&path).unwrap();
    std::fs::write(&path, b"temporarily unreadable snapshot").unwrap();
    let receipt = recover(&run);
    assert!(
        receipt[0]["runs"].as_object().unwrap().is_empty(),
        "the run must actually be skipped"
    );
    assert_eq!(receipt[0]["skipped_runs"], serde_json::json!([run.run_id]));
    std::fs::write(&path, original).unwrap();
    refreeze(&run);
    let proof = verify(&run, &run.run_id, &launch)
        .expect("consistent launch evidence for a skipped run must adopt recovery");
    assert!(!proof.identical);
    assert!(verify(&run, "unknown-run", &launch).is_err());
    metadata(&run, "later-run", &launch);
    assert!(
        verify(&run, "later-run", &launch).is_err(),
        "a run created after unfreeze cannot acquire its authority"
    );
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    value["observer_snapshot"]["canonical_task_root_identity"] =
        run.set.project.path().display().to_string().into();
    std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(
        verify(&run, &run.run_id, &launch).is_err(),
        "a skipped run still has to bind the recovered task root"
    );
}

#[test]
fn no_prior_pin_recovery_adopts_both_distinct_run_launches() {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    let first = run.set.pin().identity();
    let mut second = first.clone();
    second.freeze_event_id = "second-launch-freeze".into();
    metadata(&run, "a-first-run", &first);
    metadata(&run, "z-second-run", &second);
    std::fs::remove_file(run.set.pin_path()).unwrap();
    let receipt = recover(&run);
    assert!(receipt[0]["prior"].is_null());
    assert_eq!(receipt[0]["runs"].as_object().unwrap().len(), 2);
    assert_ne!(
        receipt[0]["runs"]["a-first-run"],
        receipt[0]["runs"]["z-second-run"]
    );
    refreeze(&run);
    verify(&run, "a-first-run", &first).expect("first run must adopt recovery");
    verify(&run, "z-second-run", &second)
        .expect("second distinct launch must also adopt the shared recovery");
    let mut pin = run.set.pin();
    let from = pin.identity();
    pin.freeze_event_id = "ordinary-followup-freeze".into();
    pin.lineage
        .push(archon_workflow::task_set_lineage::PinTransition::extending(
            &pin.lineage,
            from,
            pin.identity(),
            Default::default(),
            "ordinary followup",
        ));
    std::fs::write(run.set.pin_path(), serde_json::to_vec(&pin).unwrap()).unwrap();
    let proof =
        verify(&run, "z-second-run", &second).expect("later recorded hops must remain adoptable");
    assert_eq!(proof.recorded_hops, Some(2));
    for broken in ["digest", "source", "destination"] {
        let mut broken_pin = pin.clone();
        let link = broken_pin.lineage.last_mut().unwrap();
        match broken {
            "digest" => link.prior_link_digest = Some("broken link".into()),
            "source" => link.from.freeze_event_id = "chain gap".into(),
            _ => link.to.freeze_event_id = "wrong destination".into(),
        }
        std::fs::write(run.set.pin_path(), serde_json::to_vec(&broken_pin).unwrap()).unwrap();
        let refusal = verify(&run, "z-second-run", &second).unwrap_err();
        assert!(refusal.contains("lineage_broken"), "{broken}: {refusal}");
    }
}

#[test]
fn no_prior_pin_recovery_selects_the_run_with_surviving_preimages() {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    let second = run.set.pin().identity();
    let mut first = second.clone();
    first.freeze_event_id = "older-launch-freeze".into();
    first.acceptance_digest =
        archon_workflow::task_set_contract::content_digest(b"unavailable launch contract");
    metadata(&run, "a-first-run", &first);
    metadata(&run, "z-second-run", &second);
    std::fs::remove_file(run.set.pin_path()).unwrap();
    let receipt = recover(&run);
    assert!(receipt[0]["prior"].is_null());
    assert_eq!(receipt[0]["runs"].as_object().unwrap().len(), 2);
    let base = crate::command::workflow_task_set::recovery_lineage::refreeze_base(
        &run.set.pin_path(),
        &run.set.tasks,
    )
    .unwrap()
    .expect("the second run's authenticated contract must be selected");
    assert_eq!(base, run.set.contract());
    refreeze(&run);
    verify(&run, "z-second-run", &second).expect("the run with surviving evidence must adopt");
    assert!(verify(&run, "a-first-run", &first).is_err());
}

#[test]
fn distinct_recovery_launch_still_requires_consistent_contract_preimages() {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    let first = run.set.pin().identity();
    let mut contract = run.set.contract();
    contract.acceptance[0].criterion = "different launch obligation".into();
    let bytes = serde_json::to_vec_pretty(&contract).unwrap();
    let mut second = first.clone();
    second.freeze_event_id = "incompatible-launch-freeze".into();
    second.acceptance_digest = archon_workflow::task_set_contract::content_digest(&bytes);
    archon_workflow::task_set_lineage::ChainHistory::for_pin(&run.set.pin_path())
        .put(&bytes)
        .unwrap();
    metadata(&run, "a-first-run", &first);
    metadata(&run, "z-second-run", &second);
    std::fs::remove_file(run.set.pin_path()).unwrap();
    recover(&run);
    refreeze(&run);
    verify(&run, "a-first-run", &first).unwrap();
    let refusal = verify(&run, "z-second-run", &second).unwrap_err();
    assert!(refusal.contains("criterion_changed"), "{refusal}");
}

#[test]
fn attack_anchor_flips_when_contract_digest_is_unbound() {
    anchor_import_with_unbound_contract(false);
}

#[test]
fn recovery_import_before_completion_cannot_block_a_valid_launch() {
    anchor_import_with_unbound_contract(true);
}

fn anchor_import_with_unbound_contract(import_before: bool) {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    let mut second = run.set.pin().identity();
    second.skeleton_digest = None;
    let mut contract = run.set.contract();
    contract.acceptance[0].criterion = "older launch obligation".into();
    let older = serde_json::to_vec_pretty(&contract).unwrap();
    let mut first = second.clone();
    first.freeze_event_id = "older-launch-freeze".into();
    first.acceptance_digest = archon_workflow::task_set_contract::content_digest(&older);
    metadata(&run, "a-first-run", &first);
    metadata(&run, "z-second-run", &second);
    std::fs::remove_file(run.set.pin_path()).unwrap();
    // The interrupted publish left a contract and skeleton no launch names.
    let history = archon_workflow::task_set_lineage::ChainHistory::for_pin(&run.set.pin_path());
    let live = run.set.tasks.join(ACCEPTANCE_CONTRACT_FILE);
    let mut bytes = std::fs::read(&live).unwrap();
    history.put(&bytes).unwrap();
    bytes.push(b'\n');
    std::fs::write(&live, bytes).unwrap();
    for name in [
        archon_workflow::task_set_contract::TASK_SKELETON_FILE,
        archon_workflow::task_set_contract::TASK_SKELETON_LOCK_FILE,
    ] {
        std::fs::remove_file(run.set.tasks.join(name)).unwrap();
    }
    let receipt = recover(&run);
    assert!(receipt[0]["prior"].is_null());
    assert!(receipt[0]["contract_digest"].is_null());
    assert!(receipt[0]["skeleton"].is_null());
    if import_before {
        history.put(&older).unwrap();
    }
    refreeze(&run);
    if import_before {
        assert_eq!(run.set.pin().lineage.last().unwrap().from, first);
    }
    let before = verify(&run, "z-second-run", &second);
    assert!(
        before.is_ok(),
        "z must adopt regardless of import timing (import_before={import_before}): {before:?}"
    );
    assert!(
        verify(&run, "uncaptured-run", &second)
            .unwrap_err()
            .contains("no matching durable launch authorization"),
        "a valid launch proof cannot authorize an uncaptured run"
    );
    history.put(&older).unwrap();
    let after = verify(&run, "z-second-run", &second);
    assert!(
        after.is_ok(),
        "ATTACK: z refused after a later preimage import: {after:?}"
    );
    let refusal = verify(&run, "a-first-run", &first).unwrap_err();
    assert!(refusal.contains("criterion_changed"), "{refusal}");
    // A captured run with compatible fields still needs its own authentic bytes.
    let preimage = history.path(&second.acceptance_digest);
    let original = std::fs::read(&preimage).unwrap();
    std::fs::write(&preimage, b"corrupt own launch contract").unwrap();
    let refusal = verify(&run, "z-second-run", &second).unwrap_err();
    assert!(refusal.contains("preimage_corrupt"), "{refusal}");
    std::fs::write(&preimage, original).unwrap();
    verify(&run, "z-second-run", &second).expect("restoring its own preimage permits adoption");
}

#[test]
fn readable_snapshot_without_launch_identity_cannot_acquire_recovery_authority() {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    let launch = run.set.pin().identity();
    metadata(&run, "captured-run", &launch);
    let id = "run-without-launch";
    let path = metadata(&run, id, &launch);
    let mut snapshot: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    snapshot["observer_snapshot"]["portable_acceptance_identity"] = serde_json::Value::Null;
    std::fs::write(&path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
    let receipt = recover(&run);
    assert!(receipt[0]["runs"].get(id).is_none());
    assert!(
        !receipt[0]["skipped_runs"]
            .as_array()
            .is_some_and(|runs| runs.contains(&serde_json::json!(id))),
        "a readable snapshot without a launch identity must not be skipped: {:?}",
        receipt[0]["skipped_runs"]
    );
    metadata(&run, id, &launch);
    refreeze(&run);
    verify(&run, "captured-run", &launch).expect("the captured launch must still adopt");
    assert!(
        verify(&run, id, &launch).is_err(),
        "adding an identity after unfreeze cannot grant launch authority"
    );
}

#[test]
fn recovery_completion_source_must_bind_the_captured_authority() {
    for (has_prior, capture_other) in [(false, false), (false, true), (true, true)] {
        let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
        let launch = run.set.pin().identity();
        metadata(&run, "captured-run", &launch);
        let mut other = launch.clone();
        other.freeze_event_id = "different-source-freeze".into();
        if capture_other {
            metadata(&run, "other-captured-run", &other);
        }
        if !has_prior {
            std::fs::remove_file(run.set.pin_path()).unwrap();
        }
        let receipt = recover(&run);
        assert_eq!(!receipt[0]["prior"].is_null(), has_prior);
        refreeze(&run);
        verify(&run, "captured-run", &launch).expect("the original completion must adopt");

        replace_completion_source(&run, other.clone());
        if !has_prior && capture_other {
            // Without a prior pin, either captured source for the bound contract
            // is authorized; each requesting run must still pass its own proof.
            verify(&run, "captured-run", &launch).expect("another captured source is authorized");
            verify(&run, "other-captured-run", &other).expect("the other run also proves adoption");
            continue;
        }
        let refusal = verify(&run, "captured-run", &launch).unwrap_err();
        assert!(
            refusal.contains("recovery completion does not bind its prior anchor"),
            "{refusal}"
        );
    }
}

fn replace_completion_source(run: &Run, from: PortableAcceptanceIdentityV1) {
    // Keep the pin and receipt consistent, changing only their claimed source.
    let mut pin = run.set.pin();
    pin.lineage.last_mut().unwrap().from = from;
    std::fs::write(run.set.pin_path(), serde_json::to_vec(&pin).unwrap()).unwrap();
    let path = crate::command::workflow_task_set::recovery_lineage::path(&run.set.pin_path());
    let mut receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    receipt[0]["completed"]["lineage"] = serde_json::to_value(&pin.lineage).unwrap();
    std::fs::write(path, serde_json::to_vec(&receipt).unwrap()).unwrap();
}

#[test]
fn no_prior_completion_source_must_match_the_captured_contract_digest() {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    let launch = run.set.pin().identity();
    let mut older_contract = run.set.contract();
    older_contract.acceptance[0].criterion = "older obligation".into();
    let bytes = serde_json::to_vec_pretty(&older_contract).unwrap();
    let mut other = launch.clone();
    other.freeze_event_id = "older-source-freeze".into();
    other.acceptance_digest = archon_workflow::task_set_contract::content_digest(&bytes);
    metadata(&run, "captured-run", &launch);
    metadata(&run, "other-captured-run", &other);
    std::fs::remove_file(run.set.pin_path()).unwrap();
    let receipt = recover(&run);
    assert_eq!(receipt[0]["contract_digest"], launch.acceptance_digest);
    refreeze(&run);
    verify(&run, "captured-run", &launch).unwrap();
    replace_completion_source(&run, other);
    let refusal = verify(&run, "captured-run", &launch).unwrap_err();
    assert!(
        refusal.contains("recovery completion does not bind its prior anchor"),
        "{refusal}"
    );
}
