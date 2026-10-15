use super::*;

fn current_body_logic_version() -> u32 {
    crate::command::workflow_host_command_logic::versions()["land-task-body"]
}

#[tokio::test]
async fn a_second_write_after_restore_is_judged_as_a_candidate_without_a_loop() {
    let run = Run::new();
    let context = run.context.clone();
    let task_path = context.task_root.join("TASK-X-010.md");
    crate::command::workflow_host_command_exec_tests::seed_frozen_chain(&context, &task_path);
    crate::command::workflow_decompose_state_tests::seed_state(
        &run.store,
        &run.run_id,
        &context.task_root.join(".decompose.log"),
    );
    const SEED: &str = r##"async function workflow(w) { const target = {taskId:"TASK-X-010",fileName:"TASK-X-010.md"}; const body=["```yaml","task_id: TASK-X-010","title: Round 1","complexity: low","status: ready","depends_on: []","blocks: []","implements: []","required_env_keys: []","required_tools: []","deliverable_contracts: []","```","","# Scope","","Round 1 body.","","# Focused Tests","","- `cargo test -p archon-workflow --lib`", ""].join("\n"); for(let i=0;i<3;i++) await w.hostCommand("land-task-body",{stdin:body,target}); return "seeded"; }"##;
    const REPLAY: &str = r##"async function workflow(w) { const target = {taskId:"TASK-X-010",fileName:"TASK-X-010.md"}; const body=["```yaml","task_id: TASK-X-010","title: Round 1","complexity: low","status: ready","depends_on: []","blocks: []","implements: []","required_env_keys: []","required_tools: []","deliverable_contracts: []","```","","# Scope","","Round 1 body.","","# Focused Tests","","- `cargo test -p archon-workflow --lib`", ""].join("\n"); return await w.hostCommand("land-task-body",{stdin:body,target}); }"##;
    let mk = |revision: &'static str, executed: Arc<Mutex<Vec<String>>>| {
        Arc::new(
            FixedHostCommandExecutor::with_process(
                crate::command::workflow_host_command_catalog::fixed_decomposition_catalog(
                    revision,
                )
                .unwrap(),
                context.clone(),
                run.store.run_dir(&run.run_id),
                Arc::new(RealLandingProcess {
                    executed,
                    staged_task_reads: Arc::new(Mutex::new(Vec::new())),
                    refuse_first_two: false,
                    mutate_frozen_task: false,
                    mutate_destination_on_execute: None,
                }),
            )
            .with_logic_version(
                "land-task-body",
                Some(if revision == "rev-1" {
                    9
                } else {
                    current_body_logic_version()
                }),
            ),
        )
    };
    let lease = Arc::new(
        crate::command::workflow_executor_lease::acquire(
            &run.store.run_dir(&run.run_id),
            &run.run_id,
        )
        .unwrap(),
    );
    lease.record_executor().unwrap();
    run.run_with_lease(
        mk("rev-1", Arc::new(Mutex::new(Vec::new()))),
        SEED,
        Some(lease),
    )
    .await;
    let v2 = WorkflowV2ResultStore::new(run.store.run_dir(&run.run_id).join("v2"));
    let landing = archon_workflow::v2::script::landing_lineage::LandingLineage::of(
        &v2.load_call_records().unwrap(),
        "land-task-body",
        "TASK-X-010",
    )
    .current()
    .unwrap()
    .clone();
    let displaced = b"first outside write".to_vec();
    let between_read_and_restore = b"write in the former read/restore window".to_vec();
    let newest = b"```yaml\ntask_id: TASK-X-010\ntitle: Newest Candidate\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n# Scope\n\nNewest outside candidate.\n\n# Focused Tests\n\n- `cargo test -p archon-workflow --lib`\n".to_vec();
    let displaced_digest = archon_workflow::task_set_contract::content_digest(&displaced);
    let race_digest = archon_workflow::task_set_contract::content_digest(&between_read_and_restore);
    let newest_digest = archon_workflow::task_set_contract::content_digest(&newest);
    std::fs::write(&task_path, &displaced).unwrap();
    crate::command::workflow_test_seams::mutate_before_landing_restore(
        &landing.call.id,
        task_path.clone(),
        between_read_and_restore.clone(),
    );
    crate::command::workflow_test_seams::mutate_after_landing_restore(
        &landing.call.id,
        task_path.clone(),
        newest.clone(),
    );
    println!(
        "EVIDENCE 447j BEFORE second-write receipt_blake3={} disk_blake3={displaced_digest} race_blake3={race_digest} newest_blake3={newest_digest}",
        landing.result.data["publicationReceipt"]["entries"][0]["blake3"]
            .as_str()
            .unwrap()
    );
    let executed = Arc::new(Mutex::new(Vec::new()));
    let lease = Arc::new(
        crate::command::workflow_executor_lease::acquire(
            &run.store.run_dir(&run.run_id),
            &run.run_id,
        )
        .unwrap(),
    );
    lease.record_executor().unwrap();
    let answer = run
        .run_with_lease(mk("rev-2", Arc::clone(&executed)), REPLAY, Some(lease))
        .await;
    let disk_after = std::fs::read(&task_path).unwrap();
    let evidence_dir = run.store.run_dir(&run.run_id).join("landing-evidence");
    let evidence_first =
        std::fs::read(evidence_dir.join(format!("TASK-X-010-{displaced_digest}.bin"))).unwrap();
    let evidence_race_path = std::fs::read_dir(&evidence_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| std::fs::read(path).is_ok_and(|bytes| bytes == between_read_and_restore))
        .expect("the write in the former race window is captured in evidence");
    let evidence_race = std::fs::read(&evidence_race_path).unwrap();
    let evidence_latest =
        std::fs::read(evidence_dir.join(format!("TASK-X-010-{newest_digest}.bin"))).unwrap();
    let executed = executed.lock().unwrap();
    let newest_judged = executed
        .first()
        .is_some_and(|bytes| bytes.contains("Newest Candidate"));
    let call_count = executed.len();
    println!(
        "EVIDENCE 447j AFTER second-write newest_judged={} newest_published={} displaced_evidence_exact={} race_evidence_exact={} newest_evidence_exact={} calls={} paused={}",
        newest_judged,
        answer["publicationReceipt"]["entries"]
            .as_array()
            .is_some_and(
                |entries| entries.iter().any(|entry| entry["destination_path"]
                    == task_path.to_string_lossy().as_ref()
                    && entry["blake3"]
                        == archon_workflow::task_set_contract::content_digest(&disk_after))
            ),
        evidence_first == displaced,
        evidence_race == between_read_and_restore,
        evidence_latest == newest,
        call_count,
        run.store.load_state(&run.run_id).unwrap().status == archon_workflow::RunStatus::Paused
    );
    let race_event = std::fs::read_to_string(run.store.run_dir(&run.run_id).join("events.jsonl"))
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|event| {
            event["kind"] == "landing_receipt_displaced"
                && event["detail"]["landing_call_id"] == landing.call.id
                && event["detail"]["disk_blake3"] == race_digest
        })
        .expect("captured evidence digest is recorded in its event");
    assert_eq!(
        race_event["detail"]["evidence_path"],
        evidence_race_path
            .strip_prefix(run.store.run_dir(&run.run_id))
            .unwrap()
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(call_count, 1);
    assert!(newest_judged);
    drop(executed);
    assert_eq!(evidence_first, displaced);
    assert_eq!(evidence_race, between_read_and_restore);
    assert_eq!(evidence_latest, newest);
    assert_eq!(answer["gateEnvelope"]["report"], "accepted");
    assert_ne!(
        run.store.load_state(&run.run_id).unwrap().status,
        archon_workflow::RunStatus::Paused
    );
}
