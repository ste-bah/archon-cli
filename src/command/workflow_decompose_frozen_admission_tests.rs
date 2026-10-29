use super::*;
use crate::command::workflow_host_command_exec::{
    FixedHostCommandExecutor, WorkflowHostCommandExecutor,
};
use archon_workflow::{HostCommandRequest, WorkflowStore};

struct LocalProcess(PathBuf);

#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::HostCommandProcessAdapter for LocalProcess {
    async fn execute(
        &self,
        mut request: crate::command::workflow_host_command_catalog::ResolvedHostCommand,
        control: crate::command::workflow_host_command_supervisor::HostCommandControl,
    ) -> archon_workflow::WorkflowResult<
        crate::command::workflow_host_command_supervisor::SupervisedProcessOutput,
    > {
        // The real child gets writable test-local bootstrap directories. All
        // command execution, supervision, auditing and publication remain real.
        for name in [
            "HOME",
            "XDG_DATA_HOME",
            "ARCHON_CONFIG_DIR",
            "ARCHON_LOG_DIR",
        ] {
            request
                .environment
                .insert(name.into(), self.0.clone().into_os_string());
        }
        crate::command::workflow_host_command_supervisor::supervise_process_group(
            request, control, None,
        )
        .await
    }
}

fn alter(
    f: &Fixture,
    file: &str,
    lock: &str,
    field: &str,
    change: impl FnOnce(&mut serde_json::Value),
) {
    let path = f.tasks.join(file);
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    change(&mut value);
    let bytes = serde_json::to_vec_pretty(&value).unwrap();
    let digest = archon_workflow::task_set_contract::content_digest(&bytes);
    std::fs::write(path, bytes).unwrap();
    let pin = crate::command::workflow_task_set::acceptance_pin_path(&f.project, &f.tasks);
    for (path, key) in [(f.tasks.join(lock), "digest"), (pin, field)] {
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value[key] = digest.clone().into();
        std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
}

fn run(store: &WorkflowStore) -> archon_workflow::run::WorkflowRun {
    let mut run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "fixture".into(),
            task: "fixture".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    run.status = archon_workflow::RunStatus::Running;
    store.save_state(&run).unwrap();
    run
}

fn prior_verdicts(store: &WorkflowStore, run: &archon_workflow::run::WorkflowRun) {
    for capability in ["freeze-acceptance", "freeze-skeleton"] {
        let record: archon_workflow::WorkflowV2CallRecord = serde_json::from_value(serde_json::json!({
            "call": {"id": capability, "method": "hostCommand", "options": {"host_command": {"commandId": capability}}},
            "attempt": 1, "input_hash": "fixture", "status": "accepted",
            "result": {"status": "accepted", "summary": "fixture", "data": {
                "gateEnvelope": {"policy_findings": []},
                "postcondition": {"satisfied": true},
                "publicationReceipt": {"schema_version": 1, "call_id": capability, "command_id": capability, "committed_at": "fixture", "entries": []},
                "logicVersion": crate::command::workflow_host_command_logic::versions()[capability] - 1,
                "logicDigest": "prior"
            }}
        })).unwrap();
        archon_workflow::WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"))
            .save_call_record(&record)
            .unwrap();
    }
}

#[tokio::test]
async fn frozen_chain_authored_admission_reaches_real_parent_postcondition() {
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        let mut f = project();
        freeze_acceptance(&mut f);
        freeze_skeleton(&f);
        alter(
            &f,
            TASK_SKELETON_FILE,
            TASK_SKELETON_LOCK_FILE,
            "skeleton_digest",
            |v| {
                v["tasks"][0]["file_name"] = serde_json::json!("../TASK-X-010.md");
            },
        );
        let store = WorkflowStore::project(&f.project);
        let run = run(&store);
        prior_verdicts(&store, &run);
        let catalog =
            crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("fixture")
                .unwrap();
        let launch = super::super::prepare_launch_snapshot(
            &f.project, &f.prd, &f.tasks, &store, &f.project, &catalog,
        )
        .expect("authenticated authored content must reach execution");
        assert!(launch.frozen_chain.skeleton);
        let resumed = super::super::resume_chain_argument(
            &store,
            &run.id,
            run.generation,
            &f.project,
            &f.prd,
            &f.tasks,
            false,
        )
        .unwrap();
        assert_eq!(resumed["skeleton"], true);
        let mut context = crate::command::workflow_host_command_exec_tests::context(f._temp.path());
        context.project_root = f.project.clone();
        context.task_root = f.tasks.clone();
        context.prd_path = f.prd.clone();
        context.prd_digest =
            archon_workflow::task_set_contract::content_digest(&std::fs::read(&f.prd).unwrap());
        context.program = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("archon");
        let executor = FixedHostCommandExecutor::with_process(
            catalog,
            context,
            store.run_dir(&run.id),
            std::sync::Arc::new(LocalProcess(f.project.clone())),
        );
        let outcome = executor
            .execute(
                HostCommandRequest::new("verify-frozen-skeleton", None).unwrap(),
                Some(run.generation),
            )
            .await
            .unwrap();
        assert!(outcome.gate_envelope.is_some(), "{outcome:?}");
        let envelope = outcome.gate_envelope.unwrap();
        assert!(
            !envelope.policy_findings.is_empty(),
            "real parent must return the authored finding"
        );
        assert!(envelope.operational_error.is_none());
    })
    .await
    .expect("test watchdog");
}

