//! The launcher's reading of a task root: every combination of frozen
//! acceptance, frozen skeleton and present bodies, and the rule that a lock
//! which exists but does not verify is an error rather than `false`.

use std::path::{Path, PathBuf};

use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, TASK_SKELETON_FILE,
};

use super::test_support::contract_bytes;
use super::{FrozenChainSnapshot, FrozenStage, frozen_chain_snapshot, verify_frozen_stage};

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
    let project = temp.path().canonicalize().unwrap();
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
        }
    );
    verify_frozen_stage(
        &fixture.project,
        &fixture.prd,
        &fixture.tasks,
        FrozenStage::Acceptance,
    )
    .unwrap();
    let error = verify_frozen_stage(
        &fixture.project,
        &fixture.prd,
        &fixture.tasks,
        FrozenStage::Skeleton,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("no frozen task skeleton"),
        "{error:#}"
    );
}

#[test]
fn a_frozen_skeleton_reports_its_subjects_and_the_bodies_present() {
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    freeze_skeleton(&fixture);
    let none = snapshot(&fixture).unwrap();
    assert_eq!(
        none,
        FrozenChainSnapshot {
            acceptance: true,
            skeleton: true,
            subjects: subjects(&["TASK-X-010", "TASK-X-020"]),
            bodies: Vec::new(),
        }
    );
    std::fs::write(fixture.tasks.join("TASK-X-020.md"), "# body").unwrap();
    // A directory or a stray file is not a body.
    std::fs::create_dir(fixture.tasks.join("TASK-X-010.md")).unwrap();
    std::fs::write(fixture.tasks.join("TASK-X-999.md"), "# stray").unwrap();
    let one = snapshot(&fixture).unwrap();
    assert_eq!(one.bodies, vec!["TASK-X-020.md".to_string()]);
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
    let verified = verify_frozen_stage(
        &fixture.project,
        &fixture.prd,
        &fixture.tasks,
        FrozenStage::Skeleton,
    )
    .unwrap();
    assert_eq!(verified.subjects, subjects(&["TASK-X-010", "TASK-X-020"]));
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
        &config,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("acceptance or skeleton"),
        "{error:#}"
    );
}

/// The launch reads the frozen chain, rehearses the script against it (the
/// dry run answers `verify-frozen-skeleton` with a stand-in subject) and
/// binds the reading into the persisted arguments.
struct ArgumentsBarrier {
    project: PathBuf,
    seen: std::sync::Mutex<Option<serde_json::Value>>,
}

#[async_trait::async_trait(?Send)]
impl archon_workflow::WorkflowLlmClientFactory for ArgumentsBarrier {
    async fn build_client(
        &self,
        _request: archon_workflow::WorkflowLlmClientRequest,
    ) -> archon_workflow::WorkflowResult<std::sync::Arc<dyn archon_workflow::WorkflowLlmClient>>
    {
        let store = archon_workflow::WorkflowStore::project(&self.project);
        let run = store.list_runs()?.pop().expect("one run");
        let path = store
            .run_dir(&run.id)
            .join(crate::command::workflow_decompose::FIXED_ARGUMENTS_PATH);
        let arguments: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        *self.seen.lock().unwrap() = Some(arguments);
        Err(archon_workflow::WorkflowError::port("barrier observed"))
    }
}

#[tokio::test]
async fn a_launch_on_a_frozen_task_root_binds_the_frozen_chain_into_its_arguments() {
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    freeze_skeleton(&fixture);
    std::fs::write(fixture.tasks.join("TASK-X-010.md"), "# body").unwrap();
    let factory = ArgumentsBarrier {
        project: fixture.project.clone(),
        seen: std::sync::Mutex::new(None),
    };
    let error = crate::command::workflow_decompose::run_fixed_decomposition_with_factory(
        &fixture.project,
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        None,
        true,
        &launch_config(&fixture.project),
        &archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new()),
        &factory,
    )
    .await
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("barrier observed"),
        "{error:#}"
    );
    let seen = factory
        .seen
        .lock()
        .unwrap()
        .clone()
        .expect("arguments persisted");
    assert_eq!(
        seen["frozenChain"],
        serde_json::json!({
            "acceptance": true,
            "skeleton": true,
            "subjects": [
                {"taskId": "TASK-X-010", "fileName": "TASK-X-010.md"},
                {"taskId": "TASK-X-020", "fileName": "TASK-X-020.md"},
            ],
            "bodies": ["TASK-X-010.md"],
        })
    );
}

#[tokio::test]
async fn a_launch_on_a_task_root_whose_lock_does_not_verify_creates_no_run() {
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    std::fs::write(
        fixture.tasks.join(ACCEPTANCE_CONTRACT_FILE),
        contract_bytes("edited"),
    )
    .unwrap();
    let factory = ArgumentsBarrier {
        project: fixture.project.clone(),
        seen: std::sync::Mutex::new(None),
    };
    let error = crate::command::workflow_decompose::run_fixed_decomposition_with_factory(
        &fixture.project,
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        None,
        true,
        &launch_config(&fixture.project),
        &archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new()),
        &factory,
    )
    .await
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("does not verify"),
        "{error:#}"
    );
    assert!(factory.seen.lock().unwrap().is_none());
    assert!(!fixture.project.join(".archon/workflows").exists());
}
