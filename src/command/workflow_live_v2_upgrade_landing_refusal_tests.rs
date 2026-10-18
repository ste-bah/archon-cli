use super::*;

const TWO_REFUSED: &str = r#"async function workflow(w) {
    const target = {taskId: "TASK-X-001", fileName: "TASK-X-001.md"};
    await w.hostCommand("land-task-body", {stdin: "refused round 1", target});
    await w.hostCommand("land-task-body", {stdin: "refused round 2", target});
    return "finished";
}"#;

const ROUND_ONE_THEN_TWO: &str = r#"async function workflow(w) {
    const target = {taskId: "TASK-X-001", fileName: "TASK-X-001.md"};
    await w.hostCommand("land-task-body", {stdin: "refused round 1", target});
    return await w.hostCommand("land-task-body", {stdin: "round 2", target});
}"#;

const ROUND_ONE_WITHOUT_STDIN: &str = r#"async function workflow(w) {
    const target = {taskId: "TASK-X-001", fileName: "TASK-X-001.md"};
    return await w.hostCommand("land-task-body", {target});
}"#;

fn refused_record(run: &Run, stdin: &str) -> WorkflowV2CallRecord {
    let store = WorkflowV2ResultStore::new(run.store.run_dir(&run.run_id).join("v2"));
    store
        .load_call_records()
        .unwrap()
        .into_iter()
        .find(|record| {
            record
                .call
                .options
                .host_command
                .as_ref()
                .is_some_and(|request| request.stdin.as_deref() == Some(stdin))
        })
        .expect("recorded refused candidate")
}

fn save_record(run: &Run, record: &WorkflowV2CallRecord) {
    WorkflowV2ResultStore::new(run.store.run_dir(&run.run_id).join("v2"))
        .save_call_record(record)
        .unwrap();
}

fn without_publication(record: &mut WorkflowV2CallRecord) {
    record.result.data["publicationReceipt"] = serde_json::Value::Null;
}

