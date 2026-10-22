//! The launcher's reading of a task root: every combination of frozen
//! acceptance, frozen skeleton and present bodies, and the rule that a lock
//! which exists but does not verify is an error rather than `false`.

use std::path::{Path, PathBuf};

use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE,
};

use super::test_support::contract_bytes;
use super::{
    FrozenChainSnapshot, FrozenStage, frozen_chain_snapshot, verify_frozen_stage_with_recovery,
};

#[test]
fn frozen_override_rechecks_stale_logic_even_when_old_findings_are_open() {
    let versions = super::super::workflow_host_command_logic::versions();
    let digests = super::super::workflow_host_command_logic::digests();
    let version = versions["freeze-acceptance"];
    let digest = digests["freeze-acceptance"].0.as_str();
    let catalog =
        super::super::workflow_host_command_catalog::fixed_decomposition_catalog("fixture")
            .unwrap();
    let decide = |data: serde_json::Value| {
        let record: archon_workflow::WorkflowV2CallRecord =
            serde_json::from_value(serde_json::json!({
                "call": {"id": "call-1", "method": "hostCommand",
                         "options": {"host_command": {"commandId": "freeze-acceptance"}}},
                "attempt": 1, "input_hash": "fixture", "status": "accepted",
                "result": {"status": "accepted", "summary": "fixture", "data": data}
            }))
            .unwrap();
        super::freeze_outcome_decision(&record, &catalog).unwrap()
    };
    // The executor's own test: same version replays, whatever the digest.
    let repinned = decide(serde_json::json!({
        "gateEnvelope": {"policy_findings": []},
        "logicVersion": version,
        "logicDigest": digest,
    }));
    assert!(!repinned.verify, "{}", repinned.reason);
    assert!(repinned.reason.contains("replays under this build's logic"));
    let stale = decide(serde_json::json!({
        "gateEnvelope": {"policy_findings": []},
        "logicVersion": version.saturating_sub(1),
        "logicDigest": "old-digest",
    }));
    assert!(stale.verify, "{}", stale.reason);
    let open = decide(serde_json::json!({
        "gateEnvelope": {"policy_findings": [{"text": "repair remains open"}]},
        "logicVersion": version.saturating_sub(1),
        "logicDigest": "old-digest",
    }));
    assert!(open.verify);
    assert!(open.reason.contains("gate logic changed"));
    let current_open = decide(serde_json::json!({
        "gateEnvelope": {"policy_findings": [{"text": "repair remains open"}]},
        "logicVersion": version,
        "logicDigest": digest,
    }));
    assert!(!current_open.verify);
    assert!(current_open.reason.contains("1 open finding(s)"));
    assert!(!decide(serde_json::json!({"logicVersion": version.saturating_sub(1)})).verify);
    assert!(!super::freeze_record_status_allows_override(
        archon_workflow::WorkflowV2Status::NeedsReview,
        false,
    ));
    assert!(!super::freeze_record_status_allows_override(
        archon_workflow::WorkflowV2Status::Accepted,
        true,
    ));
}

const PRD: &str = "# PRD X\n\n## Requirements\n\n| ID | Requirement |\n|---|---|\n| REQ-X-001 | Prove the fixture. |\n\n## Acceptance Criteria\n\n| ID | Criterion |\n|---|---|\n| AC-X-001 | The fixture is proven. |\n";

struct Fixture {
    _temp: tempfile::TempDir,
    project: PathBuf,
    prd: PathBuf,
    tasks: PathBuf,
    acceptance_digest: String,
}

/// A project with a PRD and an empty task root; nothing frozen yet. The
/// project is also a (commitless) git checkout so it can serve as the
/// repository a launch is grounded in.
fn project() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let project = temp
        .path()
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap();
    let prd = project.join("prds/PRD-X.md");
    let tasks = project.join("tasks/PRD-X");
    std::fs::create_dir_all(prd.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(&prd, PRD).unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&project)
            .status()
            .unwrap()
            .success()
    );
    Fixture {
        _temp: temp,
        project,
        prd,
        tasks,
        acceptance_digest: String::new(),
    }
}

fn freeze_acceptance(fixture: &mut Fixture) {
    fixture.acceptance_digest =
        super::test_support::freeze_acceptance(&fixture.project, &fixture.prd, &fixture.tasks);
}

