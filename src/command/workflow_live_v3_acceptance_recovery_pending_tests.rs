//! Evidence loss BEFORE re-freeze must refuse ordinary publication.
use super::super::repair_tests::run_fixture_with;
use super::round_six::{metadata, recover};

fn pending_evidence(mode: u8) {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    let launch = run.set.pin().identity();
    let metadata_path = metadata(&run, &run.run_id, &launch);
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();
    value["schema_version"] = serde_json::json!("workflow-generated-v2-metadata-v1");
    std::fs::write(metadata_path, serde_json::to_vec(&value).unwrap()).unwrap();
    recover(&run);
    let log = run.set.pin_path().with_extension("publish-recovery.log");
    if mode == 0 {
        std::fs::remove_file(&log).unwrap();
    } else if mode == 1 {
        std::fs::write(&log, b"").unwrap();
    } else {
        std::fs::write(
            &log,
            b"{\"event\":\"task_set_publish_recovered\",\"authority\":{}}\n",
        )
        .unwrap();
    }
    let files = [
        run.set.pin_path(),
        run.set
            .tasks
            .join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE),
        run.set
            .tasks
            .join(archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE),
        run.set
            .tasks
            .join(archon_workflow::task_set_contract::TASK_SKELETON_FILE),
        run.set
            .tasks
            .join(archon_workflow::task_set_contract::TASK_SKELETON_LOCK_FILE),
    ];
    let before = files.iter().map(std::fs::read).collect::<Vec<_>>();
    let context = super::super::super::exec::resolve_context(
        &run.store,
        &run.run_id,
        run.runtime.target_repository_root.as_deref(),
        Some(&run.universe),
    )
    .unwrap();
    let result = super::super::super::author::publish_fresh(
        &context,
        &run.set.prd,
        &run.set.contract(),
        None,
    );
    assert!(
        result.is_err(),
        "pending recovery without authenticated evidence must refuse before publication"
    );
    let error = result.err().unwrap().error;
    assert!(error.contains("recovery"), "{error}");
    let after = files.iter().map(std::fs::read).collect::<Vec<_>>();
    for (before, after) in before.into_iter().zip(after) {
        assert_eq!(before.ok(), after.ok(), "live chain bytes must not change");
    }
}
#[test]
fn round3_305_missing_pending_recovery_log() {
    pending_evidence(0);
}
#[test]
fn round3_305_empty_pending_recovery_log() {
    pending_evidence(1);
}
#[test]
fn round3_305_unbound_pending_recovery_log() {
    pending_evidence(2);
}
