use super::*;

#[test]
fn ordinary_resume_keeps_body_prompts_unchanged_after_reverify() {
    use crate::command::workflow::workflow_decompose_reverify_tests::{
        archon_binary, fixture, invoke_reverify,
    };
    let fixture = fixture();
    std::fs::write(&fixture.bodies[0], "not a task body\n").unwrap();
    assert!(invoke_reverify(&archon_binary(), &fixture).status.success());
    let run_dir = fixture.store.run_dir(&fixture.run_id);
    let state: archon_workflow::FixedDecompositionStateV1 =
        serde_json::from_slice(&std::fs::read(run_dir.join("decomposition/state.json")).unwrap())
            .unwrap();
    let mut plain = serde_json::json!({"phaseSeed": {}});
    let mut healing = plain.clone();
    for (arguments, remediate) in [(&mut plain, false), (&mut healing, true)] {
        let plan = crate::command::workflow_decompose_remediation::prepare_with_feedback(
            &fixture.store,
            &fixture.run_id,
            &run_dir,
            arguments,
            &state.identity,
            &Default::default(),
            &Default::default(),
            remediate,
        )
        .unwrap()
        .unwrap();
        assert!(plan.pending.is_some());
    }
    assert!(plain.get("remediationBodyFindings").is_none());
    assert!(
        !healing["remediationBodyFindings"]["TASK-X-010"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    healing
        .as_object_mut()
        .unwrap()
        .remove("remediationBodyFindings");
    assert_eq!(plain, healing);
}

fn report() -> ReverifyReport {
    serde_json::from_value(serde_json::json!({
        "schema_version": 1, "id": "reverify-1-report", "supersedes_id": "final",
        "supersedes_digest": "old", "outcome": "needs_remediation", "calls": [],
        "subjects": [{"task_id": "TASK-X-010", "file_name": "TASK-X-010.md",
            "disk_digest": "body", "judged_digest": "body", "logic_version": null,
            "logic_digest": null, "build": null, "verdict": "needs_remediation",
            "findings": ["invalid at /tmp/round-1 after 12ms"]}],
        "set_gates": [], "frozen_gates": [], "checks": []
    }))
    .unwrap()
}

#[test]
fn reverify_round_decisions_cover_accept_repair_stall_oscillation_failure_and_control() {
    let a = BTreeSet::from(["body:A".into()]);
    let b = BTreeSet::from(["body:B".into()]);
    assert_eq!(decide("accepted", false, &a, &[]), Decision::Accepted);
    assert_eq!(
        decide("needs_remediation", false, &a, &[]),
        Decision::Repair
    );
    assert_eq!(
        decide("needs_remediation", false, &b, std::slice::from_ref(&a)),
        Decision::Repair
    );
    assert_eq!(
        decide("needs_remediation", false, &a, std::slice::from_ref(&a)),
        Decision::NoProgress
    );
    assert_eq!(
        decide("needs_remediation", false, &a, &[a.clone(), b]),
        Decision::Oscillation
    );
    assert_eq!(
        decide("operational_failure", false, &a, &[]),
        Decision::OperationalFailure
    );
    for outcome in ["accepted", "needs_remediation", "operational_failure"] {
        assert_eq!(decide(outcome, true, &a, &[]), Decision::Controlled);
    }
}

#[test]
fn reverify_round_invariants_ignore_volatile_diagnostics_but_keep_subject_and_gate() {
    let first = report();
    let mut second = first.clone();
    second.id = "reverify-200-report".into();
    second.subjects[0].disk_digest = "new-body".into();
    second.subjects[0].findings = vec!["invalid at /tmp/round-200 after 900ms id=xyz".into()];
    assert_ne!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
    assert_eq!(invariants(&first), invariants(&second));
    assert_eq!(
        decide(
            "needs_remediation",
            false,
            &invariants(&second),
            &[invariants(&first)]
        ),
        Decision::NoProgress
    );
    second.subjects[0].task_id = "TASK-X-020".into();
    assert_ne!(invariants(&first), invariants(&second));
}

#[test]
fn reverify_round_healing_summary_has_no_manual_resume_and_keeps_findings() {
    let report = report();
    assert!(!summary(&report, "run", false).contains("Resume:"));
    assert!(summary(&report, "run", true).contains("Resume: archon workflow resume"));
    assert!(
        serde_json::to_string(&report)
            .unwrap()
            .contains("invalid at /tmp/round-1 after 12ms")
    );
}

#[tokio::test]
async fn reverify_remediate_records_operational_stop_without_starting_authors() {
    use crate::command::workflow::workflow_decompose_reverify_tests::{archon_binary, fixture};
    let fixture = fixture();
    let output = std::process::Command::new(archon_binary())
        .args(["workflow", "reverify", &fixture.run_id, "--remediate"])
        .current_dir(&fixture.project)
        .env("ARCHON_LOG_DIR", fixture.project.join(".test-logs"))
        .env("XDG_DATA_HOME", fixture.project.join(".test-data"))
        .env("HOME", &fixture.project)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("operational_failure"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Resume:"));
    let (events, _) =
        archon_workflow::events::read_event_log(&fixture.store.events_path(&fixture.run_id))
            .unwrap();
    assert!(events.iter().any(
        |event| event.kind == WorkflowEventKind::DecompositionReverifyRound
            && event.detail["decision"] == "ObservationStopped"
    ));
    assert!(
        !events
            .iter()
            .any(|event| event.kind == WorkflowEventKind::Resumed
                || event.kind == WorkflowEventKind::AuthorAttemptStarted)
    );
    assert_eq!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        RunStatus::Completed
    );
}

#[tokio::test]
async fn reverify_remediate_stops_for_operator_pause_and_cancel() {
    use crate::command::workflow::workflow_decompose_reverify_tests::fixture;
    for action in [
        archon_workflow::LifecycleAction::Pause,
        archon_workflow::LifecycleAction::Cancel,
    ] {
        let fixture = fixture();
        let expected = if matches!(action, archon_workflow::LifecycleAction::Pause) {
            RunStatus::Paused
        } else {
            RunStatus::Cancelled
        };
        let store = fixture.store.clone();
        let id = fixture.run_id.clone();
        super::super::persist::set_after_persist_hook(move || {
            archon_workflow::LifecycleController::new(store)
                .apply(&id, action)
                .unwrap();
            false
        });
        let result = remediate(
            &fixture.project,
            &fixture.run_id,
            &ArchonConfig::default(),
            &archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new()),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(
            fixture.store.load_state(&fixture.run_id).unwrap().status,
            expected
        );
        let (events, _) =
            archon_workflow::events::read_event_log(&fixture.store.events_path(&fixture.run_id))
                .unwrap();
        assert!(
            !events
                .iter()
                .any(|event| event.kind == WorkflowEventKind::Resumed)
        );
        assert!(
            !events
                .iter()
                .any(|event| event.kind == WorkflowEventKind::DecompositionReverifyRound),
            "the old generation must not append after operator control"
        );
        assert!(events.iter().any(|event| matches!(
            event.kind,
            WorkflowEventKind::Paused | WorkflowEventKind::Cancelled
        )));
    }
}

#[test]
fn reverify_remediate_resume_handoff_fences_pause_cancel_and_competing_generation() {
    use crate::command::workflow::workflow_decompose_reverify_tests::fixture;
    let fixture = fixture();
    let mut run = fixture.store.load_state(&fixture.run_id).unwrap();
    run.status = RunStatus::Paused;
    fixture.store.save_state(&run).unwrap();
    let expected = (run.generation, run.status.clone());
    assert!(check_control(&run, Some(&expected)).is_ok());
    for status in [
        RunStatus::Paused,
        RunStatus::Cancelled,
        RunStatus::Completed,
    ] {
        run.generation = expected.0 + 1;
        run.status = status;
        assert!(check_control(&run, Some(&expected)).is_err());
    }
}

#[test]
fn reverify_round_fingerprint_matches_repeated_real_reports() {
    use crate::command::workflow::workflow_decompose_reverify_tests::{
        archon_binary, fixture, invoke_reverify,
    };
    let fixture = fixture();
    std::fs::write(&fixture.bodies[0], "not a task body\n").unwrap();
    let binary = archon_binary();
    let mut reports = Vec::new();
    for _ in 0..2 {
        let output = invoke_reverify(&binary, &fixture);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let latest = crate::command::workflow_decompose_latest_report::latest_report(
            &fixture.store,
            &fixture.run_id,
        )
        .unwrap()
        .unwrap();
        let path = fixture.store.run_dir(&fixture.run_id).join(format!(
            "{}/{}.json",
            types::REPORT_PATH,
            latest.id
        ));
        reports
            .push(serde_json::from_slice::<ReverifyReport>(&std::fs::read(path).unwrap()).unwrap());
    }
    assert_ne!(reports[0].id, reports[1].id);
    assert_ne!(reports[0].calls, reports[1].calls);
    assert_eq!(reports[0].outcome, "needs_remediation");
    let records = WorkflowV2ResultStore::new(fixture.store.run_dir(&fixture.run_id).join("v2"))
        .load_call_records()
        .unwrap();
    assert_eq!(
        call_invariants(&reports[0], &records).unwrap(),
        call_invariants(&reports[1], &records).unwrap()
    );
    let findings = |report: &ReverifyReport| {
        report
            .subjects
            .iter()
            .flat_map(|subject| subject.findings.clone())
            .chain(
                report
                    .set_gates
                    .iter()
                    .flat_map(|gate| gate.findings.clone()),
            )
            .collect::<Vec<_>>()
    };
    assert!(!findings(&reports[0]).is_empty());
    println!(
        "EVIDENCE real repeated reverify: report_ids_differ=true call_ids_differ=true finding_text_stable={} failing_invariants_stable=true",
        findings(&reports[0]) == findings(&reports[1])
    );
    assert_eq!(
        decide(
            &reports[1].outcome,
            false,
            &invariants(&reports[1]),
            &[invariants(&reports[0])]
        ),
        Decision::NoProgress
    );
}

#[test]
fn reverify_round_tracks_validator_defects_without_hashing_diagnostics() {
    let mut report = report();
    report.calls = vec!["gate-1".into()];
    let mut record = WorkflowV2CallRecord::new(
        "run",
        WorkflowV2HostCall {
            id: "gate-1".into(),
            method: WorkflowV2HostMethod::HostCommand,
            write_mode: None,
            options: WorkflowV2HostOptions {
                host_command: Some(HostCommandRequest::new("land-task-body", None).unwrap()),
                ..Default::default()
            },
        },
        1,
        "input".into(),
        WorkflowV2Result {
            data: serde_json::json!({
                "exitCode": 1, "stdout": "", "stderr": "", "stdoutBytes": 0, "stderrBytes": 0,
                "timedOut": false, "interrupted": false, "stdoutTruncated": false, "stderrTruncated": false,
                "gateEnvelope": {"schema_version": 1, "report": null, "policy_findings": [{
                    "subject": "TASK-X-010", "text": "missing heading at /tmp/1 after 1ms",
                    "source_path": "/tmp/1", "remediation_scope": "body",
                    "deterministic_defect": archon_workflow::defect::DeterministicDefect::new("section_heading", "TASK-X-010", "focused_tests")
                }]}
            }),
            ..Default::default()
        },
        Vec::new(),
    );
    let first = call_invariants(&report, &[record.clone()]).unwrap();
    let finding = &mut record.result.data["gateEnvelope"]["policy_findings"][0];
    finding["text"] = "missing heading at /tmp/99 after 900ms".into();
    finding["source_path"] = "/tmp/99".into();
    assert_eq!(first, call_invariants(&report, &[record.clone()]).unwrap());
    record.result.data["gateEnvelope"]["policy_findings"][0]["deterministic_defect"]["code"] =
        "missing_runnable_test".into();
    let repaired = call_invariants(&report, &[record]).unwrap();
    assert_ne!(first, repaired);
    assert_eq!(
        decide("needs_remediation", false, &repaired, &[first]),
        Decision::Repair
    );
    assert!(
        call_invariants(&report, &[]).is_err(),
        "missing durable evidence must not silently change a fingerprint"
    );
}

#[test]
fn reverify_round_audit_is_observational_only_when_linked_to_the_report() {
    use crate::command::workflow::workflow_decompose_reverify_tests::{
        archon_binary, fixture, invoke_reverify,
    };
    let fixture = fixture();
    std::fs::write(&fixture.bodies[0], "not a task body\n").unwrap();
    let binary = archon_binary();
    assert!(invoke_reverify(&binary, &fixture).status.success());
    let latest = crate::command::workflow_decompose_latest_report::latest_report(
        &fixture.store,
        &fixture.run_id,
    )
    .unwrap()
    .unwrap();
    let emit = |report_id: &str| {
        WorkflowEventLog::new(fixture.store.clone())
            .emit(
                &fixture.run_id,
                fixture.store.next_event_seq(&fixture.run_id).unwrap(),
                WorkflowEventKind::DecompositionReverifyRound,
                serde_json::json!({"report_id": report_id, "decision": "NoProgress"}),
            )
            .unwrap();
    };
    emit(&latest.id);
    let retry = invoke_reverify(&binary, &fixture);
    assert!(
        retry.status.success(),
        "{}",
        String::from_utf8_lossy(&retry.stderr)
    );
    emit("unrelated-report");
    let refused = invoke_reverify(&binary, &fixture);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("remediation already started"));
}

#[cfg(unix)]
#[test]
fn observation_error_survives_real_report_and_event_failures() {
    use crate::command::workflow::workflow_decompose_reverify_tests::fixture;
    use std::os::unix::fs::PermissionsExt;
    for fail_report in [true, false] {
        let fixture = fixture();
        let state = fixture.store.load_state(&fixture.run_id).unwrap();
        let path = if fail_report {
            fixture.prior_record.clone()
        } else {
            fixture.store.events_path(&fixture.run_id)
        };
        if fail_report {
            let mut record: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            record["result"] = serde_json::Value::Null;
            std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
        } else {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
        }
        let original = anyhow!("original observation failure");
        let audit = record_observation_stop(
            &fixture.store,
            &fixture.run_id,
            &(state.generation, state.status),
            1,
            &original,
        );
        assert!(audit.is_err(), "expected a real storage failure");
        let audit_text = format!("{:#}", audit.as_ref().unwrap_err());
        let text = format!("{:#}", observation_error(original, audit));
        assert!(text.contains("original observation failure"), "{text}");
        assert!(text.contains(&audit_text), "{text}");
        if !fail_report {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }
}
