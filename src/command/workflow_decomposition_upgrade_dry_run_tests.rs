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
    assert!(output.contains("seed=existing transition 1"), "{output}");
    assert!(output.contains("healed_groups_would_remove=1"), "{output}");
    assert!(output.contains("seed subject=acceptance"), "{output}");
    assert!(
        output.contains("call[0] id=acceptance-author-AC-X-001-"),
        "{output}"
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
                state.identity,
                previous,
            ),
        ],
    };
    store
        .write_run_json(&run_id, TRANSITIONS_PATH, &transitions)
        .unwrap();
    let before = snapshot_tree(&run_dir);
    let output = crate::command::workflow_decompose::dry_run_fixed_decomposition(
        project.path(),
        &run_id,
        &launch_config(project.path()),
    )
    .await
    .unwrap();
    assert!(
        output.contains("seed=would-record transition 2 (derived in memory)"),
        "{output}"
    );
    assert_eq!(
        before,
        snapshot_tree(&run_dir),
        "dry run changed a run file"
    );
}
