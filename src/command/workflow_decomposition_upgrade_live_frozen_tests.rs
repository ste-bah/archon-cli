use super::*;

fn record_freeze(
    store: &WorkflowStore,
    run_id: &str,
    project: &Path,
    task_root: &Path,
    capability: &str,
    id: &str,
    started_at: &str,
    status: &str,
    version: u32,
    findings: serde_json::Value,
    published: bool,
) {
    let command_id = id.to_string();
    let receipt = if published {
        let file = if capability == "freeze-acceptance" {
            archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE
        } else {
            archon_workflow::task_set_contract::TASK_SKELETON_FILE
        };
        let path = task_root.join(file);
        let bytes = std::fs::read(&path).unwrap();
        serde_json::json!({
            "schema_version": 1,
            "call_id": command_id,
            "command_id": capability,
            "committed_at": started_at,
            "entries": [{
                "relative_path": file,
                "destination_path": path,
                "byte_len": bytes.len(),
                "blake3": archon_workflow::task_set_contract::content_digest(&bytes)
            }]
        })
    } else {
        serde_json::Value::Null
    };
    let current_version = crate::command::workflow_host_command_logic::versions()[capability];
    let (digest, _) = crate::command::workflow_host_command_logic::digests()
        .remove(capability)
        .unwrap();
    let data = serde_json::json!({
        "exitCode": if published { serde_json::json!(0) } else { serde_json::Value::Null },
        "stdout": "",
        "stderr": "",
        "stdoutBytes": 0,
        "stderrBytes": 0,
        "timedOut": false,
        "interrupted": findings.is_null() && !published,
        "stdoutTruncated": false,
        "stderrTruncated": false,
        "gateEnvelope": if findings.is_null() { serde_json::Value::Null } else { serde_json::json!({"schema_version": 1, "report": "fixture", "policy_findings": findings}) },
        "publicationReceipt": if published { receipt } else { serde_json::Value::Null },
        "postcondition": if findings.is_null() && !published { serde_json::Value::Null } else { serde_json::json!({"satisfied": published, "summary": "fixture"}) },
        "logicVersion": version,
        "logicDigest": if version == current_version { digest } else { "previous-logic-digest".to_string() },
        "logicBuild": crate::command::workflow_host_command_logic::THIS_BUILD
    });
    let record: archon_workflow::WorkflowV2CallRecord = serde_json::from_value(serde_json::json!({
        "run_id": run_id,
        "call": {
            "id": command_id,
            "method": "hostCommand",
            "options": {"host_command": {"commandId": capability, "stdin": "{\"entries\":[]}"}}
        },
        "attempt": 1,
        "started_at": started_at,
        "input_hash": "fixture",
        "status": status,
        "result": {"status": status, "summary": "fixture", "data": data}
    }))
    .unwrap();
    let _ = project;
    archon_workflow::WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"))
        .save_call_record(&record)
        .unwrap();
}

async fn paused_frozen_run() -> (
    tempfile::TempDir,
    WorkflowStore,
    String,
    FixedDecompositionStateV1,
) {
    let project = fixture_project();
    let (store, run_id, _) =
        super::super::workflow_decomposition_drift_tests::launch_and_pause(project.path()).await;
    let state: FixedDecompositionStateV1 =
        read_json(&store.run_dir(&run_id).join(FIXED_DECOMPOSITION_STATE_PATH));
    let task_root = Path::new(&state.identity.task_root_identity);
    crate::command::workflow_decompose_frozen_chain::test_support::freeze_chain(
        project.path(),
        Path::new(&state.identity.prd_identity),
        task_root,
        &["TASK-X-010"],
    );
    (project, store, run_id, state)
}

