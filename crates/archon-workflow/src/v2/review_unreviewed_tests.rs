use serde_json::{Value, json};

use super::super::{
    WorkflowV2Result, WorkflowV2ResultStore, attach_host_review_findings, attached,
    collect_findings,
};
use super::*;
use crate::v2::call_execution::WorkflowV2CallExecution;

fn map_call() -> WorkflowV2CallExecution {
    let extra = serde_json::from_value(json!({"reviewContract": {
        "version": 1, "kind": "adversarial_findings", "stage": "map",
        "findingsPath": "data.findings", "itemTaskIdsPath": "canonical_task_ids"
    }}))
    .expect("options");
    WorkflowV2CallExecution {
        call: crate::WorkflowV2HostCall {
            id: "review-map".to_string(),
            method: crate::WorkflowV2HostMethod::Parallel,
            write_mode: None,
            options: crate::WorkflowV2HostOptions {
                extra,
                ..Default::default()
            },
        },
        // The runtime source argument, as `w.parallel(label, items)` sends it.
        input: json!({"source_data": [
            {"item_id": "review-a", "canonical_task_ids": ["TASK-A"], "task": "review"},
            {"item_id": "review-b", "canonical_task_ids": ["TASK-B"], "task": "review"},
            {"item_id": "review-c", "canonical_task_ids": ["TASK-C"], "task": "review"}
        ]}),
        depends_on: Vec::new(),
    }
}

/// The branch ids the host forms for the map's items, in item order.
fn branch_ids(execution: &WorkflowV2CallExecution, store: &WorkflowV2ResultStore) -> Vec<String> {
    crate::v2::call_data::fanout_items_for_call(execution, store)
        .expect("items")
        .into_iter()
        .map(|item| item.id)
        .collect()
}

fn reviewed(id: &str, findings: Value) -> Value {
    json!({"item_id": id, "status": "accepted",
           "result": {"status": "accepted", "data": {"findings": findings}}})
}

fn failed(id: &str, error: &str) -> Value {
    json!({"item_id": id, "status": "failed", "failure_kind": "execution",
           "result": null, "error": error})
}

fn map_result(outcomes: Vec<Value>) -> WorkflowV2Result {
    WorkflowV2Result {
        status: crate::WorkflowV2Status::NeedsReview,
        summary: "mapped".to_string(),
        data: json!({"outcomes": outcomes}),
        ..WorkflowV2Result::default()
    }
}

/// A branch that failed (after the host's one re-ask) leaves its task
/// explicitly UNREVIEWED in the host's finding set, attributed to that task —
/// never the zero findings a clean review reports.
#[test]
fn a_failed_review_branch_marks_its_task_unreviewed() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let execution = map_call();
    let ids = branch_ids(&execution, &store);
    let mut result = map_result(vec![
        reviewed(&ids[0], json!([{"id": "F1", "claim": "a real defect"}])),
        failed(
            &ids[1],
            "host call timeout: agent transport failed: subagent inactivity timeout: no model \
             output, tool call or tool result for 1800s [re-asked once by the host]",
        ),
        reviewed(&ids[2], json!([])),
    ]);

    attach_host_review_findings(&execution, &mut result, &store).unwrap();

    let findings = attached(&result.data).expect("host attachment");
    let unreviewed: Vec<&Value> = findings
        .iter()
        .filter(|f| is_unreviewed_finding(f))
        .collect();
    assert_eq!(unreviewed.len(), 1, "{findings:#?}");
    let marker = unreviewed[0];
    assert_eq!(marker["canonical_task_ids"], json!(["TASK-B"]));
    assert_eq!(marker["severity"], "blocking");
    // Never routed to a writer, which could only misreport it as refuted.
    assert_eq!(marker["attributable_to_task"], false);
    assert_eq!(marker[REVIEW_OUTCOME_KEY], UNREVIEWED_OUTCOME);
    let claim = marker["claim"].as_str().unwrap();
    assert!(claim.contains("UNREVIEWED"), "{claim}");
    assert!(claim.contains("inactivity timeout"), "{claim}");
    let attachment = &result.data[crate::v2::review_findings::HOST_REVIEW_FINDINGS_KEY];
    assert_eq!(attachment[UNREVIEWED_TASK_IDS_KEY], json!(["TASK-B"]));
    assert_eq!(attachment["map_finding_count"], 2);
    // The branch's own (absent) findings are still zero; the marker is the
    // host's, beside them, not a forged reviewer finding.
    assert_eq!(collect_findings(&result.data).len(), 1);
}

/// A clean review is still zero findings, with no unreviewed marker at all.
#[test]
fn a_clean_review_yields_zero_findings_and_no_unreviewed_marker() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let execution = map_call();
    let ids = branch_ids(&execution, &store);
    let mut result = map_result(ids.iter().map(|id| reviewed(id, json!([]))).collect());

    attach_host_review_findings(&execution, &mut result, &store).unwrap();

    assert_eq!(
        attached(&result.data).expect("attachment"),
        Vec::<Value>::new()
    );
    let attachment = &result.data[crate::v2::review_findings::HOST_REVIEW_FINDINGS_KEY];
    assert_eq!(attachment[UNREVIEWED_TASK_IDS_KEY], json!([]));
    assert_eq!(attachment["map_finding_count"], 0);
}

