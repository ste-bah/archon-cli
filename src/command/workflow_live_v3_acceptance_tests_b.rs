//! Every acceptance round covers the whole frozen contract.

use super::*;

/// Every round runs the WHOLE contract, whatever the script asks for, so a
/// check that passed earlier and regressed under a later fix cannot hide;
/// asking for a check outside the frozen contract is an operational error.
#[tokio::test]
async fn every_round_reruns_the_whole_contract_so_a_regression_cannot_hide() {
    let fixture = fixture(true);
    let first = run(&fixture, &execution(1, 3, &[])).await.unwrap();
    assert_eq!(failing_ids(&first), vec!["REQ-2", "REQ-9"]);
    // A remediation for REQ-2 breaks REQ-1, which passed in round 1.
    std::fs::remove_file(fixture.repo.path().join("present")).unwrap();
    let result = run(&fixture, &execution(2, 3, &["REQ-2"])).await.unwrap();
    let (record, _) = latest_round_record(&fixture.store.run_dir(&fixture.run_id))
        .unwrap()
        .unwrap();
    assert_eq!(record.round, 2);
    assert_eq!(record.requested_check_ids, vec!["REQ-2"]);
    assert_eq!(
        record
            .checks
            .iter()
            .map(|c| c.check_id.as_str())
            .collect::<Vec<_>>(),
        vec!["REQ-1", "REQ-2", "REQ-9"]
    );
    assert_eq!(failing_ids(&result), vec!["REQ-1", "REQ-2", "REQ-9"]);
    let bogus = run(&fixture, &execution(3, 3, &["REQ-404"])).await.unwrap();
    // Blocking, but not yet final: the loop ends on no progress (A2).
    assert!(bogus.summary.contains("REQ-404"), "{}", bogus.summary);
    let (record, _) = latest_round_record(&fixture.store.run_dir(&fixture.run_id))
        .unwrap()
        .unwrap();
    assert!(record.blocks_completion());
}

/// Batch E: a failing check whose output points into a file another task
/// owns names that task as a writer on the round record and in the reply.
#[tokio::test]
async fn a_failure_in_another_tasks_file_names_that_task_as_a_writer() {
    // The check fails with a panic at the line `grep` finds: its output
    // names the file with a line location the check itself never spells.
    // (Batch E2: only a failure signal implicates; a bare grep listing or a
    // warning does not.)
    let mut fixture = fixture_with(
        true,
        "n=$(grep -n needle src/engine.rs | cut -d: -f1); \
         echo \"thread 'main' panicked at src/engine.rs:$n:1:\" >&2; exit 1",
    );
    let engine = fixture.repo.path().join("src/engine.rs");
    std::fs::create_dir_all(engine.parent().unwrap()).unwrap();
    std::fs::write(&engine, "fn a() {}\nfn b() {}\n// needle\n").unwrap();
    fixture.universe.tasks[0].files_expected_to_change = vec!["src/engine.rs".into()];
    let result = run(&fixture, &execution(1, 3, &[])).await.unwrap();
    let (record, _) = latest_round_record(&fixture.store.run_dir(&fixture.run_id))
        .unwrap()
        .unwrap();
    let check = record
        .checks
        .iter()
        .find(|c| c.check_id == "REQ-2")
        .unwrap();
    let routing = check
        .routing
        .as_ref()
        .unwrap_or_else(|| panic!("{check:#?}"));
    assert_eq!(routing.implicated_files, ["src/engine.rs"]);
    assert_eq!(routing.writer_tasks, ["TASK-F-001"]);
    let failing = result.data["failing"].as_array().unwrap();
    let reply = failing.iter().find(|f| f["check_id"] == "REQ-2").unwrap();
    assert_eq!(
        reply["routing"]["writer_tasks"],
        serde_json::json!(["TASK-F-001"])
    );
    assert!(
        result
            .residual_gaps
            .iter()
            .any(|g| g.description.contains("routed also to TASK-F-001"))
    );
}

