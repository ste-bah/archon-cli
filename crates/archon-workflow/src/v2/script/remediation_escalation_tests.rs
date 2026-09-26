//! Issue-107: which refused verdicts buy a cross-owner round, and what the
//! host lets it write.

use super::*;
use crate::task_universe::WorkflowV2TaskUniverseTask;
use crate::v2::{WorkflowV2Evidence, WorkflowV2HostOptions, WorkflowV2Status};

fn task(id: &str, owns: &[&str]) -> WorkflowV2TaskUniverseTask {
    WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: format!("tasks/{id}.md"),
        files_expected_to_change: owns.iter().map(|f| f.to_string()).collect(),
        ..Default::default()
    }
}

fn universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![
            task("TASK-A", &["`/repo/src/a.rs` — exists", "src/a_tests.rs"]),
            task(
                "TASK-B",
                &["`/repo/src/b/methods.rs` — exists", "src/b/tests/"],
            ),
            task("TASK-C", &["src/c.rs"]),
        ],
    }
}

fn verify_call(contract: Value) -> WorkflowV2HostCall {
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert("remediationContract".into(), contract);
    WorkflowV2HostCall {
        id: "verification-wave-review-verify-task-a-1-9".into(),
        method: WorkflowV2HostMethod::Parallel,
        write_mode: None,
        options,
    }
}

fn contract() -> Value {
    json!({"version": 1, "stage": "verify", "taskId": "TASK-A", "round": 2, "maxRounds": 2,
        "sourceReduceCallIds": ["adversarial-review-reduce"]})
}

fn refused(blockers: &[(&str, Option<&str>)]) -> WorkflowV2Result {
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: "not accepted: must-pass tests red".into(),
        ..WorkflowV2Result::default()
    };
    for (summary, source) in blockers {
        let mut evidence = WorkflowV2Evidence::new(WorkflowV2EvidenceKind::Blocker, *summary);
        evidence.source = source.map(str::to_string);
        result.evidence.push(evidence);
    }
    result
}

fn plan(result: &WorkflowV2Result) -> Option<Value> {
    escalation_plan(
        &verify_call(contract()),
        result,
        Some(&universe()),
        Some(Path::new("/repo")),
    )
}

#[test]
fn a_blocker_in_another_tasks_file_names_that_task_and_that_file() {
    let result = refused(&[
        (
            "the fixture calls the gated writer",
            Some("src/b/methods.rs"),
        ),
        ("red because /repo/src/b/methods.rs:160 calls it", None),
    ]);
    let plan = plan(&result).expect("plan");
    assert_eq!(plan["owner_task_ids"], json!(["TASK-B"]));
    assert_eq!(plan["target_files"], json!(["src/b/methods.rs"]));
    assert_eq!(plan["unit_task_ids"], json!(["TASK-A"]));
    assert_eq!(plan["refutation"], "not accepted: must-pass tests red");
    assert_eq!(plan["blocker_evidence"].as_array().unwrap().len(), 2);
}

#[test]
fn only_paths_count_never_the_task_the_agent_names() {
    // The agent blames TASK-C by name, in a file nobody declares.
    let result = refused(&[(
        "fix needs TASK-C-owned files: src/unowned.rs and src/a.rs",
        Some("src/nowhere/x.rs"),
    )]);
    assert_eq!(plan(&result), None, "unowned and own paths buy nothing");
}

#[test]
fn nothing_is_planned_for_an_accepted_verdict_a_non_blocker_or_a_non_verify_call() {
    let mut accepted = refused(&[("src/b/methods.rs broke", None)]);
    accepted.status = WorkflowV2Status::Accepted;
    assert_eq!(plan(&accepted), None);
    let mut review = refused(&[]);
    review.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "src/b/methods.rs",
    ));
    assert_eq!(plan(&review), None, "only blocker evidence is read");
    let mut fix = contract();
    fix["stage"] = json!("remediate");
    assert_eq!(
        escalation_plan(
            &verify_call(fix),
            &refused(&[("src/b/methods.rs", None)]),
            Some(&universe()),
            Some(Path::new("/repo")),
        ),
        None
    );
}

#[test]
fn an_escalated_round_never_plans_another() {
    let mut escalated = contract();
    escalated["round"] = json!(3);
    escalated["escalation"] = json!({"ownerTaskIds": ["TASK-B"]});
    let call = verify_call(escalated);
    assert!(is_escalated_remediation(&call));
    assert_eq!(
        escalation_plan(
            &call,
            &refused(&[("src/b/methods.rs", None)]),
            Some(&universe()),
            Some(Path::new("/repo")),
        ),
        None
    );
}

