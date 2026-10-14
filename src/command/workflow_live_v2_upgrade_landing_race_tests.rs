use super::*;

fn current_body_logic_version() -> u32 {
    crate::command::workflow_host_command_logic::versions()["land-task-body"]
}

#[tokio::test]
async fn disk_change_after_judgement_is_published_as_the_new_candidate() {
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
        const body = ["```yaml", "task_id: TASK-X-010", "title: Round 1",
            "complexity: low", "status: ready", "depends_on: []", "blocks: []",
            "implements: []", "required_env_keys: []", "required_tools: []",
            "deliverable_contracts: []", "```", "", "# Scope", "", "Round 1 body.",
            "", "# Focused Tests", "", "- `cargo test -p archon-workflow --lib`", ""].join("\n");
        for (let i = 0; i < 3; i++) await w.hostCommand("land-task-body", {stdin: body, target});
        return "seeded";
    }"##;
    const REPLAY_OLD: &str = r##"async function workflow(w) {
        const target = {taskId: "TASK-X-010", fileName: "TASK-X-010.md"};
        const body = ["```yaml", "task_id: TASK-X-010", "title: Round 1",
            "complexity: low", "status: ready", "depends_on: []", "blocks: []",
            "implements: []", "required_env_keys: []", "required_tools: []",
            "deliverable_contracts: []", "```", "", "# Scope", "", "Round 1 body.",
            "", "# Focused Tests", "", "- `cargo test -p archon-workflow --lib`", ""].join("\n");
        return await w.hostCommand("land-task-body", {stdin: body, target});
    }"##;
    let first = Arc::new(
        FixedHostCommandExecutor::with_process(
            crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("rev-1")
                .unwrap(),
            context.clone(),
            run.store.run_dir(&run.run_id),
            Arc::new(RealLandingProcess {
                executed: Arc::new(Mutex::new(Vec::new())),
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
    let disk_before = std::fs::read(&task_path).unwrap();
    assert_eq!(
        archon_workflow::task_set_contract::content_digest(&disk_before),
        receipt_digest
    );
    let changed = b"```yaml\ntask_id: TASK-X-010\ntitle: Race Candidate\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n# Scope\n\nCandidate written after judgement.\n\n# Focused Tests\n\n- `cargo test -p archon-workflow --lib`\n".to_vec();
    let changed_digest = archon_workflow::task_set_contract::content_digest(&changed);
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
    crate::command::workflow_test_seams::mutate_after_landing_rejudge(
        &landing.call.id,
        task_path.clone(),
        changed.clone(),
    );
    let replay_lease = Arc::new(
        crate::command::workflow_executor_lease::acquire(
            &run.store.run_dir(&run.run_id),
            &run.run_id,
        )
        .unwrap(),
    );
    replay_lease.record_executor().unwrap();
    let answer = run
        .run_with_lease(replay.clone(), REPLAY_OLD, Some(replay_lease))
        .await;
    let disk_after = std::fs::read(&task_path).unwrap();
    let evidence_path = run
        .store
        .run_dir(&run.run_id)
        .join("landing-evidence")
        .join(format!("TASK-X-010-{changed_digest}.bin"));
    let evidence_bytes = std::fs::read(&evidence_path).unwrap();
    println!(
        "EVIDENCE 447j BEFORE race subject=TASK-X-010 landing={} receipt_blake3={receipt_digest} disk_blake3={changed_digest}",
        landing.call.id
    );
    println!(
        "EVIDENCE 447l AFTER post-judge evidence_exact={} disk_blake3={} verdict=new-candidate paused=false",
        evidence_bytes == changed,
        archon_workflow::task_set_contract::content_digest(&disk_after)
    );
    assert_eq!(answer["gateEnvelope"]["report"], "accepted");
    assert_eq!(evidence_bytes, changed);
    assert_ne!(
        archon_workflow::task_set_contract::content_digest(&disk_after),
        receipt_digest
    );
    assert_eq!(replay_executed.lock().unwrap().len(), 2);
    assert_eq!(
        v2.load_call_records()
            .unwrap()
            .iter()
            .filter(|record| record.call.method == WorkflowV2HostMethod::HostCommand)
            .count(),
        3
    );
    assert_ne!(
        run.store.load_state(&run.run_id).unwrap().status,
        archon_workflow::RunStatus::Paused
    );
    assert_eq!(
        archon_workflow::task_set_contract::content_digest(&disk_after),
        answer["publicationReceipt"]["entries"][0]["blake3"]
            .as_str()
            .unwrap()
    );
}

async fn run_unrebuildable_receipt_candidate(findings: bool, missing: bool) {
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
        const body = ["```yaml", "task_id: TASK-X-010", "title: Round 1",
            "complexity: low", "status: ready", "depends_on: []", "blocks: []",
            "implements: []", "required_env_keys: []", "required_tools: []",
            "deliverable_contracts: []", "```", "", "# Scope", "", "Round 1 body.",
            "", "# Focused Tests", "", "- `cargo test -p archon-workflow --lib`", ""].join("\n");
        for (let i = 0; i < 3; i++) await w.hostCommand("land-task-body", {stdin: body, target});
        return "seeded";
    }"##;
    const REPLAY: &str = r##"async function workflow(w) {
        const target = {taskId: "TASK-X-010", fileName: "TASK-X-010.md"};
        const body = ["```yaml", "task_id: TASK-X-010", "title: Round 1",
            "complexity: low", "status: ready", "depends_on: []", "blocks: []",
            "implements: []", "required_env_keys: []", "required_tools: []",
            "deliverable_contracts: []", "```", "", "# Scope", "", "Round 1 body.",
            "", "# Focused Tests", "", "- `cargo test -p archon-workflow --lib`", ""].join("\n");
        return await w.hostCommand("land-task-body", {stdin: body, target});
    }"##;
    let first = Arc::new(
        FixedHostCommandExecutor::with_process(
            crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("rev-1")
                .unwrap(),
            context.clone(),
            run.store.run_dir(&run.run_id),
            Arc::new(RealLandingProcess {
                executed: Arc::new(Mutex::new(Vec::new())),
                staged_task_reads: Arc::new(Mutex::new(Vec::new())),
                refuse_first_two: false,
                mutate_frozen_task: false,
                mutate_destination_on_execute: None,
            }),
        )
        .with_logic_version("land-task-body", Some(9)),
    );
    let lease = Arc::new(
        crate::command::workflow_executor_lease::acquire(
            &run.store.run_dir(&run.run_id),
            &run.run_id,
        )
        .unwrap(),
    );
    lease.record_executor().unwrap();
    run.run_with_lease(first, LANDINGS, Some(lease)).await;

    let v2 = WorkflowV2ResultStore::new(run.store.run_dir(&run.run_id).join("v2"));
    let mut landing = archon_workflow::v2::script::landing_lineage::LandingLineage::of(
        &v2.load_call_records().unwrap(),
        "land-task-body",
        "TASK-X-010",
    )
    .current()
    .unwrap()
    .clone();
    let old_receipt = landing.result.data["publicationReceipt"]["entries"][0]["blake3"]
        .as_str()
        .unwrap()
        .to_owned();
    landing.result.data["publicationReceipt"]["entries"][0]["blake3"] =
        serde_json::json!("unrebuildable-receipt");
    v2.restore_call_record(&landing).unwrap();
    let candidate = b"```yaml\ntask_id: TASK-X-010\ntitle: Disk Candidate\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n# Scope\n\nDisk candidate body.\n\n# Focused Tests\n\n- `cargo test -p archon-workflow --lib`\n";
    if missing {
        std::fs::remove_file(&task_path).unwrap();
    } else {
        std::fs::write(&task_path, candidate).unwrap();
    }
    let candidate_digest = archon_workflow::task_set_contract::content_digest(candidate);
    if missing {
        println!(
            "EVIDENCE 447l BEFORE missing-unrebuildable subject=TASK-X-010 exists=false receipt=unrebuildable"
        );
    } else {
        println!(
            "EVIDENCE 447j BEFORE unrebuildable subject=TASK-X-010 receipt_blake3=unrebuildable-receipt disk_blake3={candidate_digest} candidate_bytes=disk-candidate"
        );
    }
    let executed = Arc::new(Mutex::new(Vec::new()));
    let replay = Arc::new(
        FixedHostCommandExecutor::with_process(
            crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("rev-2")
                .unwrap(),
            context.clone(),
            run.store.run_dir(&run.run_id),
            Arc::new(RealLandingProcess {
                executed: Arc::clone(&executed),
                staged_task_reads: Arc::new(Mutex::new(Vec::new())),
                refuse_first_two: findings,
                mutate_frozen_task: false,
                mutate_destination_on_execute: None,
            }),
        )
        .with_logic_version("land-task-body", Some(current_body_logic_version())),
    );
    let lease = Arc::new(
        crate::command::workflow_executor_lease::acquire(
            &run.store.run_dir(&run.run_id),
            &run.run_id,
        )
        .unwrap(),
    );
    lease.record_executor().unwrap();
    let result = run.run_with_lease(replay, REPLAY, Some(lease)).await;
    let disk_after = std::fs::read(&task_path).unwrap();
    let receipt_digest = result["publicationReceipt"]["entries"]
        .as_array()
        .and_then(|entries| {
            entries
                .iter()
                .find(|entry| entry["destination_path"] == task_path.to_string_lossy().as_ref())
        })
        .and_then(|entry| entry["blake3"].as_str());
    let evidence_dir = run.store.run_dir(&run.run_id).join("landing-evidence");
    let evidence = match std::fs::read_dir(&evidence_dir) {
        Ok(entries) => entries
            .map(|entry| std::fs::read(entry.unwrap().path()).unwrap())
            .any(|bytes| bytes == candidate),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => panic!(
            "reading evidence directory {}: {error}",
            evidence_dir.display()
        ),
    };
    println!(
        "EVIDENCE 447j AFTER unrebuildable evidence_exact={evidence} disk_blake3={} receipt_equals_disk={} verdict={} paused={}",
        archon_workflow::task_set_contract::content_digest(&disk_after),
        receipt_digest
            == Some(archon_workflow::task_set_contract::content_digest(&disk_after).as_str()),
        result["gateEnvelope"]["report"],
        run.store.load_state(&run.run_id).unwrap().status == archon_workflow::RunStatus::Paused
    );
    if missing {
        println!(
            "EVIDENCE 447l AFTER reauthor_executed={} file_restored={} accepted={} paused={}",
            executed.lock().unwrap().len() == 1,
            task_path.is_file(),
            result["gateEnvelope"]["report"] == "accepted",
            run.store.load_state(&run.run_id).unwrap().status == archon_workflow::RunStatus::Paused
        );
    }
    assert_eq!(evidence, !missing);
    assert_eq!(executed.lock().unwrap().len(), 1);
    if missing {
        assert!(executed.lock().unwrap()[0].contains("title: Round 1"));
    } else {
        assert!(executed.lock().unwrap()[0].contains("Disk Candidate"));
    }
    assert_ne!(
        run.store.load_state(&run.run_id).unwrap().status,
        archon_workflow::RunStatus::Paused
    );
    assert_eq!(old_receipt.len(), 64);
    if findings {
        assert_eq!(result["gateEnvelope"]["report"], "refused");
    } else {
        assert_eq!(result["gateEnvelope"]["report"], "accepted");
        let disk_digest = archon_workflow::task_set_contract::content_digest(&disk_after);
        assert_eq!(receipt_digest, Some(disk_digest.as_str()));
    }
}

#[tokio::test]
async fn unrebuildable_receipt_disk_candidate_is_published_with_a_matching_receipt() {
    run_unrebuildable_receipt_candidate(false, false).await;
}

#[tokio::test]
async fn unrebuildable_receipt_findings_return_to_the_author_loop() {
    run_unrebuildable_receipt_candidate(true, false).await;
}

#[tokio::test]
async fn unrebuildable_missing_body_routes_to_the_author_without_pausing() {
    run_unrebuildable_receipt_candidate(false, true).await;
}