#[test]
fn frozen_chain_launch_defers_missing_cached_bytes_until_claimed_recovery() {
    let mut f = project();
    freeze_acceptance(&mut f);
    freeze_skeleton(&f);
    snapshot(&f).unwrap();
    std::fs::remove_file(f.tasks.join(TASK_SKELETON_FILE)).unwrap();
    let store = WorkflowStore::project(&f.project);
    let catalog =
        crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("fixture")
            .unwrap();
    let launch = super::super::prepare_launch_snapshot(
        &f.project, &f.prd, &f.tasks, &store, &f.project, &catalog,
    )
    .unwrap();
    assert!(launch.deferred);
    assert!(!f.tasks.join(TASK_SKELETON_FILE).exists());
    assert!(
        super::super::recover_deferred_launch_snapshot(
            &f.project, &f.prd, &f.tasks, &store, &f.project, &catalog
        )
        .unwrap()
        .0
        .skeleton
    );
    let run = run(&store);
    prior_verdicts(&store, &run);
    super::super::resume_chain_argument(
        &store,
        &run.id,
        run.generation,
        &f.project,
        &f.prd,
        &f.tasks,
        false,
    )
    .unwrap();
    let record = store.run_dir(&run.id).join(format!(
        "decomposition/resume-frozen-chain/generation-{}.json",
        run.generation
    ));
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&record).unwrap()).unwrap();
    legacy["snapshot"].as_object_mut().unwrap().remove("tasks");
    std::fs::write(record, serde_json::to_vec(&legacy).unwrap()).unwrap();
    std::fs::remove_file(f.tasks.join(TASK_SKELETON_FILE)).unwrap();
    let resumed = super::super::resume_chain_argument(
        &store,
        &run.id,
        run.generation,
        &f.project,
        &f.prd,
        &f.tasks,
        false,
    )
    .unwrap();
    assert!(f.tasks.join(TASK_SKELETON_FILE).is_file());
    assert!(
        resumed["tasks"].is_array(),
        "recorded legacy resumes recover before reading comparison entries: {resumed}"
    );
}

#[test]
fn frozen_chain_corrupt_journal_is_permanent_with_journal_remedy() {
    let mut f = project();
    freeze_acceptance(&mut f);
    let pin = crate::command::workflow_task_set::acceptance_pin_path(&f.project, &f.tasks);
    let [journal, _] = archon_workflow::task_set_publish_lock::journal_paths(&pin);
    std::fs::write(&journal, b"{").unwrap();
    let path = f.project.join("staging/envelope.json");
    super::super::stage_verify(
        &f.project,
        "acceptance",
        &f.tasks,
        &f.prd,
        Some(&path),
        Some("call-1"),
        false,
    )
    .unwrap();
    let found: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(found["operational_error"]["kind"], "permanent", "{found}");
    let text = found["operational_error"]["text"].as_str().unwrap();
    assert!(
        text.contains(journal.to_str().unwrap())
            && text.contains("inspect it and the transaction files it names, then remove it"),
        "{text}"
    );
    std::fs::write(
        &journal,
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 2, "transaction": "a".repeat(32), "state": "prepared",
            "remedy": "inspect the retained transaction", "entries": []
        }))
        .unwrap(),
    )
    .unwrap();
    let path = f.project.join("staging/incompatible-envelope.json");
    super::super::stage_verify(
        &f.project,
        "acceptance",
        &f.tasks,
        &f.prd,
        Some(&path),
        Some("call-2"),
        false,
    )
    .unwrap();
    let found: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(found["operational_error"]["kind"], "permanent", "{found}");
    let text = found["operational_error"]["text"].as_str().unwrap();
    assert!(
        text.contains(journal.to_str().unwrap())
            && text.contains("has schema 2")
            && text.contains("inspect it, then remove it"),
        "{text}"
    );
}

