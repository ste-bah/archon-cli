use super::*;

fn slot(command: &str, attempt: u32, input_hash: &str) -> WorkflowV2CallRecord {
    serde_json::from_value(serde_json::json!({
        "call": {
            "id": "stable-call-slot",
            "method": "hostCommand",
            "options": { "host_command": { "commandId": command } }
        },
        "attempt": attempt,
        "input_hash": input_hash,
        "status": "accepted",
        "result": {
            "status": "accepted",
            "data": { "logicBuild": format!("build-{attempt}") }
        }
    }))
    .expect("call slot")
}

#[test]
fn persisted_pause_credit_survives_build_bound_slot_reexecution() {
    let temp = tempfile::tempdir().expect("temporary store");
    let store = archon_workflow::WorkflowStore::new(temp.path().join("runs"));
    let spec = archon_workflow::WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
        name: "test".into(),
        task: "test".into(),
        target_repository_root: None,
        max_parallelism: 1,
        max_agents: 1,
        stages: Vec::new(),
        permissions: Default::default(),
        learning_hooks: Vec::new(),
    };
    let run = store.create_run(spec).expect("run");
    let v2_store = archon_workflow::WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let command = build_bound_commands()[0];
    let mut original = slot(command, 1, "build-one-input");
    original.call.id = "old-build-slot".into();
    v2_store.save_call_record(&original).expect("original slot");
    let pause_id = "pause-body-TASK-X-001-1";
    let record_path = super::super::workflow_live_v2_script_host_pause::pause_record_path(pause_id);
    let pause = ScriptPauseRecord {
        pause_id: pause_id.into(),
        joined: false,
        event_seq: Some(1),
        generation: 1,
        covered: covered_attempts(&[original.clone()]),
        host_taken: false,
        judged_at_pause: Default::default(),
        unrecorded_event: None,
        voided_by: None,
        previous_record: None,
    };
    let mut legacy_pause = serde_json::to_value(&pause).expect("pause JSON");
    legacy_pause["covered"][0]
        .as_object_mut()
        .expect("covered attempt")
        .remove("build_bound");
    store
        .write_run_json(&run.id, &record_path, &legacy_pause)
        .expect("pause evidence");
    let history_dir = v2_store.call_history_dir(&original.call.id);
    std::fs::create_dir_all(&history_dir).expect("history directory");
    std::fs::write(
        history_dir.join("record.json"),
        serde_json::to_vec(&original).expect("historical call record"),
    )
    .expect("historical call record");
    std::fs::remove_file(v2_store.result_path(&original.call.id)).expect("replace old slot");
    let mut reexecuted = slot(command, 2, "build-two-input");
    reexecuted.call.id = "new-build-slot".into();
    v2_store
        .save_call_record(&reexecuted)
        .expect("new build slot");

    let (credit, voided_by, previous) = super::super::WorkflowScriptHost::pause_credit(
        &store,
        &v2_store,
        &run.id,
        &record_path,
        pause_id,
    )
    .expect("credit check");

    assert!(credit.is_some(), "resume must pass the recorded pause");
    assert!(voided_by.is_none());
    assert!(previous.is_none());
}

fn build_bound_commands() -> Vec<&'static str> {
    crate::command::workflow_host_command_logic::CAPABILITY_LOGIC
        .iter()
        .filter(|logic| logic.build_bound)
        .map(|logic| logic.id)
        .collect()
}

#[test]
fn build_rederived_verification_does_not_void_pause_credit() {
    for command in build_bound_commands() {
        let original = slot(command, 1, "build-one-input");
        let replaced = slot(command, 2, "build-two-input");
        let pause: ScriptPauseRecord = serde_json::from_value(serde_json::json!({
            "pause_id": "pause-body-TASK-X-001-1",
            "joined": false,
            "event_seq": 1,
            "generation": 1,
            "covered": covered_attempts(&[original]),
            "host_taken": false
        }))
        .expect("pause record");

        assert!(
            credit_holds(&pause, &[replaced]),
            "re-execution of typed build-bound slot {command} must preserve pause credit"
        );
    }
}

#[test]
fn changed_authored_input_voids_credit_and_names_the_changed_slot() {
    let original = slot("authored-capability", 1, "authored-input-old");
    let changed = slot("authored-capability", 2, "authored-input-new");
    let pause: ScriptPauseRecord = serde_json::from_value(serde_json::json!({
        "pause_id": "pause-body-TASK-X-001-1",
        "joined": false,
        "event_seq": 1,
        "generation": 1,
        "covered": covered_attempts(&[original]),
        "host_taken": false
    }))
    .expect("pause record");

    assert_eq!(
        credit_void(&pause, &[changed]),
        Some(PauseCreditVoid {
            call_id: "stable-call-slot".into(),
            attempt: 1,
            old_input_hash: "authored-input-old".into(),
            new_input_hash: Some("authored-input-new".into()),
        })
    );
}

#[test]
fn retake_evidence_keeps_the_prior_pause_record_intact() {
    let original = slot("authored-capability", 1, "authored-input-old");
    let changed = slot("authored-capability", 2, "authored-input-new");
    let prior = serde_json::json!({
        "pause_id": "pause-body-TASK-X-001-1",
        "joined": false,
        "event_seq": 17,
        "generation": 1,
        "evidence": { "reason": "no_progress" },
        "covered": covered_attempts(&[original]),
        "host_taken": false
    });
    let record: ScriptPauseRecord =
        serde_json::from_value(prior.clone()).expect("prior pause record");

    let (voided_by, retained) =
        retake_evidence(&record, &[changed], &serde_json::to_vec(&prior).unwrap())
            .expect("valid record")
            .expect("the changed authored slot voids credit");

    assert_eq!(voided_by.call_id, "stable-call-slot");
    assert_eq!(voided_by.attempt, 1);
    assert_eq!(voided_by.old_input_hash, "authored-input-old");
    assert_eq!(
        voided_by.new_input_hash.as_deref(),
        Some("authored-input-new")
    );
    assert_eq!(
        retained, prior,
        "the earlier event, evidence, and coverage remain available"
    );
}