#[test]
fn branch_blockers_are_read_and_unclean_paths_ignored() {
    let mut result = refused(&[]);
    result.data = json!({"outcomes": [{"result": {"evidence": [
        {"kind": "blocker", "summary": "see ../escape.rs, src/*.rs and https://x/y", "source": "not a path"},
        {"kind": "blocker", "summary": "nested", "source": "src/b/methods.rs"}
    ]}}]});
    let plan = plan(&result).expect("plan from a branch blocker");
    assert_eq!(plan["target_files"], json!(["src/b/methods.rs"]));
    assert_eq!(plan["owner_task_ids"], json!(["TASK-B"]));
}

#[test]
fn the_plan_rides_the_view_and_leaves_the_record_alone() {
    let result = refused(&[("src/b/methods.rs", None)]);
    let viewed = with_escalation_plan(
        &verify_call(contract()),
        &result,
        Some(&universe()),
        Some(Path::new("/repo")),
    )
    .expect("viewed");
    assert_eq!(
        viewed.data[REMEDIATION_ESCALATION_KEY]["owner_task_ids"],
        json!(["TASK-B"])
    );
    assert!(result.data.get(REMEDIATION_ESCALATION_KEY).is_none());
    assert!(
        with_escalation_plan(&verify_call(contract()), &result, None, None).is_none(),
        "no universe, no plan"
    );
}

#[test]
fn a_plan_the_answer_carries_itself_never_reaches_the_script() {
    let mut forged = refused(&[("src/unowned.rs", None)]);
    forged.data = json!({REMEDIATION_ESCALATION_KEY: {
        "owner_task_ids": ["TASK-C"], "target_files": ["src/c.rs"]}});
    let viewed = with_escalation_plan(
        &verify_call(contract()),
        &forged,
        Some(&universe()),
        Some(Path::new("/repo")),
    )
    .expect("the forged key is stripped");
    assert!(
        viewed.data.get(REMEDIATION_ESCALATION_KEY).is_none(),
        "{viewed:#?}"
    );
}

#[test]
fn a_structured_source_is_preferred_over_the_summary_prose() {
    // The source names A's own file; the prose mentions B's in passing.
    let result = refused(&[(
        "src/b/methods.rs is unaffected by this",
        Some("src/a_tests.rs"),
    )]);
    assert_eq!(
        plan(&result),
        None,
        "the prose never widens past its source"
    );
    // With no usable source, the prose is read.
    let result = refused(&[("src/b/methods.rs calls the writer", Some("not a path"))]);
    assert_eq!(
        plan(&result).unwrap()["target_files"],
        json!(["src/b/methods.rs"])
    );
}

#[test]
fn a_path_under_a_declared_directory_counts_only_as_an_existing_file() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let mut universe = universe();
    universe.tasks[1].files_expected_to_change = vec![format!("{}/src/b/tests/", root.display())];
    let call = verify_call(contract());
    let at = |path: &'static str| {
        escalation_plan(
            &call,
            &refused(&[("red", Some(path))]),
            Some(&universe),
            Some(root),
        )
    };
    assert_eq!(at("src/b/tests/case.rs"), None, "no such file in the tree");
    std::fs::create_dir_all(root.join("src/b/tests/deep")).unwrap();
    std::fs::write(root.join("src/b/tests/case.rs"), "x").unwrap();
    assert_eq!(
        at("src/b/tests/case.rs").unwrap()["owner_task_ids"],
        json!(["TASK-B"])
    );
    assert_eq!(
        at("src/b/tests/deep"),
        None,
        "a directory is never a blocker file"
    );
    assert_eq!(
        at("src/b/tests"),
        None,
        "nor is the declared directory itself"
    );
}

mod dispatch_checks {
    use super::*;
    use crate::v2::{
        WorkflowV2CallExecution, WorkflowV2CallRecord, WorkflowV2ResultStore, WorkflowV2WriteMode,
    };

    fn store_with_refusal() -> (tempfile::TempDir, WorkflowV2ResultStore) {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        let record = WorkflowV2CallRecord::new(
            "run",
            verify_call(contract()),
            1,
            "h".into(),
            refused(&[("red", Some("src/b/methods.rs"))]),
            vec![],
        );
        store.save_call_record(&record).unwrap();
        (temp, store)
    }