/// A branch that returned findings reviewed its task whatever status it
/// reported; one that returned a failed verdict with nothing to show did not.
#[test]
fn a_verdict_with_findings_is_a_review_and_an_empty_failure_is_not() {
    let item_ids = BTreeMap::from([
        ("b0".to_string(), vec!["T0".to_string()]),
        ("b1".to_string(), vec!["T1".to_string()]),
        ("b2".to_string(), vec!["T2".to_string()]),
    ]);
    let data = json!({"outcomes": [
        {"item_id": "b0", "status": "needs_review",
         "result": {"status": "needs_review", "data": {"findings": [{"id": "X"}]}}},
        {"item_id": "b1", "status": "failed",
         "result": {"status": "failed", "data": {"findings": [{"id": "Y"}]}}},
        {"item_id": "b2", "status": "failed", "result": {"status": "failed", "data": {}}}
    ]});
    let markers = unreviewed_findings(&data, &item_ids, "coverage");
    assert_eq!(markers.len(), 1);
    assert_eq!(unreviewed_task_ids(&markers), vec!["T2"]);
    assert_eq!(markers[0]["review_branch_id"], "b2");
    // Two failed branches are two markers: their identities never collide.
    let two = json!({"outcomes": [failed("b1", "x"), failed("b2", "x")]});
    let markers = unreviewed_findings(&two, &item_ids, "coverage");
    assert_eq!(unreviewed_task_ids(&markers), vec!["T1", "T2"]);
    assert_ne!(
        crate::v2::review_findings::finding_key(&markers[0]),
        crate::v2::review_findings::finding_key(&markers[1])
    );
    // A value with no branch views is not a map this speaks for: neither a
    // map that ran no branch at all nor a view that is not a stored outcome.
    for empty in [
        json!({"findings": []}),
        json!({"outcomes": [], "items": []}),
        json!({"items": [{"task": "an input item, not an outcome"}]}),
    ] {
        assert!(
            unreviewed_findings(&empty, &item_ids, "k").is_empty(),
            "{empty}"
        );
    }
}

/// The reduce's final set carries the marker through from the map record, so
/// the accounting the script reports shows the task as unreviewed.
#[test]
fn the_unreviewed_marker_survives_into_the_final_reduce_set() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let map_execution = map_call();
    let ids = branch_ids(&map_execution, &store);
    let mut map = map_result(vec![
        reviewed(&ids[0], json!([])),
        failed(&ids[1], "subagent timed out after 14400s"),
        reviewed(&ids[2], json!([])),
    ]);
    attach_host_review_findings(&map_execution, &mut map, &store).unwrap();
    store
        .save_call_record(&crate::WorkflowV2CallRecord::new(
            store.run_id(),
            map_execution.call.clone(),
            1,
            "input".to_string(),
            map,
            Vec::new(),
        ))
        .unwrap();

    let extra = serde_json::from_value(json!({"reviewContract": {
        "version": 1, "kind": "adversarial_findings", "stage": "reduce_final",
        "sourceMapCallIds": ["review-map"], "preserveMapFindings": true
    }}))
    .unwrap();
    let reduce_execution = WorkflowV2CallExecution {
        call: crate::WorkflowV2HostCall {
            id: "review-reduce".to_string(),
            method: crate::WorkflowV2HostMethod::Reduce,
            write_mode: None,
            options: crate::WorkflowV2HostOptions {
                extra,
                ..Default::default()
            },
        },
        input: Value::Null,
        depends_on: Vec::new(),
    };
    let mut reduce = WorkflowV2Result {
        status: crate::WorkflowV2Status::Accepted,
        summary: "reduced".to_string(),
        data: json!({"findings": []}),
        ..WorkflowV2Result::default()
    };
    attach_host_review_findings(&reduce_execution, &mut reduce, &store).unwrap();

    let final_set = attached(&reduce.data).expect("final set");
    assert_eq!(unreviewed_task_ids(&final_set), vec!["TASK-B"]);
    let attachment = &reduce.data[crate::v2::review_findings::HOST_REVIEW_FINDINGS_KEY];
    assert_eq!(attachment[UNREVIEWED_TASK_IDS_KEY], json!(["TASK-B"]));
}

#[test]
fn a_review_map_is_known_by_its_contract_not_its_name() {
    assert!(crate::v2::review_findings::is_review_map_call(&map_call()));
    let mut renamed = map_call();
    renamed.call.id = "anything-at-all".to_string();
    assert!(crate::v2::review_findings::is_review_map_call(&renamed));
    renamed.call.options.extra.clear();
    assert!(!crate::v2::review_findings::is_review_map_call(&renamed));
}
