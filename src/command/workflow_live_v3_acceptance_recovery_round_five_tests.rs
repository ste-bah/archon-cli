//! Recovery authority, coverage and the actual generated-workflow resume.
use super::super::repair_tests::{Run, run_fixture_with};
use super::*;
use archon_workflow::task_set_contract::{AcceptanceLock, TASK_SKELETON_FILE};
use archon_workflow::task_set_lineage::{LaunchLineage, PinTransition};
use archon_workflow::task_skeleton::{TaskSkeleton, TaskSkeletonLock};
const TXN: &str = "aabbccddeeff00112233445566778899";

fn recover(run: &Run) {
    let metadata = run
        .store
        .run_dir(&run.run_id)
        .join("v2/generated-metadata.json");
    std::fs::create_dir_all(metadata.parent().unwrap()).unwrap();
    std::fs::write(&metadata, serde_json::json!({
        "schema_version": "test", "observer_snapshot": {
            "schema_version": 1,
            "canonical_task_root_identity": run.set.tasks.canonicalize().map(archon_shell::paths::plain).unwrap(),
            "expected_artifact_paths": [], "portable_acceptance_identity": run.set.pin().identity(), "lineage_recording": 1
        }
    }).to_string()).unwrap();
    let lock = run.set.tasks.join(ACCEPTANCE_LOCK_FILE);
    std::fs::write(
        lock.with_file_name(format!(".{ACCEPTANCE_LOCK_FILE}.{TXN}.old")),
        b"invalid rollback backup",
    )
    .unwrap();
    std::fs::write(&lock, b"invalid live lock").unwrap();
    crate::command::workflow_task_set::recover_interrupted_publish(
        &run.set.pin_path(),
        &run.set.tasks,
    )
    .unwrap();
}

fn receipt(run: &Run) -> (std::path::PathBuf, serde_json::Value) {
    let path = crate::command::workflow_task_set::recovery_lineage::path(&run.set.pin_path());
    let value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    (path, value)
}

fn verify(
    run: &Run,
    launch: &archon_workflow::PortableAcceptanceIdentityV1,
) -> std::result::Result<archon_workflow::task_set_lineage::ChainProof, String> {
    crate::command::acceptance_chain::verify_launch_chain(
        launch,
        LaunchLineage::Recorded,
        &run.set.pin(),
        &run.set.pin_path(),
        &run.set.tasks,
        &run.run_id,
    )
}

#[tokio::test]
async fn recovery_receipt_requires_the_durable_unfreeze_log() {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    let launch = run.set.pin().identity();
    pre_implementation_head(&run);
    recover(&run);
    stage(&run, &accepting()).await;
    verify(&run, &launch).expect("the untampered recovery must be adoptable");
    let log = run.set.pin_path().with_extension("publish-recovery.log");
    std::fs::remove_file(log).unwrap();
    assert!(
        verify(&run, &launch).is_err(),
        "a standalone receipt cannot authorize adoption"
    );
}

#[tokio::test]
async fn recovery_receipt_binds_prior_pin_and_run_anchors() {
    for field in ["prior", "runs"] {
        let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
        let launch = run.set.pin().identity();
        pre_implementation_head(&run);
        recover(&run);
        stage(&run, &accepting()).await;
        verify(&run, &launch).expect("the untampered recovery must be adoptable");
        let (path, mut value) = receipt(&run);
        if field == "prior" {
            value[0]["prior"]["freeze_event_id"] = "forged-prior".into();
        } else {
            value[0]["runs"]["unrelated-run"] = serde_json::to_value(&launch).unwrap();
        }
        std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(
            verify(&run, &launch).is_err(),
            "forged {field} must not authorize adoption"
        );
    }
}