#[tokio::test]
async fn superseded_refusal_with_changed_logic_is_judged_again_on_recorded_candidate() {
    let run = Run::new();
    run.run(run.build(9), TWO_REFUSED).await;
    let mut refusal = refused_record(&run, "refused round 1");
    assert_eq!(refusal.result.data["logicVersion"], 9);
    without_publication(&mut refusal);
    save_record(&run, &refusal);
    let mut refusal_two = refused_record(&run, "refused round 2");
    without_publication(&mut refusal_two);
    save_record(&run, &refusal_two);

    let store = WorkflowV2ResultStore::new(run.store.run_dir(&run.run_id).join("v2"));
    let records_before = store.load_call_records().unwrap();
    let current = run.build(13);
    current.accept_refused.store(true, Ordering::SeqCst);
    let result = run.run(current.clone(), ROUND_ONE).await;

    assert_eq!(
        current.executed.lock().unwrap().as_slice(),
        &[String::from("refused round 1")]
    );
    assert!(
        result["gateEnvelope"]["policy_findings"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(result["gateEnvelope"]["report"], "refused round 1");
    let records_after = store.load_call_records().unwrap();
    assert_eq!(records_after.len(), records_before.len() + 1);
    assert!(records_after.iter().any(|record| {
        record.result.data["logicVersion"] == 13
            && record
                .call
                .options
                .host_command
                .as_ref()
                .is_some_and(|request| request.stdin.as_deref() == Some("refused round 1"))
    }));
}

#[tokio::test]
async fn later_accepted_round_blocks_rejudging_superseded_refusal() {
    let run = Run::new();
    run.run(run.build(9), ROUND_ONE_THEN_TWO).await;
    let mut refusal = refused_record(&run, "refused round 1");
    without_publication(&mut refusal);
    save_record(&run, &refusal);

    let before = WorkflowV2ResultStore::new(run.store.run_dir(&run.run_id).join("v2"))
        .load_call_records()
        .unwrap();
    let host = run.build(13);
    let result = run.run(host.clone(), ROUND_ONE).await;

    assert_eq!(
        result["gateEnvelope"]["policy_findings"][0]["text"],
        "gap in refused round 1"
    );
    assert!(host.executed.lock().unwrap().is_empty());
    let after = WorkflowV2ResultStore::new(run.store.run_dir(&run.run_id).join("v2"))
        .load_call_records()
        .unwrap();
    assert_eq!(after.len(), before.len());
    let current = archon_workflow::v2::script::landing_lineage::LandingLineage::of(
        &after,
        "land-task-body",
        "TASK-X-001",
    )
    .current()
    .unwrap();
    assert_eq!(
        current
            .call
            .options
            .host_command
            .as_ref()
            .unwrap()
            .stdin
            .as_deref(),
        Some("round 2")
    );
}

#[tokio::test]
async fn superseded_refusal_with_current_logic_replays_without_execution() {
    let run = Run::new();
    run.run(run.build(9), TWO_REFUSED).await;
    let mut refusal = refused_record(&run, "refused round 1");
    without_publication(&mut refusal);
    save_record(&run, &refusal);

    let current = run.build(9);
    let result = run.run(current.clone(), ROUND_ONE).await;

    assert_eq!(
        result["gateEnvelope"]["policy_findings"][0]["text"],
        "gap in refused round 1"
    );
    assert!(current.executed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn superseded_refusal_without_recorded_candidate_fails_loudly() {
    let run = Run::new();
    run.run(run.build(9), TWO_REFUSED).await;
    let mut refusal = refused_record(&run, "refused round 1");
    without_publication(&mut refusal);
    refusal.call.options.host_command.as_mut().unwrap().stdin = None;
    save_record(&run, &refusal);
    let mut refusal_two = refused_record(&run, "refused round 2");
    without_publication(&mut refusal_two);
    save_record(&run, &refusal_two);
    assert!(archon_workflow::v2::script::landing_lineage::landing_refused(&refusal));
    assert!(refusal.result.data["publicationReceipt"].is_null());
    assert!(!run.build(13).keys.outcome_logic_holds(&refusal).unwrap());
    assert!(
        WorkflowV2ResultStore::new(run.store.run_dir(&run.run_id).join("v2"))
            .load_call_record(&refusal.call.id)
            .unwrap()
            .unwrap()
            .call
            .options
            .host_command
            .unwrap()
            .stdin
            .is_none()
    );

    let error = run
        .run_summary_with_lease(run.build(13), ROUND_ONE, None)
        .await
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("has no recorded candidate bytes")
    );
}

#[tokio::test]
async fn missing_stdin_accepted_fallback_does_not_answer_different_candidate() {
    let run = Run::new();
    let accepting = run.build(9);
    accepting.accept_refused.store(true, Ordering::SeqCst);
    run.run(accepting, ROUND_ONE).await;

    let mut accepted = refused_record(&run, "refused round 1");
    assert!(matches!(
        accepted.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ));
    accepted.call.options.host_command.as_mut().unwrap().stdin = None;
    save_record(&run, &accepted);

    const DIFFERENT_CANDIDATE: &str = r#"async function workflow(w) {
        const target = {taskId: "TASK-X-001", fileName: "TASK-X-001.md"};
        return await w.hostCommand("land-task-body", {stdin: "different candidate", target});
    }"#;
    let current = run.build(9);
    let result = run.run(current.clone(), DIFFERENT_CANDIDATE).await;

    assert_eq!(
        current.executed.lock().unwrap().as_slice(),
        &[String::from("different candidate")]
    );
    assert_eq!(result["gateEnvelope"]["report"], "different candidate");
}

#[tokio::test]
async fn refusal_without_stdin_and_current_logic_replays_without_error() {
    let run = Run::new();
    run.run(run.build(9), TWO_REFUSED).await;
    let mut refusal = refused_record(&run, "refused round 1");
    without_publication(&mut refusal);
    refusal.call.options.host_command.as_mut().unwrap().stdin = None;
    save_record(&run, &refusal);
    let mut refusal_two = refused_record(&run, "refused round 2");
    without_publication(&mut refusal_two);
    save_record(&run, &refusal_two);

    let current = run.build(9);
    let result = run.run(current.clone(), ROUND_ONE_WITHOUT_STDIN).await;

    assert_eq!(
        result["gateEnvelope"]["policy_findings"][0]["text"],
        "gap in refused round 1"
    );
    assert!(current.executed.lock().unwrap().is_empty());
}