/// A two-task skeleton on top of the frozen acceptance contract.
fn freeze_skeleton(fixture: &Fixture) {
    super::test_support::freeze_skeleton(
        &fixture.project,
        &fixture.tasks,
        &fixture.acceptance_digest,
        &["TASK-X-010", "TASK-X-020"],
    );
}

/// A launch config grounding the authors in `repository`.
fn launch_config(repository: &Path) -> archon_core::config::ArchonConfig {
    let mut config = archon_core::config::ArchonConfig::default();
    config.workflow.repository_root = Some(repository.to_path_buf());
    config
}

fn snapshot(fixture: &Fixture) -> anyhow::Result<FrozenChainSnapshot> {
    frozen_chain_snapshot(&fixture.project, &fixture.prd, &fixture.tasks)
}

fn subjects(ids: &[&str]) -> Vec<archon_workflow::HostCommandSubject> {
    ids.iter()
        .map(|id| archon_workflow::HostCommandSubject {
            task_id: (*id).into(),
            file_name: format!("{id}.md"),
        })
        .collect()
}

#[test]
fn an_absent_or_empty_task_root_freezes_nothing() {
    let fixture = project();
    assert_eq!(snapshot(&fixture).unwrap(), FrozenChainSnapshot::default());
    let missing = frozen_chain_snapshot(
        &fixture.project,
        &fixture.prd,
        &fixture.project.join("tasks/absent"),
    )
    .unwrap();
    assert_eq!(missing, FrozenChainSnapshot::default());
    assert_eq!(
        FrozenChainSnapshot::default().to_argument(),
        serde_json::json!({"acceptance": false, "skeleton": false, "subjects": [], "bodies": []})
    );
}

#[test]
fn a_contract_without_its_lock_is_not_frozen() {
    let fixture = project();
    std::fs::write(
        fixture.tasks.join(ACCEPTANCE_CONTRACT_FILE),
        contract_bytes("whatever"),
    )
    .unwrap();
    assert_eq!(snapshot(&fixture).unwrap(), FrozenChainSnapshot::default());
}

#[test]
fn a_frozen_acceptance_contract_alone_is_reported_without_subjects() {
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    let found = snapshot(&fixture).unwrap();
    assert_eq!(
        found,
        FrozenChainSnapshot {
            acceptance: true,
            skeleton: false,
            subjects: Vec::new(),
            bodies: Vec::new(),
            body_digests: Vec::new(),
            preserved_digests: Vec::new(),
        }
    );
    verify_frozen_stage_with_recovery(
        &fixture.project,
        &fixture.prd,
        &fixture.tasks,
        FrozenStage::Acceptance,
        true,
    )
    .unwrap();
    let error = verify_frozen_stage_with_recovery(
        &fixture.project,
        &fixture.prd,
        &fixture.tasks,
        FrozenStage::Skeleton,
        true,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("no frozen task skeleton"),
        "{error:#}"
    );
}

#[test]
fn frozen_acceptance_reverify_reads_under_the_existing_shared_chain_lock() {
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    let pin_path =
        crate::command::workflow_task_set::acceptance_pin_path(&fixture.project, &fixture.tasks);
    let publish_lock_path = archon_workflow::task_set_publish_lock::lock_path(&pin_path);
    std::fs::write(&publish_lock_path, b"").unwrap();
    let shared =
        archon_workflow::task_set_publish_lock::PublishLockFile::hold_shared_existing(&pin_path)
            .unwrap()
            .expect("the existing publish lock is held shared");
    let verified = super::verify_frozen_stage_with_recovery(
        &fixture.project,
        &fixture.prd,
        &fixture.tasks,
        FrozenStage::Acceptance,
        false,
    )
    .expect("read-only frozen acceptance reverify must use its held shared lock");
    assert!(verified.acceptance);
    drop(shared);
}

