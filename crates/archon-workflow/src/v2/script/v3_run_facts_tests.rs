//! Host call records reduced to terminal-rule facts.

use super::*;
use crate::v2::WorkflowV2SourceTaskGraph;
use crate::v2::acceptance_stage::ACCEPTANCE_STAGE_TOOL;

fn call(id: &str, method: WorkflowV2HostMethod, extra: serde_json::Value) -> WorkflowV2HostCall {
    let mut options = WorkflowV2HostOptions::default();
    if let serde_json::Value::Object(map) = extra {
        options.extra = map.into_iter().collect();
    }
    WorkflowV2HostCall {
        id: id.into(),
        method,
        write_mode: None,
        options,
    }
}

fn record(
    call: &WorkflowV2HostCall,
    status: WorkflowV2Status,
    summary: &str,
    data: serde_json::Value,
) -> WorkflowV2CallRecord {
    WorkflowV2CallRecord::new(
        "run",
        call.clone(),
        1,
        "input".into(),
        WorkflowV2Result {
            status,
            summary: summary.into(),
            data,
            ..WorkflowV2Result::default()
        },
        Vec::new(),
    )
}

#[test]
fn roles_follow_the_host_contracts_on_the_call() {
    let mut write = call(
        "agents-1",
        WorkflowV2HostMethod::Fanout,
        serde_json::json!({}),
    );
    write.write_mode = Some(WorkflowV2WriteMode::Worktree);
    assert_eq!(authored_call_role(&write), AuthoredCallRole::Write);
    let mut verify = call(
        "verification-wave-verify-a-2",
        WorkflowV2HostMethod::Parallel,
        serde_json::json!({}),
    );
    verify.options.item_kind = Some("focused_verification".into());
    assert_eq!(authored_call_role(&verify), AuthoredCallRole::TaskVerify);
    let map = call(
        "adversarial-review-map",
        WorkflowV2HostMethod::Parallel,
        serde_json::json!({ "reviewContract": { "kind": "adversarial_findings", "stage": "map" } }),
    );
    assert_eq!(
        authored_call_role(&map),
        AuthoredCallRole::Review {
            kind: "adversarial_findings".into(),
            stage: "map".into()
        }
    );
    let contract = |stage: &str| serde_json::json!({ "remediationContract": { "stage": stage, "taskId": "TASK-A", "round": 2 } });
    assert_eq!(
        authored_call_role(&call(
            "fix",
            WorkflowV2HostMethod::Fanout,
            contract("remediate")
        )),
        AuthoredCallRole::RemediationFix {
            task: "TASK-A".into(),
            round: 2
        }
    );
    // The no-patch checkpoint carries a verify contract but is no verifier.
    assert_eq!(
        authored_call_role(&call(
            "review-verify-a-2-no-patch",
            WorkflowV2HostMethod::Checkpoint,
            contract("verify")
        )),
        AuthoredCallRole::RemediationVerify {
            task: "TASK-A".into(),
            round: 2,
            agent: false
        }
    );
    assert_eq!(
        authored_call_role(&call(
            "verification-wave-review-verify-a-2",
            WorkflowV2HostMethod::Parallel,
            contract("verify")
        )),
        AuthoredCallRole::RemediationVerify {
            task: "TASK-A".into(),
            round: 2,
            agent: true
        }
    );
    let acceptance = call(
        "acceptance-contract-run-1",
        WorkflowV2HostMethod::Tool,
        serde_json::json!({ "tool": ACCEPTANCE_STAGE_TOOL }),
    );
    assert_eq!(
        authored_call_role(&acceptance),
        AuthoredCallRole::Acceptance
    );
}

#[test]
fn per_task_status_comes_from_branch_outcomes_not_the_wave() {
    let mut write = call(
        "agents-4",
        WorkflowV2HostMethod::Fanout,
        serde_json::json!({}),
    );
    write.write_mode = Some(WorkflowV2WriteMode::Worktree);
    let data = serde_json::json!({ "outcomes": [
        { "item_id": "a", "canonical_task_ids": ["TASK-A"], "status": "accepted" },
        { "item_id": "b", "canonical_task_ids": ["TASK-B"], "status": "accepted", "contract_valid": false },
        { "item_id": "c", "canonical_task_ids": [], "task_ids": ["TASK-C"], "status": "failed", "failure_kind": "execution" },
    ]});
    let fact = call_fact(
        &write,
        Some(&record(&write, WorkflowV2Status::NeedsReview, "wave", data)),
    );
    assert_eq!(
        fact.task("TASK-A").unwrap().status,
        WorkflowV2Status::Accepted
    );
    assert_eq!(
        fact.task("TASK-B").unwrap().status,
        WorkflowV2Status::NeedsReview
    );
    let c = fact.task("TASK-C").unwrap();
    assert_eq!((c.status, c.transport), (WorkflowV2Status::Failed, true));
}