#[test]
fn frozen_chain_resume_keeps_generation_entries_after_repaired_publication() {
    let mut f = project();
    freeze_acceptance(&mut f);
    freeze_skeleton(&f);
    let store = WorkflowStore::project(&f.project);
    let run = run(&store);
    prior_verdicts(&store, &run);
    let first = super::super::resume_chain_argument(
        &store,
        &run.id,
        run.generation,
        &f.project,
        &f.prd,
        &f.tasks,
        false,
    )
    .unwrap();
    assert!(first["tasks"].is_array(), "{first}");
    alter(
        &f,
        TASK_SKELETON_FILE,
        TASK_SKELETON_LOCK_FILE,
        "skeleton_digest",
        |v| {
            v["tasks"][0]["implements"] = serde_json::json!(["AC-X-001", "REQ-X-001"]);
        },
    );
    let resumed = super::super::resume_chain_argument(
        &store,
        &run.id,
        run.generation,
        &f.project,
        &f.prd,
        &f.tasks,
        false,
    )
    .unwrap();
    assert_eq!(
        resumed["tasks"], first["tasks"],
        "replay must compare against this generation's original entries"
    );
}

#[test]
fn frozen_chain_duplicate_authored_ids_keep_consistent_repair_comparison_inputs() {
    let mut f = project();
    freeze_acceptance(&mut f);
    freeze_skeleton(&f);
    alter(
        &f,
        TASK_SKELETON_FILE,
        TASK_SKELETON_LOCK_FILE,
        "skeleton_digest",
        |v| {
            v["tasks"][1]["task_id"] = v["tasks"][0]["task_id"].clone();
        },
    );
    let store = WorkflowStore::project(&f.project);
    let catalog =
        crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("fixture")
            .unwrap();
    let launch = super::super::prepare_launch_snapshot(
        &f.project, &f.prd, &f.tasks, &store, &f.project, &catalog,
    )
    .unwrap();
    let snapshot = launch.frozen_chain;
    let unique: std::collections::BTreeSet<_> = snapshot.tasks.iter().map(|t| &t.task_id).collect();
    assert_eq!(
        snapshot.tasks.len(),
        unique.len(),
        "duplicate authored ids must not fail the script's one-entry-per-subject admission rule"
    );
    assert_eq!(snapshot.subjects.len(), unique.len());
    assert_eq!(snapshot.tasks[0].task_id, "TASK-X-010");
    let path = f.project.join("staging/duplicate-envelope.json");
    super::super::stage_verify(
        &f.project,
        "skeleton",
        &f.tasks,
        &f.prd,
        Some(&path),
        Some("call-1"),
        false,
    )
    .unwrap();
    let found: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert!(!found["policy_findings"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn frozen_chain_latest_publication_anchors_retained_subjects_before_rebind() {
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        let mut f = project();
        freeze_acceptance(&mut f);
        freeze_skeleton(&f);
        snapshot(&f).unwrap();
        let original = std::fs::read(f.tasks.join(TASK_SKELETON_FILE)).unwrap();
        let old_lock = std::fs::read(f.tasks.join(TASK_SKELETON_LOCK_FILE)).unwrap();
        let mut next: serde_json::Value = serde_json::from_slice(&original).unwrap();
        let mut added = next["tasks"][0].clone();
        added["task_id"] = "TASK-X-030".into();
        added["file_name"] = "TASK-X-030.md".into();
        next["tasks"].as_array_mut().unwrap().push(added);
        let store = WorkflowStore::project(&f.project);
        let run = run(&store);
        let mut context = crate::command::workflow_host_command_exec_tests::context(f._temp.path());
        context.project_root = f.project.clone();
        context.task_root = f.tasks.clone();
        context.prd_path = f.prd.clone();
        context.prd_digest =
            archon_workflow::task_set_contract::content_digest(&std::fs::read(&f.prd).unwrap());
        context.program = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("archon");
        let executor = FixedHostCommandExecutor::with_process(
            crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("fixture")
                .unwrap(),
            context,
            store.run_dir(&run.id),
            std::sync::Arc::new(LocalProcess(f.project.clone())),
        );
        let outcome = executor
            .execute(
                HostCommandRequest::new("freeze-skeleton", Some(next.to_string())).unwrap(),
                Some(run.generation),
            )
            .await
            .unwrap();
        assert_eq!(outcome.subjects.len(), 3, "{outcome:?}");
        let pin_path = crate::command::workflow_task_set::acceptance_pin_path(&f.project, &f.tasks);
        let mut pin: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&pin_path).unwrap()).unwrap();
        pin["skeleton_digest"] = serde_json::Value::Null;
        pin["skeleton_gate"] = serde_json::Value::Null;
        std::fs::write(pin_path, serde_json::to_vec(&pin).unwrap()).unwrap();
        std::fs::write(f.tasks.join(TASK_SKELETON_LOCK_FILE), old_lock).unwrap();
        let error = crate::command::workflow_task_set::prepare_skeleton_freeze_from_candidate(
            &f.project,
            &f.tasks,
            &f.prd,
            archon_core::config::GateMode::Observe,
            original,
        )
        .expect_err(
            "an older lock/cache pair cannot erase the latest accepted subject after rebind",
        );
        assert!(format!("{error:#}").contains("pin"), "{error:#}");
    })
    .await
    .expect("test watchdog");
}

#[path = "workflow_decompose_frozen_unsettled_tests.rs"]
mod unsettled_tests;