#[test]
fn a_frozen_skeleton_reports_its_subjects_and_the_bodies_present() {
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    freeze_skeleton(&fixture);
    let none = snapshot(&fixture).unwrap();
    let skeleton_digests = [TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE]
        .map(|name| {
            (
                name.to_string(),
                archon_workflow::task_set_contract::content_digest(
                    &std::fs::read(fixture.tasks.join(name)).unwrap(),
                ),
            )
        })
        .to_vec();
    assert!(none.acceptance && none.skeleton);
    assert_eq!(none.subjects, subjects(&["TASK-X-010", "TASK-X-020"]));
    assert!(none.bodies.is_empty() && none.body_digests.is_empty());
    assert_eq!(none.preserved_digests, skeleton_digests);
    std::fs::write(fixture.tasks.join("TASK-X-020.md"), "# body").unwrap();
    // A directory or a stray file is not a body.
    std::fs::create_dir(fixture.tasks.join("TASK-X-010.md")).unwrap();
    std::fs::write(fixture.tasks.join("TASK-X-999.md"), "# stray").unwrap();
    let one = snapshot(&fixture).unwrap();
    assert_eq!(one.bodies, vec!["TASK-X-020.md".to_string()]);
    assert_eq!(
        one.body_digests,
        vec![(
            "TASK-X-020.md".to_string(),
            archon_workflow::task_set_contract::content_digest(b"# body"),
        )]
    );
    assert_eq!(
        one.preserved_digests,
        [
            skeleton_digests[0].clone(),
            skeleton_digests[1].clone(),
            (
                "TASK-X-020.md".to_string(),
                archon_workflow::task_set_contract::content_digest(b"# body"),
            ),
        ]
    );
    assert_eq!(
        one.to_argument(),
        serde_json::json!({
            "acceptance": true,
            "skeleton": true,
            "subjects": [
                {"taskId": "TASK-X-010", "fileName": "TASK-X-010.md"},
                {"taskId": "TASK-X-020", "fileName": "TASK-X-020.md"},
            ],
            "bodies": ["TASK-X-020.md"],
        })
    );
    std::fs::remove_dir(fixture.tasks.join("TASK-X-010.md")).unwrap();
    std::fs::write(fixture.tasks.join("TASK-X-010.md"), "# body").unwrap();
    assert_eq!(
        snapshot(&fixture).unwrap().bodies,
        vec!["TASK-X-010.md".to_string(), "TASK-X-020.md".to_string()]
    );
    let verified = verify_frozen_stage_with_recovery(
        &fixture.project,
        &fixture.prd,
        &fixture.tasks,
        FrozenStage::Skeleton,
        true,
    )
    .unwrap();
    assert_eq!(verified.subjects, subjects(&["TASK-X-010", "TASK-X-020"]));
    let before = snapshot(&fixture).unwrap().body_digests;
    let preserved_before = snapshot(&fixture).unwrap().preserved_digests;
    std::fs::write(fixture.tasks.join("TASK-X-020.md"), "# changed body").unwrap();
    assert_ne!(snapshot(&fixture).unwrap().body_digests, before);
    assert_ne!(
        snapshot(&fixture).unwrap().preserved_digests,
        preserved_before
    );
}

#[test]
fn a_newer_acceptance_freeze_leaves_an_intact_skeleton_pending_for_resume() {
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    freeze_skeleton(&fixture);

    // The acceptance phase publishes its new pin before the skeleton phase
    // rebinds the skeleton to that acceptance digest.
    let mut prd = std::fs::read(&fixture.prd).unwrap();
    prd.extend_from_slice(b"\n<!-- newer acceptance freeze -->\n");
    std::fs::write(&fixture.prd, prd).unwrap();
    freeze_acceptance(&mut fixture);
    let pending = snapshot(&fixture).expect("the intact stale skeleton is pending");
    assert!(pending.acceptance);
    assert!(!pending.skeleton);
    assert!(pending.subjects.is_empty());

    // Replaying the normal skeleton freeze writes fresh gate provenance and
    // restores the ordinary complete-chain snapshot.
    freeze_skeleton(&fixture);
    let complete = snapshot(&fixture).expect("the normally frozen chain verifies");
    assert!(complete.acceptance && complete.skeleton);
    assert_eq!(complete.subjects, subjects(&["TASK-X-010", "TASK-X-020"]));
}