/// Batch I: the reply's evidence for a failing check keeps every failure
/// line and the end of each stream: 300 warning lines then `Error: X` on
/// stderr, and a failure mid-stdout followed by many lines.
#[tokio::test]
async fn a_failing_checks_reply_keeps_its_failure_lines_and_its_end() {
    let fixture = fixture_with(
        true,
        "i=0; while [ $i -lt 300 ]; do echo \"warning: unused variable v$i in src/m$i.rs\" >&2; i=$((i+1)); done; \
         echo 'Error: X' >&2; echo 'AssertionError: middle Y'; \
         i=0; while [ $i -lt 400 ]; do echo \"teardown step $i completed\"; i=$((i+1)); done; exit 1",
    );
    let result = run(&fixture, &execution(1, 3, &[])).await.unwrap();
    let failing = result.data["failing"].as_array().unwrap();
    let reply = failing.iter().find(|f| f["check_id"] == "REQ-2").unwrap();
    let (stderr, stdout) = (
        reply["stderr_tail"].as_str().unwrap(),
        reply["stdout_tail"].as_str().unwrap(),
    );
    assert!(stderr.ends_with("Error: X"), "{stderr}");
    assert!(stdout.contains("AssertionError: middle Y"), "{stdout}");
    assert!(stdout.ends_with("teardown step 399 completed"), "{stdout}");
    assert!(stderr.len() <= 4000 && stdout.len() <= 4000);
    // Batch I2: the reply names the contract the harness ran, by its
    // absolute path, and the check's exact command.
    let contract = std::fs::canonicalize(
        fixture
            .project
            .path()
            .join("tasks/set/acceptance-contract.json"),
    )
    .map(archon_shell::paths::plain)
    .unwrap();
    assert_eq!(result.data["contract_path"], contract.display().to_string());
    let frozen = reply["frozen_check"].as_str().unwrap();
    assert!(
        frozen.contains(&format!(
            "The only authoritative acceptance contract is {}",
            contract.display()
        )),
        "{frozen}"
    );
    assert!(
        frozen.contains("any other copy or draft of the acceptance contract"),
        "{frozen}"
    );
    assert!(frozen.ends_with("i=$((i+1)); done; exit 1"), "{frozen}");
}

#[test]
fn the_scratch_guardian_narrows_an_observation_to_the_requested_checks() {
    use crate::command::acceptance_scratch_guardian::{
        Request, parse_request_line, request_line, validate_selected,
    };
    let fixture = fixture(true);
    let scratch = tempfile::tempdir().unwrap();
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(
        fixture.project.path(),
        &fixture.task_root,
    );
    let request = Request {
        policy: ScratchPolicy {
            repository: fixture.repo.path().canonicalize().map(plain).unwrap(),
            project: fixture.project.path().canonicalize().map(plain).unwrap(),
            task_root: fixture.task_root.canonicalize().map(plain).unwrap(),
            scratch_parent: scratch.path().to_path_buf(),
            project_inputs: vec![],
            project_input_excludes: vec![],
            combined: false,
            toolchain_path: "/usr/bin:/bin".into(),
            environment: Default::default(),
            environment_allowlist: vec![],
            cargo_seed: None,
            timeout_secs: 60,
            output_bytes: 2048,
            scratch_bytes: 16_777_216,
            build_cache: None,
        },
        source_commit: "0".repeat(40),
        pin_path: pin_path.clone(),
        expected_pin_digest: content_digest(&std::fs::read(&pin_path).unwrap()),
        evidence: scratch.path().join("evidence"),
        resource_limits: Some(Default::default()),
    };
    let ids = |refs: &[archon_workflow::acceptance_world::FrozenCommandRef]| {
        refs.iter()
            .map(|r| r.acceptance_id.clone())
            .collect::<Vec<_>>()
    };
    let (_, _, all) = validate_selected(&request, &None).expect("whole chain");
    assert_eq!(ids(&all), vec!["REQ-1", "REQ-2", "REQ-9"]);
    let selection = Some(std::collections::BTreeSet::from(["REQ-2".to_string()]));
    let (_, _, narrowed) = validate_selected(&request, &selection).expect("narrowed");
    assert_eq!(ids(&narrowed), vec!["REQ-2"]);
    let error = validate_selected(
        &request,
        &Some(std::collections::BTreeSet::from(["REQ-404".to_string()])),
    )
    .expect_err("an id outside the chain is refused");
    assert!(
        error.to_string().contains("not in the pinned contract"),
        "{error}"
    );
    let line = request_line(&request, &selection).unwrap();
    let (parsed, carried) = parse_request_line(std::str::from_utf8(&line).unwrap()).unwrap();
    assert_eq!(carried, selection);
    assert_eq!(parsed.evidence, request.evidence);
    let plain = request_line(&request, &None).unwrap();
    let (parsed, carried) = parse_request_line(std::str::from_utf8(&plain).unwrap()).unwrap();
    assert_eq!(carried, None);
    assert_eq!(parsed.pin_path, request.pin_path);
    let mut old_request: serde_json::Value = serde_json::from_slice(&plain).unwrap();
    old_request
        .as_object_mut()
        .unwrap()
        .remove("resource_limits");
    let error = parse_request_line(&old_request.to_string())
        .err()
        .expect("an old request without resource limits was accepted");
    assert!(
        error
            .to_string()
            .contains(&request.pin_path.display().to_string())
    );
    assert!(error.to_string().contains("missing resource_limits"));
}
