use super::*;

fn snapshot_tree(root: &Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    fn visit(
        root: &Path,
        current: &Path,
        out: &mut std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
    ) {
        for entry in std::fs::read_dir(current).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                visit(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    visit(root, root, &mut out);
    out
}

#[tokio::test]
async fn resume_dry_run_leaves_run_directory_byte_identical() {
    let project = fixture_project();
    let (store, run_id, _) =
        super::super::workflow_decomposition_drift_tests::launch_and_pause(project.path()).await;
    let state: FixedDecompositionStateV1 =
        read_json(&store.run_dir(&run_id).join(FIXED_DECOMPOSITION_STATE_PATH));

    let output = crate::command::workflow_decompose::dry_run_fixed_decomposition(
        project.path(),
        &run_id,
        &launch_config(project.path()),
    )
    .await
    .unwrap();
    assert!(
        output.contains("seed=none (no harness transition)"),
        "{output}"
    );

    let mut old = state.identity.clone();
    old.script_digest = "previous-script-digest".into();
    let transitions = RuntimeTransitions {
        schema_version: crate::command::workflow_decompose_transitions::TRANSITIONS_SCHEMA_VERSION,
        transitions: vec![
            crate::command::workflow_decompose_transitions::RuntimeTransition::new(
                old,
                state.identity.clone(),
            ),
        ],
    };
    store
        .write_run_json(&run_id, TRANSITIONS_PATH, &transitions)
        .unwrap();
    let reply_id = "acceptance-author-AC-X-001-1";
    let carried_entry = serde_json::json!({
        "id": "AC-X-001",
        "criterion": "the fixture is proven",
        "check": {"kind": "command", "command": "test -f output", "cwd": "project_root"},
        "gap_permitted": false,
        "covers": ["REQ-X-001"],
        "judgment": {"verdict": "accepted", "counterexample": "", "reason": "", "host_call_id": ""}
    });
    let reply_record: archon_workflow::WorkflowV2CallRecord = serde_json::from_value(serde_json::json!({
        "call": {"id": reply_id, "method": "agent", "options": {"result_mode": "rawOutcome"}},
        "attempt": 1,
        "started_at": "2026-10-06T01:00:00+00:00",
        "input_hash": "dry-run-fixture",
        "status": "accepted",
        "result": {"status": "accepted", "summary": "", "evidence": [], "artifacts": [], "commands_run": [], "files_read": [], "files_changed": [], "task_coverage": [], "residual_gaps": [], "data": {"content": carried_entry.to_string(), "stopReason": "end_turn"}}
    })).unwrap();
    archon_workflow::WorkflowV2ResultStore::new(store.run_dir(&run_id).join("v2"))
        .save_call_record(&reply_record)
        .unwrap();
    let recorded_seed = crate::command::workflow_decompose_seed::current_seed(
        &store,
        &run_id,
        Path::new(&state.log_path),
        &std::collections::BTreeMap::new(),
    )
    .unwrap()
    .expect("the changed harness records a seed");
    assert_eq!(recorded_seed.transition_index, 0);
    let run_dir = store.run_dir(&run_id);
    let groups = run_dir.join(crate::command::workflow_host_command_groups::GROUP_RECORDS_DIR);
    std::fs::create_dir_all(&groups).unwrap();
    // This mirrors the healed missing-job fixture: a known survivor identity
    // and a reaped group prove the record ended on both Unix and Windows.
    let mut ended = archon_shell::spawn::command(std::env::current_exe().unwrap())
        .arg("--help")
        .spawn()
        .unwrap();
    let ended_pid = ended.id();
    #[cfg(unix)]
    let ended_identity = (
        ended_pid,
        archon_shell::process_tree::identity_of(ended_pid)
            .unwrap()
            .unwrap(),
    );
    #[cfg(windows)]
    let ended_identity = (
        ended_pid,
        archon_shell::job_object::identity_of(ended_pid)
            .unwrap()
            .unwrap(),
    );
    ended.wait().unwrap();
    let missing_job = format!("Local\\archon-dry-run-healed-{}", uuid::Uuid::new_v4());
    std::fs::write(
        groups.join(format!("{ended_pid}.json")),
        serde_json::to_vec(
            &crate::command::workflow_host_command_groups::HostCommandGroupRecord {
                schema_version: 1,
                pgid: ended_pid,
                pid: ended_pid,
                session: None,
                job: Some(missing_job),
                survivors: vec![ended_identity],
                stalled: true,
                survivors_unknown: false,
                teardown_complete: false,
                command_id: "healed-fixture".into(),
                host_pid: ended_pid,
                host_start: None,
                leader_start: None,
                started_at: "2026-10-06T01:00:00+00:00".into(),
                file: None,
            },
        )
        .unwrap(),
    )
    .unwrap();
    let before = snapshot_tree(&run_dir);
    let output = crate::command::workflow_decompose::dry_run_fixed_decomposition(
        project.path(),
        &run_id,
        &launch_config(project.path()),
    )
    .await
    .unwrap();
    assert!(output.contains("admission=admitted"), "{output}");
    assert!(
        output.contains("arguments.json=equal metadata=enriched launch_digest_anchor=equal"),
        "{output}"
    );
    assert!(
        output.contains(&format!(
            "seed=existing transition {}",
            recorded_seed.transition_index
        )),
        "{output}"
    );
    assert!(output.contains("healed_groups_would_remove=1"), "{output}");
    assert!(output.contains("seed subject=acceptance"), "{output}");
    assert!(
        output.contains("call[0] id=acceptance-author-AC-X-001-"),
        "{output}"
    );
    assert!(
        output.contains("capability=freeze-acceptance"),
        "host command calls name their capability: {output}"
    );
    assert!(
        output.contains("criterion=\"The fixture is proven.\""),
        "{output}"
    );
    assert_eq!(
        before,
        snapshot_tree(&run_dir),
        "dry run changed a run file"
    );

    let state: FixedDecompositionStateV1 = read_json(&run_dir.join(FIXED_DECOMPOSITION_STATE_PATH));
    let mut previous = state.identity.clone();
    previous.script_digest = "previous-script-digest".into();
    let transitions = RuntimeTransitions {
        schema_version: crate::command::workflow_decompose_transitions::TRANSITIONS_SCHEMA_VERSION,
        transitions: vec![
            crate::command::workflow_decompose_transitions::RuntimeTransition::new(
                previous.clone(),
                state.identity.clone(),
            ),
            crate::command::workflow_decompose_transitions::RuntimeTransition::new(
                state.identity.clone(),
                previous,
            ),
        ],
    };
    store
        .write_run_json(&run_id, TRANSITIONS_PATH, &transitions)
        .unwrap();
    let would_record_index = transitions.transitions.len();
    let would_record_seed =
        crate::command::workflow_decompose_seed::current_seed_read_only_for_transition(
            &store,
            &run_id,
            would_record_index,
            &state.identity,
            &std::collections::BTreeMap::new(),
            &std::collections::BTreeMap::new(),
            None,
        )
        .unwrap()
        .expect("the changed runtime derives an in-memory seed");
    assert_eq!(would_record_seed.transition_index, would_record_index);
    let before = snapshot_tree(&run_dir);
    let output = crate::command::workflow_decompose::dry_run_fixed_decomposition(
        project.path(),
        &run_id,
        &launch_config(project.path()),
    )
    .await
    .unwrap();
    assert!(
        output.contains(&format!(
            "seed=would-record transition {} (derived in memory)",
            would_record_seed.transition_index
        )),
        "{output}"
    );
    assert_eq!(
        before,
        snapshot_tree(&run_dir),
        "dry run changed a run file"
    );
}

#[tokio::test]
async fn upgraded_resume_dry_run_does_not_skip_an_unrecorded_freeze_landing() {
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
    let contract_path =
        task_root.join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
    let skeleton_path = task_root.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE);
    let contract_before = std::fs::read(&contract_path).unwrap();
    let skeleton_before = std::fs::read(&skeleton_path).unwrap();

    let mut old = state.identity.clone();
    old.script_digest = "previous-script-digest".into();
    store
        .write_run_json(
            &run_id,
            TRANSITIONS_PATH,
            &RuntimeTransitions {
                schema_version:
                    crate::command::workflow_decompose_transitions::TRANSITIONS_SCHEMA_VERSION,
                transitions: vec![
                    crate::command::workflow_decompose_transitions::RuntimeTransition::new(
                        old,
                        state.identity.clone(),
                    ),
                ],
            },
        )
        .unwrap();

    // Match the pre-fix writer: the stored transition seed has no
    // resume_frozen_chain member, even though the live root is now frozen.
    let old_seed = crate::command::workflow_decompose_seed::current_seed(
        &store,
        &run_id,
        Path::new(&state.log_path),
        &std::collections::BTreeMap::new(),
    )
    .unwrap()
    .expect("legacy transition seed");
    assert_eq!(old_seed.transition_index, 0);
    let old_seed_path = std::fs::read_dir(
        store
            .run_dir(&run_id)
            .join(crate::command::workflow_decompose_seed::SEEDS_DIR),
    )
    .unwrap()
    .map(|entry| entry.unwrap().path())
    .find(|path| {
        path.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("transition-0-")
    })
    .unwrap();
    let old_seed_value: serde_json::Value = read_json(&old_seed_path);
    assert!(old_seed_value.get("resume_frozen_chain").is_none());

    let before = snapshot_tree(&store.run_dir(&run_id));
    let task_root_before = snapshot_tree(task_root);
    let pin_path =
        crate::command::workflow_task_set::acceptance_pin_path(project.path(), task_root);
    let publish_lock = archon_workflow::task_set_publish_lock::lock_path(&pin_path);
    let lock_existed_before = publish_lock.exists();
    let output = crate::command::workflow_decompose::dry_run_fixed_decomposition(
        project.path(),
        &run_id,
        &launch_config(project.path()),
    )
    .await
    .unwrap();
    assert!(
        !output.contains("capability=verify-frozen-acceptance"),
        "{output}"
    );
    assert!(
        !output.contains("capability=verify-frozen-skeleton"),
        "{output}"
    );
    assert!(output.contains("capability=freeze-acceptance"), "{output}");
    assert!(output.contains("capability=freeze-skeleton"), "{output}");
    assert!(output.contains("acceptance-author"), "{output}");
    assert_eq!(before, snapshot_tree(&store.run_dir(&run_id)));
    assert_eq!(task_root_before, snapshot_tree(task_root));
    assert_eq!(
        lock_existed_before,
        publish_lock.exists(),
        "dry-run created a publish lock"
    );
    assert_eq!(contract_before, std::fs::read(&contract_path).unwrap());
    assert_eq!(skeleton_before, std::fs::read(&skeleton_path).unwrap());
    let launch_arguments: serde_json::Value =
        read_json(&store.run_dir(&run_id).join(FIXED_ARGUMENTS_PATH));
    assert_eq!(launch_arguments["frozenChain"]["acceptance"], false);
}

#[tokio::test]
async fn dry_run_without_transition_replays_unrecorded_freezes_and_writes_nothing() {
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

    let run_before = snapshot_tree(&store.run_dir(&run_id));
    let task_before = snapshot_tree(task_root);
    let pin = crate::command::workflow_task_set::acceptance_pin_path(project.path(), task_root);
    let lock = archon_workflow::task_set_publish_lock::lock_path(&pin);
    let lock_before = lock.exists();
    let output = crate::command::workflow_decompose::dry_run_fixed_decomposition(
        project.path(),
        &run_id,
        &launch_config(project.path()),
    )
    .await
    .unwrap();

    assert!(
        output.contains("seed=none (no harness transition)"),
        "{output}"
    );
    assert!(
        !output.contains("capability=verify-frozen-acceptance"),
        "{output}"
    );
    assert!(
        !output.contains("capability=verify-frozen-skeleton"),
        "{output}"
    );
    assert!(output.contains("capability=freeze-acceptance"), "{output}");
    assert!(output.contains("capability=freeze-skeleton"), "{output}");
    assert!(output.contains("acceptance-author"), "{output}");
    assert_eq!(run_before, snapshot_tree(&store.run_dir(&run_id)));
    assert_eq!(task_before, snapshot_tree(task_root));
    assert_eq!(lock_before, lock.exists(), "dry-run created a publish lock");
}