#[tokio::test]
async fn interrupted_freeze_attempt_keeps_last_verdict_chain_on_production_resume() {
    let (project, store, run_id, state) = paused_frozen_run().await;
    let task_root = Path::new(&state.identity.task_root_identity);
    let current = crate::command::workflow_host_command_logic::versions();
    for (capability, id) in [
        ("freeze-acceptance", "freeze-acceptance-1"),
        ("freeze-skeleton", "freeze-skeleton-1"),
    ] {
        record_freeze(
            &store,
            &run_id,
            project.path(),
            task_root,
            capability,
            id,
            "2026-10-10T10:00:00Z",
            "accepted",
            current[capability] - 1,
            serde_json::json!([]),
            true,
        );
    }
    record_freeze(
        &store,
        &run_id,
        project.path(),
        task_root,
        "freeze-acceptance",
        "freeze-acceptance-2",
        "2026-10-10T10:01:00Z",
        "needs_review",
        current["freeze-acceptance"],
        serde_json::Value::Null,
        false,
    );
    let ordinal_record = archon_workflow::WorkflowV2CallRecord::new(
        &run_id,
        archon_workflow::WorkflowV2HostCall {
            id: "acceptance-author-AC-X-001-7".into(),
            method: archon_workflow::WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        },
        1,
        "ordinal-floor-fixture".into(),
        archon_workflow::WorkflowV2Result::accepted("recorded author ordinal"),
        Vec::new(),
    );
    archon_workflow::WorkflowV2ResultStore::new(store.run_dir(&run_id).join("v2"))
        .save_call_record(&ordinal_record)
        .unwrap();

    let dry_run = crate::command::workflow_decompose::dry_run_fixed_decomposition(
        project.path(),
        &run_id,
        &launch_config(project.path()),
    )
    .await
    .unwrap();
    assert!(dry_run.contains("admission=admitted"), "{dry_run}");
    let generation = store.load_state(&run_id).unwrap().generation;
    let effective = crate::command::workflow_decompose_frozen_chain::resume_chain_argument(
        &store,
        &run_id,
        generation,
        project.path(),
        Path::new(&state.identity.prd_identity),
        task_root,
        false,
    )
    .unwrap();
    assert_eq!(effective["acceptance"], true, "{effective}");
    assert_eq!(effective["skeleton"], true, "{effective}");

    let before = [
        archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE,
        archon_workflow::task_set_contract::TASK_SKELETON_FILE,
        archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE,
        archon_workflow::task_set_contract::TASK_SKELETON_LOCK_FILE,
    ]
    .map(|name| std::fs::read(task_root.join(name)).unwrap());
    let resume = UpgradeBarrier {
        run_id: run_id.clone(),
        store: store.clone(),
        expected: state.identity.clone(),
        builds: AtomicUsize::new(0),
    };
    let error = resume_fixed_decomposition_at_binary_revision(
        project.path(),
        &run_id,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &resume,
        crate::command::workflow_decompose_progress::DecompositionCliUiSink::shared(),
        None,
        None,
        "logic-N-plus-one-resume-test",
    )
    .await
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("barrier observed"),
        "{error:#}"
    );
    assert_eq!(resume.builds.load(Ordering::SeqCst), 1);
    // What reached the script: the live chain and the ordinal floors.
    let script = crate::command::workflow_decompose_seed::resume::capture::take().unwrap();
    assert_eq!(script["frozenChain"]["acceptance"], true, "{script}");
    assert_eq!(script["frozenChain"]["skeleton"], true, "{script}");
    assert_eq!(
        script["phaseSeed"]["author_ordinals"]["acceptance"], 7,
        "{script}"
    );
    assert_eq!(
        before,
        [
            archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE,
            archon_workflow::task_set_contract::TASK_SKELETON_FILE,
            archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE,
            archon_workflow::task_set_contract::TASK_SKELETON_LOCK_FILE,
        ]
        .map(|name| std::fs::read(task_root.join(name)).unwrap())
    );
}

