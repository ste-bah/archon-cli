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
    let result = refused(&[(
        "red at src/b/tests/gate.rs:8 because /repo/src/b/methods.rs:160 calls the gated writer",
        Some("src/b/tests/gate.rs"),
    )]);
    let plan = plan(&result).expect("plan");
    assert_eq!(plan["owner_task_ids"], json!(["TASK-B"]));
    assert_eq!(
        plan["target_files"],
        json!(["src/b/methods.rs", "src/b/tests/gate.rs"])
    );
    assert_eq!(plan["unit_task_ids"], json!(["TASK-A"]));
    assert_eq!(plan["refutation"], "not accepted: must-pass tests red");
    assert_eq!(plan["blocker_evidence"].as_array().unwrap().len(), 1);
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
        {"kind": "blocker", "summary": "see ../escape.rs, src/*.rs and https://x/y", "source": "src/b/tests/deep/case.rs"}
    ]}}]});
    let plan = plan(&result).expect("plan from a branch blocker");
    assert_eq!(plan["target_files"], json!(["src/b/tests/deep/case.rs"]));
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
