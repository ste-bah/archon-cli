//! Issues 301/305: authenticate skeleton recovery and name missing evidence.
use super::super::repair_tests::run_fixture_with;
use super::round_six::{metadata, recover, refreeze, verify};
use archon_workflow::task_set_contract::{TASK_SKELETON_FILE, content_digest};

fn changed_skeleton(mode: &str) {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    let launch = run.set.pin().identity();
    metadata(&run, &run.run_id, &launch);
    let history = archon_workflow::task_set_lineage::ChainHistory::for_pin(&run.set.pin_path());
    let path = run.set.tasks.join(TASK_SKELETON_FILE);
    let original = std::fs::read(&path).unwrap();
    history.put(&original).unwrap();
    let mut changed = original.clone();
    match mode {
        "format" => changed.push(b'\n'),
        "authorized" | "authorized-binding" => {
            let mut contract = run.set.contract();
            history
                .put(&serde_json::to_vec_pretty(&contract).unwrap())
                .unwrap();
            if let archon_workflow::task_set_contract::AcceptanceCheck::Command {
                command, ..
            } = &mut contract.acceptance[0].check
            {
                command.push_str(" && true");
            }
            let contract_bytes = serde_json::to_vec_pretty(&contract).unwrap();
            let acceptance = content_digest(&contract_bytes);
            if mode == "authorized" {
                std::fs::write(
                    run.set
                        .tasks
                        .join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE),
                    &contract_bytes,
                )
                .unwrap();
            } else {
                history.put(&contract_bytes).unwrap();
            }
            let mut typed: archon_workflow::task_skeleton::TaskSkeleton =
                serde_json::from_slice(&original).unwrap();
            typed.acceptance_digest = acceptance.clone();
            changed = serde_json::to_vec_pretty(&typed).unwrap();
            history.put(&changed).unwrap();
            let mut later = launch.clone();
            later.freeze_event_id = "authorized-later-launch".into();
            later.acceptance_digest = acceptance;
            later.skeleton_digest = Some(content_digest(&changed));
            metadata(&run, "later-authorized-run", &later);
            changed.push(b'\n');
        }
        "missing" => {}
        _ => unreachable!(),
    }
    if mode == "missing" {
        std::fs::remove_file(&path).unwrap();
    } else {
        std::fs::write(&path, &changed).unwrap();
    }
    std::fs::remove_file(run.set.pin_path()).unwrap();
    let receipt = recover(&run);
    assert!(receipt[0]["prior"].is_null());
    refreeze(&run);
    verify(&run, &run.run_id, &launch)
        .expect("re-freeze must retain the launch-bound skeleton from authenticated history");
    // A self-consistent changed skeleton still fails the launch proof.
    let mut skeleton: archon_workflow::task_skeleton::TaskSkeleton =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    skeleton.tasks[0]
        .implements
        .push("tampered-obligation".into());
    let bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
    let mut pin = run.set.pin();
    pin.skeleton_digest = Some(content_digest(&bytes));
    std::fs::write(&path, bytes).unwrap();
    std::fs::write(run.set.pin_path(), serde_json::to_vec(&pin).unwrap()).unwrap();
    assert!(
        verify(&run, &run.run_id, &launch).is_err(),
        "tampering must remain refused"
    );
}

#[test]
fn issue301_changed_live_format_retains_archived_launch_skeleton() {
    changed_skeleton("format");
}
#[test]
fn issue301_authorized_live_rebinding_retains_launch_skeleton() {
    changed_skeleton("authorized");
}
#[test]
fn issue301_missing_live_skeleton_retains_archived_launch_skeleton() {
    changed_skeleton("missing");
}

#[test]
fn issue305_missing_log_names_evidence_and_remedy() {
    missing_evidence(true, false);
}
#[test]
fn issue305_missing_receipt_names_evidence_and_remedy() {
    missing_evidence(false, true);
}
#[test]
fn issue305_both_missing_names_evidence_and_remedy() {
    missing_evidence(true, true);
}

