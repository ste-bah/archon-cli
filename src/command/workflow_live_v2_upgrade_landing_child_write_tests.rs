use super::*;

#[tokio::test]
async fn rejudge_child_write_to_frozen_task_cannot_change_the_live_task() {
    let run = Run::new();
    let context = run.context.clone();
    let task_path = context.task_root.join("TASK-X-010.md");
    crate::command::workflow_host_command_exec_tests::seed_frozen_chain(&context, &task_path);
    crate::command::workflow_decompose_state_tests::seed_state(
        &run.store,
        &run.run_id,
        &context.task_root.join(".decompose.log"),
    );
    let executed = Arc::new(Mutex::new(Vec::new()));
    let first = Arc::new(
        FixedHostCommandExecutor::with_process(
            crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("rev-1")
                .unwrap(),
            context.clone(),
            run.store.run_dir(&run.run_id),
            Arc::new(RealLandingProcess {
                executed: Arc::clone(&executed),
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
    run.run_with_lease(first, REAL_TASK_SCRIPT, Some(lease))
        .await;
    let records = WorkflowV2ResultStore::new(run.store.run_dir(&run.run_id).join("v2"))
        .load_call_records()
        .unwrap();
    let mut landing = records
        .iter()
        .rev()
        .find(|record| {
            record
                .call
                .options
                .host_command
                .as_ref()
                .is_some_and(|request| request.command_id == "land-task-body")
                && serde_json::from_value::<archon_workflow::HostCommandResult>(
                    record.result.data.clone(),
                )
                .is_ok_and(|outcome| outcome.publication_receipt.is_some())
        })
        .unwrap()
        .clone();
    let generation = run.store.load_state(&run.run_id).unwrap().generation;
    let mut bytes_before_rejudge = std::fs::read(&task_path).unwrap();
    let landed_body = String::from_utf8(bytes_before_rejudge.clone()).unwrap();
    let changed_frontmatter = landed_body.replacen("depends_on: []", "depends_on: [TASK-X-009]", 1);
    assert_ne!(changed_frontmatter, landed_body);
    bytes_before_rejudge = changed_frontmatter.into_bytes();
    std::fs::write(&task_path, &bytes_before_rejudge).unwrap();
    let mut receipt_outcome: archon_workflow::HostCommandResult =
        serde_json::from_value(landing.result.data.clone()).unwrap();
    let receipt = receipt_outcome.publication_receipt.as_mut().unwrap();
    let path_key = task_path.to_string_lossy();
    let receipt_entry = receipt
        .entries
        .iter_mut()
        .find(|entry| entry.destination_path == path_key)
        .expect("task body receipt entry");
    receipt_entry.byte_len = bytes_before_rejudge.len() as u64;
    receipt_entry.blake3 =
        archon_workflow::task_set_contract::content_digest(&bytes_before_rejudge);
    landing.result.data = serde_json::to_value(&receipt_outcome).unwrap();
    let staged_task_reads = Arc::new(Mutex::new(Vec::new()));
    let staging_root = context.run_staging_root.clone();
    let rejudge = FixedHostCommandExecutor::with_process(
        crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("rev-2")
            .unwrap(),
        context,
        run.store.run_dir(&run.run_id),
        Arc::new(RealLandingProcess {
            executed: Arc::clone(&executed),
            staged_task_reads: Arc::clone(&staged_task_reads),
            refuse_first_two: false,
            mutate_frozen_task: true,
            mutate_destination_on_execute: None,
        }),
    )
    .with_logic_version("land-task-body", Some(13));
    let outcome = rejudge
        .rejudge_landing(&landing, Some(generation), true)
        .await
        .expect("private staged task can be rejudged")
        .expect("current receipt still matches")
        .outcome;
    let bytes_after_rejudge = std::fs::read(&task_path).unwrap();
    assert_eq!(bytes_after_rejudge, bytes_before_rejudge);
    let staged_reads = staged_task_reads.lock().unwrap();
    assert_eq!(staged_reads.len(), 1);
    let (_staged_path, separately_read_staged_bytes) = &staged_reads[0];
    let expected_receipt_digest = receipt_outcome
        .publication_receipt
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .find(|entry| entry.destination_path == path_key)
        .unwrap()
        .blake3
        .clone();
    let staged_digest =
        archon_workflow::task_set_contract::content_digest(separately_read_staged_bytes);
    println!(
        "REJUDGE 447 receipt_digest={expected_receipt_digest} staged_read_digest={staged_digest} landed_body_digest={}",
        archon_workflow::task_set_contract::content_digest(&bytes_before_rejudge)
    );
    assert_eq!(staged_digest, expected_receipt_digest);
    assert_eq!(
        executed.lock().unwrap().last().unwrap().as_bytes(),
        bytes_before_rejudge,
        "re-judge must use the landed frontmatter even when it differs from the frozen frontmatter"
    );
    assert_eq!(outcome.gate_envelope.as_ref().unwrap().report, "accepted");
    assert!(outcome.publication_receipt.is_some());
    assert!(
        !serde_json::to_string(&outcome)
            .unwrap()
            .contains(&staging_root.to_string_lossy().to_string())
    );
}
