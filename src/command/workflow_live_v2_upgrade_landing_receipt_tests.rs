use super::*;

fn current_body_logic_version() -> u32 {
    crate::command::workflow_host_command_logic::versions()["land-task-body"]
}

#[tokio::test]
async fn a_missing_current_landing_file_is_restored_and_rejudged_without_pausing() {
    let run = Run::new();
    let context = run.context.clone();
    let task_path = context.task_root.join("TASK-X-010.md");
    crate::command::workflow_host_command_exec_tests::seed_frozen_chain(&context, &task_path);
    crate::command::workflow_decompose_state_tests::seed_state(
        &run.store,
        &run.run_id,
        &context.task_root.join(".decompose.log"),
    );
    const SCRIPT: &str = r##"async function workflow(w) {
        const target = {taskId:"TASK-X-010",fileName:"TASK-X-010.md"};
        const body = ["```yaml","task_id: TASK-X-010","title: Round A","complexity: low","status: ready","depends_on: []","blocks: []","implements: []","required_env_keys: []","required_tools: []","deliverable_contracts: []","```","","# Scope","","Round A body.","","# Focused Tests","","- `cargo test -p archon-workflow --lib`", ""].join("\n");
        await w.hostCommand("land-task-body",{stdin:body,target});
        return await w.hostCommand("land-task-body",{stdin:body,target});
    }"##;
    let mk = |revision: &'static str, executed: Arc<Mutex<Vec<String>>>, version| {
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
            .with_logic_version("land-task-body", Some(version)),
        )
    };
    let lease = || {
        let lease = Arc::new(
            crate::command::workflow_executor_lease::acquire(
                &run.store.run_dir(&run.run_id),
                &run.run_id,
            )
            .unwrap(),
        );
        lease.record_executor().unwrap();
        lease
    };
    let first = mk("rev-1", Arc::new(Mutex::new(Vec::new())), 9);
    run.run_with_lease(first, SCRIPT, Some(lease())).await;
    let v2 = WorkflowV2ResultStore::new(run.store.run_dir(&run.run_id).join("v2"));
    let records = v2.load_call_records().unwrap();
    let landing = archon_workflow::v2::script::landing_lineage::LandingLineage::of(
        &records,
        "land-task-body",
        "TASK-X-010",
    )
    .current()
    .unwrap()
    .clone();
    std::fs::remove_file(&task_path).unwrap();
    println!(
        "EVIDENCE 447l BEFORE missing landing={} exists=false receipt={}",
        landing.call.id,
        landing.result.data["publicationReceipt"]["entries"][0]["blake3"]
            .as_str()
            .unwrap()
    );
    let judged = Arc::new(Mutex::new(Vec::new()));
    let replay = mk("rev-2", Arc::clone(&judged), current_body_logic_version());
    let answer = run.run_with_lease(replay, SCRIPT, Some(lease())).await;
    let disk = std::fs::read(&task_path).unwrap();
    let digest = archon_workflow::task_set_contract::content_digest(&disk);
    println!(
        "EVIDENCE 447l AFTER exists=true restored_and_judged={} calls={} paused={}",
        digest
            == landing.result.data["publicationReceipt"]["entries"][0]["blake3"]
                .as_str()
                .unwrap(),
        judged.lock().unwrap().len(),
        run.store.load_state(&run.run_id).unwrap().status == archon_workflow::RunStatus::Paused
    );
    assert_eq!(answer["gateEnvelope"]["report"], "accepted");
    assert_eq!(
        digest,
        landing.result.data["publicationReceipt"]["entries"][0]["blake3"]
            .as_str()
            .unwrap()
    );
    assert_eq!(judged.lock().unwrap().len(), 2);
    assert_ne!(
        run.store.load_state(&run.run_id).unwrap().status,
        archon_workflow::RunStatus::Paused
    );
}