#[test]
fn a_tampered_pending_skeleton_is_still_refused() {
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    freeze_skeleton(&fixture);
    let mut prd = std::fs::read(&fixture.prd).unwrap();
    prd.extend_from_slice(b"\n<!-- newer acceptance freeze -->\n");
    std::fs::write(&fixture.prd, prd).unwrap();
    freeze_acceptance(&mut fixture);

    let skeleton_path = fixture.tasks.join(TASK_SKELETON_FILE);
    let mut bytes = std::fs::read(&skeleton_path).unwrap();
    bytes.extend_from_slice(b" ");
    std::fs::write(&skeleton_path, bytes).unwrap();

    let error = snapshot(&fixture).expect_err("a skeleton that misses its lock is tampered");
    assert!(error.to_string().contains("does not verify"), "{error:#}");
}

#[test]
fn a_lock_that_does_not_verify_is_an_error_never_a_silent_false() {
    // Acceptance contract edited after its lock.
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    std::fs::write(
        fixture.tasks.join(ACCEPTANCE_CONTRACT_FILE),
        contract_bytes("edited"),
    )
    .unwrap();
    let error = snapshot(&fixture).unwrap_err();
    assert!(error.to_string().contains("does not verify"), "{error:#}");

    // Lock without a pin.
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    std::fs::remove_file(crate::command::workflow_task_set::acceptance_pin_path(
        &fixture.project,
        &fixture.tasks,
    ))
    .unwrap();
    let error = snapshot(&fixture).unwrap_err();
    assert!(error.to_string().contains("host pin"), "{error:#}");

    // Lock without its contract.
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    std::fs::remove_file(fixture.tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap();
    assert!(snapshot(&fixture).is_err());

    // Skeleton lock without an acceptance lock.
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    freeze_skeleton(&fixture);
    std::fs::remove_file(fixture.tasks.join(ACCEPTANCE_LOCK_FILE)).unwrap();
    let error = snapshot(&fixture).unwrap_err();
    assert!(error.to_string().contains("inconsistent"), "{error:#}");

    // Skeleton edited after its lock.
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    freeze_skeleton(&fixture);
    std::fs::write(fixture.tasks.join(TASK_SKELETON_FILE), b"{}").unwrap();
    let error = snapshot(&fixture).unwrap_err();
    assert!(
        error.to_string().contains("task skeleton")
            && error.to_string().contains("does not verify"),
        "{error:#}"
    );

    // The PRD changed since the contract was frozen.
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    std::fs::write(&fixture.prd, format!("{PRD}\nAn added line.\n")).unwrap();
    let error = snapshot(&fixture).unwrap_err();
    assert!(error.to_string().contains("different PRD"), "{error:#}");
}

/// The staged child reports a failed verification through the envelope, so
/// the script stops with the reason rather than a missing receipt.
#[test]
fn the_staged_verify_child_reports_through_the_envelope_and_publishes_nothing() {
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    let staging = fixture.project.join("staging");
    std::fs::create_dir_all(&staging).unwrap();
    let envelope = staging.join("gate-envelope.json");
    let mut config = archon_core::config::ArchonConfig::default();
    config.workflow.gate_mode = archon_core::config::GateMode::Enforce;
    for (stage, expect_error) in [("acceptance", false), ("skeleton", true)] {
        super::handle_staged_verify(
            &fixture.project,
            stage,
            Path::new("tasks/PRD-X"),
            Path::new("prds/PRD-X.md"),
            Some(&envelope),
            Some("call-1"),
            false,
            &config,
        )
        .unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&envelope).unwrap()).unwrap();
        assert_eq!(value["policy_findings"], serde_json::json!([]));
        assert_eq!(
            value["operational_error"].is_null(),
            !expect_error,
            "{stage}: {value}"
        );
        assert_eq!(
            std::fs::read_dir(&staging).unwrap().count(),
            1,
            "{stage}: the envelope is the only staged output"
        );
    }
    let error = super::handle_staged_verify(
        &fixture.project,
        "bodies",
        Path::new("tasks/PRD-X"),
        Path::new("prds/PRD-X.md"),
        Some(&envelope),
        Some("call-1"),
        false,
        &config,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("acceptance or skeleton"),
        "{error:#}"
    );
}

#[path = "workflow_decompose_frozen_chain_launch_args_tests.rs"]
mod launch_args_tests;