#[tokio::test]
async fn recovery_receipt_still_checks_launch_contract_and_skeleton() {
    for changed in ["contract", "skeleton"] {
        let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
        let launch = run.set.pin().identity();
        pre_implementation_head(&run);
        recover(&run);
        stage(&run, &accepting()).await;
        verify(&run, &launch).expect("the untampered recovery must be adoptable");
        let mut contract = run.set.contract();
        if changed == "contract" {
            contract.acceptance[0].criterion = "forged obligation".into();
        }
        let bytes = serde_json::to_vec_pretty(&contract).unwrap();
        std::fs::write(run.set.tasks.join(ACCEPTANCE_CONTRACT_FILE), &bytes).unwrap();
        let mut pin = run.set.pin();
        pin.acceptance_digest = archon_workflow::task_set_contract::content_digest(&bytes);
        let lock_path = run.set.tasks.join(ACCEPTANCE_LOCK_FILE);
        let mut lock: AcceptanceLock =
            serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
        lock.digest = pin.acceptance_digest.clone();
        std::fs::write(lock_path, serde_json::to_vec(&lock).unwrap()).unwrap();
        let skeleton_path = run.set.tasks.join(TASK_SKELETON_FILE);
        let mut skeleton: TaskSkeleton =
            serde_json::from_slice(&std::fs::read(&skeleton_path).unwrap()).unwrap();
        skeleton.acceptance_digest = pin.acceptance_digest.clone();
        if changed == "skeleton" {
            skeleton.tasks[0]
                .implements
                .push("forged requirement".into());
        }
        let bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
        pin.skeleton_digest = Some(archon_workflow::task_set_contract::content_digest(&bytes));
        std::fs::write(skeleton_path, bytes).unwrap();
        let lock = TaskSkeletonLock {
            algorithm: "blake3".into(),
            digest: pin.skeleton_digest.clone().unwrap(),
            acceptance_digest: pin.acceptance_digest.clone(),
            gate: pin.skeleton_gate.clone().unwrap(),
        };
        std::fs::write(
            run.set.tasks.join(TASK_SKELETON_LOCK_FILE),
            serde_json::to_vec(&lock).unwrap(),
        )
        .unwrap();
        pin.check_sources_digest = None;
        let ids = ["AC-F-001".to_string()].into_iter().collect();
        pin.lineage = vec![PinTransition::extending(
            &[],
            launch.clone(),
            pin.identity(),
            ids,
            &format!("recovery-refreeze:{TXN}"),
        )];
        std::fs::write(run.set.pin_path(), serde_json::to_vec(&pin).unwrap()).unwrap();
        let (path, mut value) = receipt(&run);
        value[0]["completed"]["identity"] = serde_json::to_value(pin.identity()).unwrap();
        value[0]["completed"]["lineage"] = serde_json::to_value(&pin.lineage).unwrap();
        std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(
            verify(&run, &launch).is_err(),
            "a self-consistent forged {changed} must fail the launch checks"
        );
    }
}

#[test]
fn recovery_refreeze_reruns_skeleton_coverage() {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    recover(&run);
    let mut contract = run.set.contract();
    contract.acceptance[0].id = "AC-F-002".into();
    let mut prepared = crate::command::workflow_task_set::prepare_from_judged(
        run.set.project.path(),
        &run.set.tasks,
        &run.set.prd,
        archon_core::config::GateMode::Enforce,
        &contract,
        None,
    )
    .unwrap();
    prepared.record_recovery_refreeze().unwrap();
    assert!(
        prepared
            .findings
            .iter()
            .any(|finding| finding.text.contains("implements nothing")),
        "A12 must be evaluated on the new contract"
    );
}

#[test]
fn stale_recovery_root_does_not_block_a_fresh_freeze() {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    recover(&run);
    let (path, mut value) = receipt(&run);
    value[0]["task_root"] = "/missing-old-task-root".into();
    std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    let mut prepared = crate::command::workflow_task_set::prepare_from_judged(
        run.set.project.path(),
        &run.set.tasks,
        &run.set.prd,
        archon_core::config::GateMode::Enforce,
        &run.set.contract(),
        None,
    )
    .unwrap();
    assert!(
        prepared.record_recovery_refreeze().is_ok(),
        "stale authority is skipped, logged and retained for retry"
    );
}