#[tokio::test]
async fn a_stale_current_landing_is_rejudged_from_disk_without_publishing() {
    let run = Run::new();
    let context = run.context.clone();
    let task_path = context.task_root.join("TASK-X-010.md");
    crate::command::workflow_host_command_exec_tests::seed_frozen_chain(&context, &task_path);
    crate::command::workflow_decompose_state_tests::seed_state(
        &run.store,
        &run.run_id,
        &context.task_root.join(".decompose.log"),
    );
    const LANDINGS: &str = r##"async function workflow(w) {
        const target = {taskId: "TASK-X-010", fileName: "TASK-X-010.md"};
        const body = (title) => [
            "```yaml", "task_id: TASK-X-010", "title: " + title,
            "complexity: low", "status: ready", "depends_on: []", "blocks: []",
            "implements: []", "required_env_keys: []", "required_tools: []",
            "deliverable_contracts: []", "```", "", "# Scope", "",
            title + " body.", "", "# Focused Tests", "",
            "- `cargo test -p archon-workflow --lib`", ""
        ].join("\n");
        await w.hostCommand("land-task-body", {stdin: body("Round A"), target});
        await w.hostCommand("land-task-body", {stdin: body("Round B"), target});
        return "seeded";
    }"##;
    const REPLAY_OLD: &str = r##"async function workflow(w) {
        const target = {taskId: "TASK-X-010", fileName: "TASK-X-010.md"};
        const body = [
            "```yaml", "task_id: TASK-X-010", "title: Round A",
            "complexity: low", "status: ready", "depends_on: []", "blocks: []",
            "implements: []", "required_env_keys: []", "required_tools: []",
            "deliverable_contracts: []", "```", "", "# Scope", "",
            "Round A body.", "", "# Focused Tests", "",
            "- `cargo test -p archon-workflow --lib`", ""
        ].join("\n");
        const landing = await w.hostCommand("land-task-body", {stdin: body, target});
        await w.finalReport("current-body", {
            status: "accepted", inputs: {landing}, task: "Report the current task body"
        });
        return landing;
    }"##;

    let first_executed = Arc::new(Mutex::new(Vec::new()));
    let first = Arc::new(
        FixedHostCommandExecutor::with_process(
            crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("rev-1")
                .unwrap(),
            context.clone(),
            run.store.run_dir(&run.run_id),
            Arc::new(RealLandingProcess {
                executed: Arc::clone(&first_executed),
                staged_task_reads: Arc::new(Mutex::new(Vec::new())),
                refuse_first_two: false,
                mutate_frozen_task: false,
                mutate_destination_on_execute: None,
            }),
        )
        .with_logic_version("land-task-body", Some(9)),
    );
    let first_lease = Arc::new(
        crate::command::workflow_executor_lease::acquire(
            &run.store.run_dir(&run.run_id),
            &run.run_id,
        )
        .unwrap(),
    );
    first_lease.record_executor().unwrap();
    run.run_with_lease(first, LANDINGS, Some(first_lease)).await;
    assert_eq!(first_executed.lock().unwrap().len(), 2);
    let disk_before = std::fs::read(&task_path).unwrap();
    let digest_before = archon_workflow::task_set_contract::content_digest(&disk_before);
    let v2 = WorkflowV2ResultStore::new(run.store.run_dir(&run.run_id).join("v2"));
    let current_before = archon_workflow::v2::script::landing_lineage::LandingLineage::of(
        &v2.load_call_records().unwrap(),
        "land-task-body",
        "TASK-X-010",
    )
    .current()
    .unwrap()
    .clone();
    println!(
        "EVIDENCE 447b BEFORE requests=2 receipts={:?} disk_digest={digest_before}",
        current_before.result.data["publicationReceipt"]["entries"]
    );

    let second_executed = Arc::new(Mutex::new(Vec::new()));
    let staged_task_reads = Arc::new(Mutex::new(Vec::new()));
    let second = Arc::new(
        FixedHostCommandExecutor::with_process(
            crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("rev-2")
                .unwrap(),
            context.clone(),
            run.store.run_dir(&run.run_id),
            Arc::new(RealLandingProcess {
                executed: Arc::clone(&second_executed),
                staged_task_reads: Arc::clone(&staged_task_reads),
                refuse_first_two: false,
                mutate_frozen_task: false,
                mutate_destination_on_execute: None,
            }),
        )
        .with_logic_version("land-task-body", Some(current_body_logic_version())),
    );
    let second_lease = Arc::new(
        crate::command::workflow_executor_lease::acquire(
            &run.store.run_dir(&run.run_id),
            &run.run_id,
        )
        .unwrap(),
    );
    second_lease.record_executor().unwrap();
    let outcome = run
        .run_with_lease(second.clone(), REPLAY_OLD, Some(second_lease))
        .await;
    let disk_after = std::fs::read(&task_path).unwrap();
    let after_digest = archon_workflow::task_set_contract::content_digest(&disk_after);
    let receipt_digest = outcome["publicationReceipt"]["entries"][0]["blake3"]
        .as_str()
        .unwrap();
    assert_eq!(outcome["logicVersion"], current_body_logic_version());
    println!(
        "EVIDENCE 447b AFTER answer_receipt={receipt_digest} logicVersion={} published={}",
        outcome["logicVersion"],
        after_digest != digest_before
    );
    assert_eq!(receipt_digest, after_digest);
    assert_eq!(disk_after, disk_before);
    assert_eq!(second_executed.lock().unwrap().len(), 1);
    let staged_reads = staged_task_reads.lock().unwrap();
    assert_eq!(staged_reads.len(), 1);
    let (_staged_path, staged_bytes) = &staged_reads[0];
    let receipt_task_digest = outcome["publicationReceipt"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["destination_path"] == task_path.to_string_lossy().as_ref())
        .unwrap()["blake3"]
        .as_str()
        .unwrap();
    assert_eq!(
        archon_workflow::task_set_contract::content_digest(staged_bytes),
        receipt_task_digest
    );
    let final_report = v2
        .load_call_records()
        .unwrap()
        .into_iter()
        .find(|record| record.call.method == WorkflowV2HostMethod::FinalReport)
        .expect("final report record after resumed landing");
    assert_eq!(final_report.result.status, WorkflowV2Status::Accepted);

    let third_executed = Arc::new(Mutex::new(Vec::new()));
    let third = Arc::new(
        FixedHostCommandExecutor::with_process(
            crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("rev-2")
                .unwrap(),
            context.clone(),
            run.store.run_dir(&run.run_id),
            Arc::new(RealLandingProcess {
                executed: Arc::clone(&third_executed),
                staged_task_reads: Arc::new(Mutex::new(Vec::new())),
                refuse_first_two: false,
                mutate_frozen_task: false,
                mutate_destination_on_execute: None,
            }),
        )
        .with_logic_version("land-task-body", Some(current_body_logic_version())),
    );
    let third_lease = Arc::new(
        crate::command::workflow_executor_lease::acquire(
            &run.store.run_dir(&run.run_id),
            &run.run_id,
        )
        .unwrap(),
    );
    third_lease.record_executor().unwrap();
    run.run_with_lease(third, REPLAY_OLD, Some(third_lease))
        .await;
    assert!(third_executed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_write_between_restore_and_rejudge_becomes_the_new_candidate() {
    let run = Run::new();
    let context = run.context.clone();
    let task_path = context.task_root.join("TASK-X-010.md");
    crate::command::workflow_host_command_exec_tests::seed_frozen_chain(&context, &task_path);
    crate::command::workflow_decompose_state_tests::seed_state(
        &run.store,
        &run.run_id,
        &context.task_root.join(".decompose.log"),
    );
    const LANDINGS: &str = r##"async function workflow(w) {
        const target = {taskId: "TASK-X-010", fileName: "TASK-X-010.md"};
        const body = [
            "```yaml", "task_id: TASK-X-010", "title: Round 1",
            "complexity: low", "status: ready", "depends_on: []", "blocks: []",
            "implements: []", "required_env_keys: []", "required_tools: []",
            "deliverable_contracts: []", "```", "", "# Scope", "",
            "Round 1 body.", "", "# Focused Tests", "",
            "- `cargo test -p archon-workflow --lib`", ""
        ].join("\n");
        for (let i = 0; i < 3; i++) {
            await w.hostCommand("land-task-body", {stdin: body, target});
        }
        return "seeded";
    }"##;
    const REPLAY_OLD: &str = r##"async function workflow(w) {
        const target = {taskId: "TASK-X-010", fileName: "TASK-X-010.md"};
        const body = [
            "```yaml", "task_id: TASK-X-010", "title: Round 1",
            "complexity: low", "status: ready", "depends_on: []", "blocks: []",
            "implements: []", "required_env_keys: []", "required_tools: []",
            "deliverable_contracts: []", "```", "", "# Scope", "",
            "Round 1 body.", "", "# Focused Tests", "",
            "- `cargo test -p archon-workflow --lib`", ""
        ].join("\n");
        return await w.hostCommand("land-task-body", {stdin: body, target});
    }"##;
    let first_executed = Arc::new(Mutex::new(Vec::new()));
    let first = Arc::new(
        FixedHostCommandExecutor::with_process(
            crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("rev-1")
                .unwrap(),
            context.clone(),
            run.store.run_dir(&run.run_id),
            Arc::new(RealLandingProcess {
                executed: Arc::clone(&first_executed),
                staged_task_reads: Arc::new(Mutex::new(Vec::new())),
                refuse_first_two: false,
                mutate_frozen_task: false,
                mutate_destination_on_execute: None,
            }),
        )
        .with_logic_version("land-task-body", Some(9)),
    );
    let first_lease = Arc::new(
        crate::command::workflow_executor_lease::acquire(
            &run.store.run_dir(&run.run_id),
            &run.run_id,
        )
        .unwrap(),
    );
    first_lease.record_executor().unwrap();
    run.run_with_lease(first, LANDINGS, Some(first_lease)).await;
    assert_eq!(first_executed.lock().unwrap().len(), 3);
    let v2 = WorkflowV2ResultStore::new(run.store.run_dir(&run.run_id).join("v2"));
    let records_before = v2.load_call_records().unwrap();
    let landing = archon_workflow::v2::script::landing_lineage::LandingLineage::of(
        &records_before,
        "land-task-body",
        "TASK-X-010",
    )
    .current()
    .unwrap()
    .clone();
    let receipt_digest = landing.result.data["publicationReceipt"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["destination_path"] == task_path.to_string_lossy().as_ref())
        .unwrap()["blake3"]
        .as_str()
        .unwrap()
        .to_owned();
    std::fs::write(&task_path, b"operator edit").unwrap();
    let disk_digest = archon_workflow::task_set_contract::content_digest(b"operator edit");
    let candidate_bytes = landing
        .call
        .options
        .host_command
        .as_ref()
        .unwrap()
        .stdin
        .as_deref()
        .unwrap()
        .replace("Round 1", "Fresh Racing Candidate")
        .into_bytes();
    crate::command::workflow_test_seams::mutate_before_landing_rejudge(
        &landing.call.id,
        task_path.clone(),
        candidate_bytes,
    );
    crate::command::workflow_test_seams::assert_evidence_before_landing_restore(
        &landing.call.id,
        run.store
            .run_dir(&run.run_id)
            .join("landing-evidence")
            .join(format!("TASK-X-010-{disk_digest}.bin")),
    );

    let replay_executed = Arc::new(Mutex::new(Vec::new()));
    let replay = Arc::new(
        FixedHostCommandExecutor::with_process(
            crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("rev-2")
                .unwrap(),
            context.clone(),
            run.store.run_dir(&run.run_id),
            Arc::new(RealLandingProcess {
                executed: Arc::clone(&replay_executed),
                staged_task_reads: Arc::new(Mutex::new(Vec::new())),
                refuse_first_two: false,
                mutate_frozen_task: false,
                mutate_destination_on_execute: None,
            }),
        )
        .with_logic_version("land-task-body", Some(current_body_logic_version())),
    );
    let replay_lease = Arc::new(
        crate::command::workflow_executor_lease::acquire(
            &run.store.run_dir(&run.run_id),
            &run.run_id,
        )
        .unwrap(),
    );
    replay_lease.record_executor().unwrap();
    let result = run
        .run_with_lease(replay.clone(), REPLAY_OLD, Some(replay_lease))
        .await;
    let disk_after = std::fs::read(&task_path).unwrap();
    let run_dir = run.store.run_dir(&run.run_id);
    let evidence_files = std::fs::read_dir(run_dir.join("landing-evidence"))
        .map(|entries| entries.collect::<Result<Vec<_>, _>>().unwrap())
        .unwrap_or_default();
    let evidence_bytes = evidence_files
        .iter()
        .map(|entry| std::fs::read(entry.path()).unwrap())
        .find(|bytes| bytes == b"operator edit")
        .expect("displaced bytes are kept byte-for-byte");
    let events = std::fs::read_to_string(run_dir.join("events.jsonl")).unwrap();
    println!(
        "EVIDENCE 447j BEFORE subject=TASK-X-010 landing={} receipt_blake3={receipt_digest} disk_blake3={disk_digest} disk_bytes=operator-edit",
        landing.call.id
    );
    assert_eq!(
        result["gateEnvelope"]["report"], "accepted",
        "mismatch self-heals: {result:?}"
    );
    assert_eq!(evidence_bytes, b"operator edit");
    let answer_receipt_digest = result["publicationReceipt"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["destination_path"] == task_path.to_string_lossy().as_ref())
        .unwrap()["blake3"]
        .as_str()
        .unwrap();
    assert_eq!(
        archon_workflow::task_set_contract::content_digest(&disk_after),
        answer_receipt_digest
    );
    assert_ne!(answer_receipt_digest, receipt_digest);
    assert_eq!(replay_executed.lock().unwrap().len(), 1);
    let displaced_event = events
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|event| {
            event["kind"] == "landing_receipt_displaced"
                && event["detail"]["landing_call_id"] == landing.call.id
        })
        .expect("the event names the displaced landing");
    assert_eq!(displaced_event["detail"]["subject"], "TASK-X-010");
    assert_eq!(displaced_event["detail"]["receipt_blake3"], receipt_digest);
    assert_eq!(displaced_event["detail"]["disk_blake3"], disk_digest);
    assert_eq!(
        displaced_event["detail"]["evidence_path"],
        format!("landing-evidence/TASK-X-010-{disk_digest}.bin")
    );
    println!(
        "EVIDENCE 447l AFTER evidence_bytes_exact=true candidate_published=true disk_blake3={} paused=false",
        archon_workflow::task_set_contract::content_digest(&disk_after)
    );
    assert_eq!(
        v2.load_call_records()
            .unwrap()
            .iter()
            .filter(|record| record.call.method == WorkflowV2HostMethod::HostCommand)
            .count(),
        3,
        "only the three original landings remain persisted"
    );
    assert_ne!(
        run.store.load_state(&run.run_id).unwrap().status,
        archon_workflow::RunStatus::Paused
    );
}