#[test]
fn a_wholesale_failure_falls_back_to_the_source_graph_or_names_no_task() {
    let verify = call(
        "verification-wave-verify-a-3",
        WorkflowV2HostMethod::Parallel,
        serde_json::json!({}),
    );
    let mut failed = record(
        &verify,
        WorkflowV2Status::Failed,
        "agent transport failed: 520",
        serde_json::json!({ "error": "x" }),
    );
    let graph: WorkflowV2SourceTaskGraph = serde_json::from_value(serde_json::json!({
        "schema_version": "v1",
        "items": [{ "item_id": "verify-a-3-check", "canonical_task_ids": ["TASK-A"] }],
    }))
    .unwrap();
    failed.source_task_graph = Some(graph);
    let fact = call_fact(&verify, Some(&failed));
    assert!(fact.transport);
    assert_eq!(
        fact.task("TASK-A").unwrap().status,
        WorkflowV2Status::Failed
    );
    let mut write = call(
        "remediate-a-3",
        WorkflowV2HostMethod::Fanout,
        serde_json::json!({}),
    );
    write.write_mode = Some(WorkflowV2WriteMode::Worktree);
    let fact = call_fact(
        &write,
        Some(&record(
            &write,
            WorkflowV2Status::Failed,
            "bad input",
            serde_json::json!({}),
        )),
    );
    assert!(fact.tasks.is_empty());
    assert!(!fact.transport);
}

#[test]
fn a_repeated_call_keeps_its_last_position_and_invalidated_records_are_absent() {
    let one = call("one", WorkflowV2HostMethod::Agent, serde_json::json!({}));
    let two = call("two", WorkflowV2HostMethod::Agent, serde_json::json!({}));
    let facts = authored_call_facts(&[one.clone(), two.clone(), one.clone()], |id| {
        let mut rec = record(
            &one,
            WorkflowV2Status::Accepted,
            "ok",
            serde_json::json!({}),
        );
        if id == "two" {
            rec.invalidated_by = Some("restart".into());
        }
        Ok(Some(rec))
    })
    .unwrap();
    assert_eq!(
        facts.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(),
        vec!["two", "one"]
    );
    assert_eq!(facts[0].status, None);
}

#[test]
fn writable_tasks_are_those_declaring_a_file() {
    let task = |id: &str, files: &[&str], shared: &[&str]| {
        crate::task_universe::WorkflowV2TaskUniverseTask {
            canonical_task_id: id.into(),
            files_expected_to_change: files.iter().map(|s| s.to_string()).collect(),
            shared_append_target_files: shared.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    };
    let universe = crate::task_universe::WorkflowV2TaskUniverse {
        schema_version: "t".into(),
        source_roots: Vec::new(),
        tasks: vec![
            task("A", &["src/a.rs"], &[]),
            task("B", &[], &["mod.rs"]),
            task("C", &["  "], &[]),
        ],
    };
    let writable = writable_task_ids(Some(&universe));
    assert_eq!(writable.into_iter().collect::<Vec<_>>(), vec!["A", "B"]);
}

// 5: a branch agent cannot claim tasks its host-built item was not given.
#[test]
fn task_attribution_comes_from_the_host_item_not_the_agent_report() {
    let mut write = call("w", WorkflowV2HostMethod::Fanout, serde_json::json!({}));
    write.write_mode = Some(WorkflowV2WriteMode::Worktree);
    let data = serde_json::json!({ "outcomes": [
        { "item_id": "w-0", "canonical_task_ids": ["TASK-A", "TASK-B"], "status": "accepted" },
    ]});
    let mut rec = record(&write, WorkflowV2Status::Accepted, "ok", data);
    let graph: WorkflowV2SourceTaskGraph = serde_json::from_value(serde_json::json!({
        "schema_version": "v1",
        "items": [{ "item_id": "a", "canonical_task_ids": ["TASK-A"] }],
    }))
    .unwrap();
    rec.source_task_graph = Some(graph);
    let fact = call_fact(&write, Some(&rec));
    assert_eq!(
        fact.task("TASK-A").unwrap().status,
        WorkflowV2Status::Accepted
    );
    assert!(fact.task("TASK-B").is_none(), "{:?}", fact.tasks);
}

// The host's dispatched items decide attribution; a dispatched branch that
// never reported is charged the call's status and is no review.
#[test]
fn dispatched_items_attribute_every_branch_including_silent_ones() {
    let map = call("m", WorkflowV2HostMethod::Parallel, serde_json::json!({}));
    let data = serde_json::json!({ "outcomes": [
        { "item_id": "m-0", "canonical_task_ids": ["TASK-X"], "status": "failed", "failure_kind": "semantic", "result": { "status": "failed" } },
    ]});
    let rec =
        record(&map, WorkflowV2Status::NeedsReview, "wave", data).with_dispatched_items(vec![
            crate::v2::WorkflowV2DispatchedItem {
                item_id: "m-0".into(),
                canonical_task_ids: vec!["TASK-A".into()],
            },
            crate::v2::WorkflowV2DispatchedItem {
                item_id: "m-1".into(),
                canonical_task_ids: vec!["TASK-B".into()],
            },
        ]);
    let fact = call_fact(&map, Some(&rec));
    assert!(!fact.agent_attributed);
    assert!(
        fact.task("TASK-X").is_none(),
        "the agent's own claim is ignored"
    );
    let a = fact.task("TASK-A").unwrap();
    assert_eq!(
        (a.status, a.not_reviewed),
        (WorkflowV2Status::Failed, false),
        "a verdict is a review"
    );
    let b = fact.task("TASK-B").unwrap();
    assert_eq!(
        (b.status, b.not_reviewed),
        (WorkflowV2Status::NeedsReview, true)
    );
}