#[tokio::test]
async fn real_resume_recovers_first_freeze_crash_before_approval_and_execution() {
    use archon_workflow::{WorkflowBundle, WorkflowBundleOrigin};
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    pre_implementation_head(&run);
    let universe = archon_workflow::task_universe::WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![run.set.tasks.display().to_string()],
        tasks: vec![archon_workflow::task_universe::WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-F-001".into(),
            implements: vec!["AC-F-001".into()],
            source_path: run.set.tasks.join("TASK-F-001.md").display().to_string(),
            ..Default::default()
        }],
    };
    let source = "export default async function workflow(w) { await w.tool('acceptance-contract-run', {tool:'acceptance-contract-run', round:1, maxRounds:3, checkIds:[]}); }";
    let plan = crate::command::workflow_live::WorkflowScriptPlan::generated(
        "resume",
        source,
        Vec::new(),
        Some(universe.clone()),
        Default::default(),
        &Default::default(),
    )
    .unwrap();
    let state = run.store.load_state(&run.run_id).unwrap();
    WorkflowBundle::create_for_run(
        &run.store,
        &state,
        source,
        WorkflowBundleOrigin::GeneratedHarness,
    )
    .unwrap();
    // First publication had no old pin, and was killed before its pin rename.
    let launch = run.set.pin().identity();
    let metadata = run
        .store
        .run_dir(&run.run_id)
        .join("v2/generated-metadata.json");
    std::fs::create_dir_all(metadata.parent().unwrap()).unwrap();
    std::fs::write(metadata, serde_json::json!({"schema_version":"test", "task_universe": universe, "script_lifecycle":false, "scaffold_hash":plan.scaffold_hash(), "observer_snapshot": {"schema_version":1,"canonical_task_root_identity":run.set.tasks.canonicalize().map(archon_shell::paths::plain).unwrap(),"expected_artifact_paths":[],"portable_acceptance_identity":launch,"lineage_recording":1}}).to_string()).unwrap();
    let pin = run.set.pin_path();
    std::fs::rename(
        &pin,
        pin.with_file_name(format!(
            ".{}.{TXN}.new",
            pin.file_name().unwrap().to_string_lossy()
        )),
    )
    .unwrap();
    let (sink, _rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let output = super::super::super::super::resume_generated_v2_workflow(
        run.set.project.path(),
        &run.store,
        &run.run_id,
        std::sync::Arc::new(accepting()),
        sink,
        vec![],
        crate::command::workflow_live::LiveApprovalMode::InteractiveSurface,
        true,
        &Default::default(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(output.contains("Workflow awaiting approval"), "{output}");
    assert!(
        !pin.exists(),
        "real resume must settle the missing-pin crash before approval"
    );
    assert!(!run.set.tasks.join(ACCEPTANCE_LOCK_FILE).exists());
    assert!(
        run.set
            .pin_path()
            .with_extension("publish-recovery.log")
            .exists()
    );
}

#[tokio::test]
async fn adopted_recovery_cleans_its_moved_aside_files() {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    pre_implementation_head(&run);
    recover(&run);
    stage(&run, &accepting()).await;
    for directory in [&run.set.tasks, run.set.pin_path().parent().unwrap()] {
        assert!(
            !std::fs::read_dir(directory).unwrap().any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".unverified-")),
            "adoption must consume inspection files"
        );
    }
}

#[tokio::test]
async fn moved_aside_pin_digest_is_bound_before_adoption() {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    pre_implementation_head(&run);
    recover(&run);
    let pin = run.set.pin_path();
    let aside = pin.with_file_name(format!(
        ".{}.unverified-{TXN}",
        pin.file_name().unwrap().to_string_lossy()
    ));
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&aside).unwrap()).unwrap();
    value["freeze_event_id"] = "tampered-inspection-pin".into();
    std::fs::write(aside, serde_json::to_vec(&value).unwrap()).unwrap();
    let contract_before = run.set.contract_bytes();
    let (_result, record) = stage(&run, &accepting()).await;
    assert!(record.blocks_completion());
    assert!(
        record
            .operational_errors
            .iter()
            .any(|error| error.contains("recovery")),
        "the altered preimage must produce a named refusal: {:?}",
        record.operational_errors
    );
    assert!(
        !pin.exists(),
        "untrusted evidence must refuse before re-freeze"
    );
    assert_eq!(run.set.contract_bytes(), contract_before);
}