fn missing_evidence(log: bool, receipt: bool) {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    let launch = run.set.pin().identity();
    metadata(&run, &run.run_id, &launch);
    recover(&run);
    refreeze(&run);
    verify(&run, &run.run_id, &launch).unwrap();
    let log_path = run.set.pin_path().with_extension("publish-recovery.log");
    let receipt_path = run.set.pin_path().with_extension("recovery-lineage");
    if log {
        std::fs::remove_file(&log_path).unwrap();
    }
    if receipt {
        std::fs::remove_file(&receipt_path).unwrap();
    }
    let error = verify(&run, &run.run_id, &launch).unwrap_err();
    if log && receipt {
        assert!(error.contains(&log_path.display().to_string()), "{error}");
    }
    let required = if receipt { &receipt_path } else { &log_path };
    assert!(error.contains(&required.display().to_string()), "{error}");
    assert!(
        error.contains("restore") && error.contains("re-freeze"),
        "{error}"
    );
}

#[test]
fn issue301_unbound_live_skeleton_tampering_is_refused_before_rebinding() {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    let launch = run.set.pin().identity();
    metadata(&run, &run.run_id, &launch);
    let path = run.set.tasks.join(TASK_SKELETON_FILE);
    let bytes = std::fs::read(&path).unwrap();
    archon_workflow::task_set_lineage::ChainHistory::for_pin(&run.set.pin_path())
        .put(&bytes)
        .unwrap();
    let mut skeleton: archon_workflow::task_skeleton::TaskSkeleton =
        serde_json::from_slice(&bytes).unwrap();
    skeleton.tasks[0]
        .implements
        .push("tampered-obligation".into());
    std::fs::write(path, serde_json::to_vec_pretty(&skeleton).unwrap()).unwrap();
    std::fs::remove_file(run.set.pin_path()).unwrap();
    recover(&run);
    let mut prepared = crate::command::workflow_task_set::prepare_from_judged(
        run.set.project.path(),
        &run.set.tasks,
        &run.set.prd,
        archon_core::config::GateMode::Observe,
        &run.set.contract(),
        None,
    )
    .unwrap();
    let error = prepared
        .record_recovery_refreeze()
        .expect_err("unbound semantic tampering must be refused");
    assert!(error.to_string().contains("skeleton_changed"), "{error}");
}

#[test]
fn issue301_authorized_changed_shape_is_selected_from_authenticated_history() {
    let run = run_fixture_with(&[
        ("AC-F-001", "test -f present", true),
        ("AC-F-002", "test -f present", true),
    ]);
    let original = run.set.pin().identity();
    metadata(&run, "a-original-run", &original);
    let history = archon_workflow::task_set_lineage::ChainHistory::for_pin(&run.set.pin_path());
    let path = run.set.tasks.join(TASK_SKELETON_FILE);
    let bytes = std::fs::read(&path).unwrap();
    history.put(&bytes).unwrap();
    let mut skeleton: archon_workflow::task_skeleton::TaskSkeleton =
        serde_json::from_slice(&bytes).unwrap();
    skeleton.tasks.reverse();
    let mut bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
    let mut authorized = original.clone();
    authorized.freeze_event_id = "authorized-shape-freeze".into();
    authorized.skeleton_digest = Some(history.put(&bytes).unwrap().0);
    metadata(&run, "z-authorized-run", &authorized);
    bytes.push(b'\n');
    std::fs::write(path, bytes).unwrap();
    std::fs::remove_file(run.set.pin_path()).unwrap();
    recover(&run);
    refreeze(&run);
    verify(&run, "z-authorized-run", &authorized)
        .expect("the live shape matches the newer authorized launch, not directory ordering");
    assert!(
        verify(&run, "a-original-run", &original).is_err(),
        "an incompatible older skeleton is still refused"
    );
}

#[test]
fn issue301_authorized_skeleton_binding_is_rebound_to_selected_contract() {
    changed_skeleton("authorized-binding");
}
