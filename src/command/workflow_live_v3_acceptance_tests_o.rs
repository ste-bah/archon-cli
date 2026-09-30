//! Batch O: nothing passes without a frozen contract whose checks ran (A8),
//! a pass that ran no test is a failure (A3), and the contract is held to
//! the PRD as it is now (A7).
use super::*;

/// A8: nothing passes without a frozen contract whose checks ran -- an
/// unfrozen, a lost and a never-declared contract all block completion.
#[tokio::test]
async fn an_unfrozen_lost_or_undeclared_contract_never_passes() {
    let unfrozen = fixture(false);
    let result = run(&unfrozen, &execution(1, 3, &[])).await.unwrap();
    assert!(result.summary.contains("not frozen"), "{}", result.summary);
    let (record, _) = latest_round_record(&unfrozen.store.run_dir(&unfrozen.run_id))
        .unwrap()
        .unwrap();
    assert!(record.blocks_completion());
    // The same error again is no progress: the loop ends, still blocked.
    run(&unfrozen, &execution(2, 3, &[])).await.unwrap();
    let result = run(&unfrozen, &execution(3, 3, &[])).await.unwrap();
    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert_eq!(result.data["final"], true);

    // Tasks that name the checks they implement declare a contract: losing
    // it cannot pass vacuously.
    let lost = fixture(false);
    std::fs::remove_file(lost.task_root.join(ACCEPTANCE_CONTRACT_FILE)).unwrap();
    let result = run(&lost, &execution(1, 3, &[])).await.unwrap();
    assert!(
        result.summary.contains("declares an acceptance contract"),
        "{}",
        result.summary
    );

    // A task set that never declared one does not pass either.
    let mut absent = fixture(false);
    std::fs::remove_file(absent.task_root.join(ACCEPTANCE_CONTRACT_FILE)).unwrap();
    for task in &mut absent.universe.tasks {
        task.implements.clear();
    }
    let result = run(&absent, &execution(1, 3, &[])).await.unwrap();
    assert_eq!(result.data["contract_present"], false);
    assert!(
        result
            .summary
            .contains("an authored run completes only on a frozen acceptance contract"),
        "{}",
        result.summary
    );
    let (record, _) = latest_round_record(&absent.store.run_dir(&absent.run_id))
        .unwrap()
        .unwrap();
    assert!(record.blocks_completion());
}

/// A3: exit 0 from a run that reports it ran no test is a failure, routed
/// to the check's owners like any other.
#[tokio::test]
async fn a_pass_that_ran_no_test_is_a_failure() {
    let fixture = fixture_with(true, "test -f present && printf 'running 0 tests\\n'");
    let result = run(&fixture, &execution(1, 3, &[])).await.unwrap();
    assert_eq!(failing_ids(&result), vec!["REQ-2", "REQ-9"]);
    let failing = result.data["failing"].as_array().unwrap();
    assert_eq!(failing[0]["status"], "failed", "{}", failing[0]);
    assert_eq!(
        failing[0]["owning_tasks"],
        serde_json::json!(["TASK-F-002"])
    );
    assert!(
        failing[0]["stdout_tail"]
            .as_str()
            .is_some_and(|tail| tail.contains("no test ran")),
        "{}",
        failing[0]
    );
}

/// A7: a PRD requirement no frozen check covers blocks the round, naming the
/// supplementary check it is owed; the checks still run.
#[tokio::test]
async fn a_requirement_no_check_covers_blocks_the_round() {
    let fixture = fixture(true);
    std::fs::write(
        fixture.project.path().join("prd.md"),
        "# PRD\n\n- REQ-AB-001: the store keeps raw responses.\n",
    )
    .unwrap();
    let result = run(&fixture, &execution(1, 3, &[])).await.unwrap();
    assert_eq!(failing_ids(&result), vec!["REQ-2", "REQ-9"]);
    let errors = result.data["operational_errors"].to_string();
    assert!(errors.contains("SUP-REQ-AB-001"), "{errors}");
    let (record, _) = latest_round_record(&fixture.store.run_dir(&fixture.run_id))
        .unwrap()
        .unwrap();
    assert_eq!(record.checks.len(), 3);
    assert!(record.blocks_completion());
}
