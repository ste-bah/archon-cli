//! Persistence regressions for reverify findings, ledgers, and retries.
use super::workflow_decompose_reverify_tests::{archon_binary, fixture, invoke_reverify};
use archon_workflow::{
    RunStatus, WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod,
    WorkflowV2HostOptions, WorkflowV2Result, WorkflowV2ResultStore,
};

#[tokio::test]
async fn reverify_records_and_report_subjects_carry_the_judging_logic_stamp() {
    // EXPECTED BEFORE RUN: every persisted reverify host-command outcome and
    // every body subject in the report carries its capability's version,
    // digest, and build stamp from this binary's CAPABILITY_LOGIC table.
    let fixture = fixture();
    let output = invoke_reverify(&archon_binary(), &fixture);
    println!(
        "EVIDENCE logic_stamps COMMAND: success={} stdout={} stderr={}",
        output.status.success(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let v2 = WorkflowV2ResultStore::new(fixture.store.run_dir(&fixture.run_id).join("v2"));
    let records = v2.load_call_records().unwrap();
    let gates = records
        .iter()
        .filter(|record| {
            record.call.id.starts_with("reverify-")
                && record.call.method == WorkflowV2HostMethod::HostCommand
        })
        .collect::<Vec<_>>();
    assert!(!gates.is_empty());
    println!(
        "EVIDENCE logic_stamps BEFORE: reverify_host_call_records={} expected=version+digest+build",
        gates.len()
    );
    for record in &gates {
        let capability = record
            .call
            .options
            .host_command
            .as_ref()
            .unwrap()
            .command_id
            .as_str();
        let logic = crate::command::workflow_host_command_logic::CAPABILITY_LOGIC
            .iter()
            .find(|logic| logic.id == capability)
            .unwrap();
        assert_eq!(
            record.result.data["logicVersion"], logic.version,
            "{capability}"
        );
        assert_eq!(
            record.result.data["logicDigest"], logic.sources_digest,
            "{capability}"
        );
        assert_eq!(
            record.result.data["logicBuild"].as_str(),
            Some(crate::command::workflow_host_command_logic::THIS_BUILD),
            "{capability}"
        );
    }
    let latest = super::super::workflow_decompose_latest_report::latest_report(
        &fixture.store,
        &fixture.run_id,
    )
    .unwrap()
    .unwrap();
    let report = records
        .iter()
        .find(|record| record.call.id == latest.id)
        .unwrap();
    let report_path = fixture
        .store
        .run_dir(&fixture.run_id)
        .join(format!("decomposition/reverify/reports/{}.json", latest.id));
    let report_json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(report.result.data["subjects"], report_json["subjects"]);
    let body_logic = crate::command::workflow_host_command_logic::CAPABILITY_LOGIC
        .iter()
        .find(|logic| logic.id == "land-task-body")
        .unwrap();
    for subject in report_json["subjects"].as_array().unwrap() {
        assert_eq!(subject["logic_version"], body_logic.version);
        assert_eq!(subject["logic_digest"], body_logic.sources_digest);
        assert_eq!(
            subject["build"].as_str(),
            Some(crate::command::workflow_host_command_logic::THIS_BUILD)
        );
    }
    for gate in report_json["set_gates"]
        .as_array()
        .unwrap()
        .iter()
        .chain(report_json["frozen_gates"].as_array().unwrap())
    {
        let capability = gate["command_id"].as_str().unwrap();
        let logic = crate::command::workflow_host_command_logic::CAPABILITY_LOGIC
            .iter()
            .find(|logic| logic.id == capability)
            .unwrap();
        assert_eq!(gate["logic_version"], logic.version, "{capability}");
        assert_eq!(gate["logic_digest"], logic.sources_digest, "{capability}");
        assert_eq!(
            gate["build"].as_str(),
            Some(crate::command::workflow_host_command_logic::THIS_BUILD),
            "{capability}"
        );
    }
    println!(
        "EVIDENCE logic_stamps AFTER: host_call_records={} all_gate_stamps_match_CAPABILITY_LOGIC=true report_file_read=true report_subjects={} all_subject_and_gate_stamps_match_CAPABILITY_LOGIC=true",
        gates.len(),
        report_json["subjects"].as_array().unwrap().len()
    );
}

#[tokio::test]
async fn invalid_utf8_body_is_persisted_as_remediation_finding() {
    // EXPECTED BEFORE RUN: invalid UTF-8 in a frozen task body is an input
    // finding for that subject, with its decoding cause in the persisted report.
    let fixture = fixture();
    let invalid = [0xff, 0xfe, b'\n'];
    std::fs::write(&fixture.bodies[0], invalid).unwrap();
    let run_record_path = fixture.store.state_path(&fixture.run_id);
    let run_record_before = std::fs::read(&run_record_path).unwrap();
    println!("EVIDENCE invalid_utf8 BEFORE: subject=TASK-X-010 bytes=3 utf8=invalid run=Completed");
    let output = invoke_reverify(&archon_binary(), &fixture);
    println!(
        "EVIDENCE invalid_utf8 COMMAND: success={} stdout={} stderr={}",
        output.status.success(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let v2 = WorkflowV2ResultStore::new(fixture.store.run_dir(&fixture.run_id).join("v2"));
    let latest = super::super::workflow_decompose_latest_report::latest_report(
        &fixture.store,
        &fixture.run_id,
    )
    .unwrap()
    .unwrap();
    let records = v2.load_call_records().unwrap();
    let report_record = records
        .iter()
        .find(|record| record.call.id == latest.id)
        .unwrap();
    let body_record = records
        .iter()
        .find(|record| {
            record.call.id.starts_with("reverify-") && record.call.id.ends_with("-body-0")
        })
        .unwrap();
    let body_logic = crate::command::workflow_host_command_logic::CAPABILITY_LOGIC
        .iter()
        .find(|logic| logic.id == "land-task-body")
        .unwrap();
    assert_eq!(body_record.result.data["logicVersion"], body_logic.version);
    assert_eq!(
        body_record.result.data["logicDigest"],
        body_logic.sources_digest
    );
    assert_eq!(
        body_record.result.data["logicBuild"].as_str(),
        Some(crate::command::workflow_host_command_logic::THIS_BUILD)
    );
    let subject = &report_record.result.data["subjects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|subject| subject["task_id"] == "TASK-X-010")
        .unwrap();
    assert_eq!(subject["verdict"], "needs_remediation");
    assert!(
        subject["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding.as_str().unwrap().contains("UTF-8"))
    );
    let report_path = fixture
        .store
        .run_dir(&fixture.run_id)
        .join(format!("decomposition/reverify/reports/{}.json", latest.id));
    let report_json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
    let report_subject = report_json["subjects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|subject| subject["task_id"] == "TASK-X-010")
        .unwrap();
    assert_eq!(report_subject["logic_version"], body_logic.version);
    assert_eq!(report_subject["logic_digest"], body_logic.sources_digest);
    assert_eq!(
        report_subject["build"].as_str(),
        Some(crate::command::workflow_host_command_logic::THIS_BUILD)
    );
    let ledger_path = fixture.store.run_dir(&fixture.run_id).join(format!(
        "decomposition/reverify/remediation-{}.json",
        latest.id
    ));
    assert!(
        !ledger_path.exists(),
        "operational failure must not write a ledger"
    );
    assert_eq!(std::fs::read(&run_record_path).unwrap(), run_record_before);
    assert_eq!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        RunStatus::Completed
    );
    assert_eq!(std::fs::read(&fixture.bodies[0]).unwrap(), invalid);
    println!(
        "EVIDENCE invalid_utf8 AFTER: report={} verdict=needs_remediation cause_present=true body_unchanged=true call_and_report_stamps_match_land_task_body=true run_record_bytes_unchanged=true ledger_present=false exit_success={}",
        latest.id,
        output.status.success()
    );
}

#[tokio::test]
async fn each_reverify_uses_a_distinct_ledger_and_preserves_the_first() {
    // EXPECTED BEFORE RUN: every report has a report-specific ledger path;
    // a later finding cannot change bytes stored for an earlier report.
    let fixture = fixture();
    std::fs::write(&fixture.bodies[0], b"not a task body\n").unwrap();
    let binary = archon_binary();
    let first = invoke_reverify(&binary, &fixture);
    assert!(
        fixture
            .store
            .run_dir(&fixture.run_id)
            .join("decomposition/reverify/reports")
            .is_dir(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first_report = super::super::workflow_decompose_latest_report::latest_report(
        &fixture.store,
        &fixture.run_id,
    )
    .unwrap()
    .unwrap();
    let first_ledger = fixture.store.run_dir(&fixture.run_id).join(format!(
        "decomposition/reverify/remediation-{}.json",
        first_report.id
    ));
    let first_bytes = std::fs::read(&first_ledger).unwrap();
    fixture
        .store
        .with_run_lock(&fixture.run_id, |store| {
            let mut run = store.load_state(&fixture.run_id)?;
            run.status = RunStatus::Completed;
            store.save_state(&run)
        })
        .unwrap();
    println!(
        "EVIDENCE ledger_per_report BEFORE: first_report={} first_ledger_bytes={} second_run=ready",
        first_report.id,
        first_bytes.len()
    );
    let second = invoke_reverify(&binary, &fixture);
    assert!(
        fixture
            .store
            .run_dir(&fixture.run_id)
            .join("decomposition/reverify/reports")
            .is_dir(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_report = super::super::workflow_decompose_latest_report::latest_report(
        &fixture.store,
        &fixture.run_id,
    )
    .unwrap()
    .unwrap();
    let second_ledger = fixture.store.run_dir(&fixture.run_id).join(format!(
        "decomposition/reverify/remediation-{}.json",
        second_report.id
    ));
    assert_ne!(first_ledger, second_ledger);
    assert_eq!(std::fs::read(&first_ledger).unwrap(), first_bytes);
    assert!(second_ledger.is_file());
    println!(
        "EVIDENCE ledger_per_report AFTER: first_report={} first_bytes_unchanged=true second_report={} second_ledger_present=true",
        first_report.id, second_report.id
    );
}

#[tokio::test]
async fn partial_invocation_records_are_never_replaced_on_retry() {
    // EXPECTED BEFORE RUN: a persisted body-call record with no completion
    // event reserves its invocation identity; retry must leave its bytes intact.
    let fixture = fixture();
    let seq = fixture.store.next_event_seq(&fixture.run_id).unwrap();
    archon_workflow::WorkflowEventLog::new(fixture.store.clone())
        .emit(
            &fixture.run_id,
            seq,
            archon_workflow::WorkflowEventKind::DecompositionReverifyStarted,
            serde_json::json!({"invocation_id": format!("reverify-{seq}")}),
        )
        .unwrap();
    let v2 = WorkflowV2ResultStore::new(fixture.store.run_dir(&fixture.run_id).join("v2"));
    let id = format!("reverify-{seq}-body-0");
    let record = WorkflowV2CallRecord::new(
        &fixture.run_id,
        WorkflowV2HostCall {
            id: id.clone(),
            method: WorkflowV2HostMethod::HostCommand,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        1,
        "partial-invocation-record".into(),
        WorkflowV2Result::default(),
        Vec::new(),
    );
    v2.save_call_record(&record).unwrap();
    let path = v2.result_path(&id);
    let before = std::fs::read(&path).unwrap();
    println!(
        "EVIDENCE partial_invocation BEFORE: next_seq={} existing_call={} completion_event=absent bytes={}",
        seq,
        id,
        before.len()
    );
    let output = invoke_reverify(&archon_binary(), &fixture);
    assert!(
        std::fs::read(&path).unwrap() == before,
        "partial invocation record changed"
    );
    assert!(
        fixture
            .store
            .run_dir(&fixture.run_id)
            .join("decomposition/reverify/reports")
            .is_dir(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let latest = super::super::workflow_decompose_latest_report::latest_report(
        &fixture.store,
        &fixture.run_id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(latest.id, format!("reverify-{}-report", seq + 1));
    println!(
        "EVIDENCE partial_invocation AFTER: existing_call={} bytes_unchanged=true new_report={} command_success={}",
        id,
        latest.id,
        output.status.success()
    );
}