#[tokio::test]
async fn open_acceptance_findings_keep_the_chain_prefix_closed_on_resume() {
    let (project, store, run_id, state) = paused_frozen_run().await;
    let task_root = Path::new(&state.identity.task_root_identity);
    let current = crate::command::workflow_host_command_logic::versions();
    record_freeze(
        &store,
        &run_id,
        project.path(),
        task_root,
        "freeze-acceptance",
        "freeze-acceptance-1",
        "2026-10-10T10:00:00Z",
        "accepted",
        current["freeze-acceptance"] - 1,
        serde_json::json!([{"text":"repair remains open", "subject":"acceptance", "remediation_scope":"candidate_artifact"}]),
        true,
    );
    record_freeze(
        &store,
        &run_id,
        project.path(),
        task_root,
        "freeze-skeleton",
        "freeze-skeleton-1",
        "2026-10-10T10:00:01Z",
        "accepted",
        current["freeze-skeleton"] - 1,
        serde_json::json!([]),
        true,
    );
    let effective = crate::command::workflow_decompose_frozen_chain::resume_chain_argument(
        &store,
        &run_id,
        store.load_state(&run_id).unwrap().generation,
        project.path(),
        Path::new(&state.identity.prd_identity),
        task_root,
        true,
    )
    .unwrap();
    assert_eq!(effective["acceptance"], false, "{effective}");
    assert_eq!(effective["skeleton"], false, "{effective}");
    assert!(effective["subjects"].as_array().unwrap().is_empty());
    let resume = UpgradeBarrier {
        run_id: run_id.clone(),
        store: store.clone(),
        expected: state.identity.clone(),
        builds: AtomicUsize::new(0),
    };
    let error = resume_fixed_decomposition_with_factory(
        project.path(),
        &run_id,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &resume,
    )
    .await
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("barrier observed"),
        "{error:#}"
    );
    assert_eq!(resume.builds.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn later_publication_supersedes_the_last_clean_verdict() {
    let (project, store, run_id, state) = paused_frozen_run().await;
    let task_root = Path::new(&state.identity.task_root_identity);
    let current = crate::command::workflow_host_command_logic::versions();
    record_freeze(
        &store,
        &run_id,
        project.path(),
        task_root,
        "freeze-acceptance",
        "freeze-acceptance-1",
        "2026-10-10T10:00:00Z",
        "accepted",
        current["freeze-acceptance"] - 1,
        serde_json::json!([]),
        true,
    );
    record_freeze(
        &store,
        &run_id,
        project.path(),
        task_root,
        "freeze-acceptance",
        "freeze-acceptance-2",
        "2026-10-10T10:00:01Z",
        "needs_review",
        current["freeze-acceptance"],
        serde_json::Value::Null,
        true,
    );
    let effective = crate::command::workflow_decompose_frozen_chain::resume_chain_argument(
        &store,
        &run_id,
        store.load_state(&run_id).unwrap().generation,
        project.path(),
        Path::new(&state.identity.prd_identity),
        task_root,
        true,
    )
    .unwrap();
    assert_eq!(effective["acceptance"], false, "{effective}");
    assert_eq!(effective["skeleton"], false, "{effective}");
}

#[tokio::test]
async fn same_logic_resume_does_not_flip_frozen_phases() {
    let (project, store, run_id, state) = paused_frozen_run().await;
    let task_root = Path::new(&state.identity.task_root_identity);
    let current = crate::command::workflow_host_command_logic::versions();
    for (capability, id) in [
        ("freeze-acceptance", "freeze-acceptance-1"),
        ("freeze-skeleton", "freeze-skeleton-1"),
    ] {
        record_freeze(
            &store,
            &run_id,
            project.path(),
            task_root,
            capability,
            id,
            "2026-10-10T10:00:00Z",
            "accepted",
            current[capability],
            serde_json::json!([]),
            true,
        );
    }
    let effective = crate::command::workflow_decompose_frozen_chain::resume_chain_argument(
        &store,
        &run_id,
        store.load_state(&run_id).unwrap().generation,
        project.path(),
        Path::new(&state.identity.prd_identity),
        task_root,
        true,
    )
    .unwrap();
    assert_eq!(effective["acceptance"], false, "{effective}");
    assert_eq!(effective["skeleton"], false, "{effective}");
    let resume = UpgradeBarrier {
        run_id: run_id.clone(),
        store: store.clone(),
        expected: state.identity.clone(),
        builds: AtomicUsize::new(0),
    };
    let error = resume_fixed_decomposition_with_factory(
        project.path(),
        &run_id,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &resume,
    )
    .await
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("barrier observed"),
        "{error:#}"
    );
    assert_eq!(resume.builds.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn revision_only_resume_persists_live_frozen_chain_without_a_phase_seed() {
    let project = fixture_project();
    let (store, run_id, _) =
        super::super::workflow_decomposition_drift_tests::launch_and_pause(project.path()).await;
    let state: FixedDecompositionStateV1 =
        read_json(&store.run_dir(&run_id).join(FIXED_DECOMPOSITION_STATE_PATH));
    let task_root = Path::new(&state.identity.task_root_identity);
    crate::command::workflow_decompose_frozen_chain::test_support::freeze_chain(
        project.path(),
        Path::new(&state.identity.prd_identity),
        task_root,
        &["TASK-X-010"],
    );
    let contract = archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE;
    let skeleton = archon_workflow::task_set_contract::TASK_SKELETON_FILE;
    let acceptance_lock = archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE;
    let skeleton_lock = archon_workflow::task_set_contract::TASK_SKELETON_LOCK_FILE;
    let before = [contract, skeleton, acceptance_lock, skeleton_lock]
        .map(|name| std::fs::read(task_root.join(name)).unwrap());
    let resume = UpgradeBarrier {
        run_id: run_id.clone(),
        store: store.clone(),
        expected: state.identity.clone(),
        builds: AtomicUsize::new(0),
    };
    let error = resume_fixed_decomposition_at_binary_revision(
        project.path(),
        &run_id,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &resume,
        crate::command::workflow_decompose_progress::DecompositionCliUiSink::shared(),
        None,
        None,
        "logic-version-only-revision",
    )
    .await
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("barrier observed"),
        "{error:#}"
    );
    assert_eq!(resume.builds.load(Ordering::SeqCst), 1);

    let generation = store.load_state(&run_id).unwrap().generation;
    let record: serde_json::Value = read_json(&store.run_dir(&run_id).join(format!(
        "decomposition/resume-frozen-chain/generation-{generation}.json"
    )));
    assert_eq!(record["snapshot"]["acceptance"], true);
    assert_eq!(record["snapshot"]["skeleton"], true);
    assert!(
        !store
            .run_dir(&run_id)
            .join(crate::command::workflow_decompose_seed::SEEDS_DIR)
            .exists(),
        "revision-only drift must not need a phase seed"
    );
    assert_eq!(
        before,
        [contract, skeleton, acceptance_lock, skeleton_lock]
            .map(|name| std::fs::read(task_root.join(name)).unwrap())
    );
}

#[path = "workflow_decomposition_upgrade_live_frozen_receipt_tests.rs"]
mod receipt_chain;

#[path = "workflow_decomposition_upgrade_accepted_chain_tests.rs"]
mod accepted_chain;