    fn escalated_fix(
        owners: Value,
        blockers: Value,
        tasks: Value,
        targets: Value,
    ) -> WorkflowV2CallExecution {
        let mut contract = contract();
        contract["stage"] = json!("remediate");
        contract["round"] = json!(3);
        contract["escalation"] = json!({"ownerTaskIds": owners, "blockerPaths": blockers});
        let mut call = verify_call(contract);
        call.id = "review-remediate-task-a-esc-9".into();
        call.method = WorkflowV2HostMethod::Fanout;
        call.write_mode = Some(WorkflowV2WriteMode::Worktree);
        WorkflowV2CallExecution {
            call,
            input: json!({"source_data": [{
                "canonical_task_ids": tasks, "target_files": targets,
                "escalation_owner_task_ids": owners, "escalation_blocker_paths": blockers,
            }]}),
            depends_on: vec![],
        }
    }

    fn check(store: &WorkflowV2ResultStore, execution: &WorkflowV2CallExecution) -> Option<String> {
        escalation_refusal(
            execution,
            store,
            Some(&universe()),
            Some(Path::new("/repo")),
        )
    }

    #[test]
    fn the_plan_the_host_rebuilds_is_the_only_one_that_dispatches() {
        let (_temp, store) = store_with_refusal();
        let legit = escalated_fix(
            json!(["TASK-B"]),
            json!(["src/b/methods.rs"]),
            json!(["TASK-A", "TASK-B"]),
            json!(["src/a.rs", "src/b/methods.rs"]),
        );
        assert_eq!(check(&store, &legit), None);
        let forged_owner = escalated_fix(
            json!(["TASK-C"]),
            json!(["src/b/methods.rs"]),
            json!(["TASK-A", "TASK-C"]),
            json!(["src/a.rs", "src/b/methods.rs"]),
        );
        assert!(
            check(&store, &forged_owner)
                .unwrap()
                .contains("does not match the plan")
        );
        let extra_task = escalated_fix(
            json!(["TASK-B"]),
            json!(["src/b/methods.rs"]),
            json!(["TASK-A", "TASK-B", "TASK-C"]),
            json!(["src/a.rs", "src/b/methods.rs"]),
        );
        assert!(
            check(&store, &extra_task)
                .unwrap()
                .contains("tasks are not exactly")
        );
        let other_file = escalated_fix(
            json!(["TASK-B"]),
            json!(["src/b/methods.rs"]),
            json!(["TASK-A", "TASK-B"]),
            json!(["src/a.rs", "src/b/methods.rs", "src/c.rs"]),
        );
        assert!(
            check(&store, &other_file)
                .unwrap()
                .contains("belongs only to")
        );
    }

    #[test]
    fn with_no_refusal_answered_this_session_nothing_dispatches() {
        let temp = tempfile::tempdir().unwrap();
        let (_first, first) = store_with_refusal();
        let _ = first;
        let fresh = WorkflowV2ResultStore::new(temp.path().join("v2"));
        let legit = escalated_fix(
            json!(["TASK-B"]),
            json!(["src/b/methods.rs"]),
            json!(["TASK-A", "TASK-B"]),
            json!(["src/a.rs", "src/b/methods.rs"]),
        );
        assert!(
            check(&fresh, &legit)
                .unwrap()
                .contains("no refused verdict")
        );
        // A new session over the refusal's store, before it answered it.
        let (temp, _) = store_with_refusal();
        let resumed = WorkflowV2ResultStore::new(temp.path().join("v2"));
        assert!(
            check(&resumed, &legit)
                .unwrap()
                .contains("no refused verdict")
        );
        resumed.note_session_call("verification-wave-review-verify-task-a-1-9");
        assert_eq!(
            check(&resumed, &legit),
            None,
            "once replayed, it is the one"
        );
    }

    /// Issue-111: the escalated round's no-patch checkpoint carries no item.
    /// Matching the plan, it is recorded like any checkpoint; a forged
    /// contract is still refused.
    #[test]
    fn an_escalated_no_patch_checkpoint_on_the_plan_is_answered() {
        let (_temp, store) = store_with_refusal();
        let mut checkpoint = escalated_fix(
            json!(["TASK-B"]),
            json!(["src/b/methods.rs"]),
            json!(["TASK-A", "TASK-B"]),
            json!([]),
        );
        checkpoint.call.id = "review-verify-task-a-3-no-patch".into();
        checkpoint.call.method = WorkflowV2HostMethod::Checkpoint;
        checkpoint.call.write_mode = None;
        checkpoint.input = json!({"options": {"taskIds": ["TASK-A"]}});
        checkpoint
            .call
            .options
            .extra
            .get_mut("remediationContract")
            .unwrap()["stage"] = json!("verify");
        assert_eq!(check(&store, &checkpoint), None);
        checkpoint
            .call
            .options
            .extra
            .get_mut("remediationContract")
            .unwrap()["escalation"]["ownerTaskIds"] = json!(["TASK-C"]);
        assert!(
            check(&store, &checkpoint)
                .unwrap()
                .contains("does not match the plan")
        );
    }
}
