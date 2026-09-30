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
