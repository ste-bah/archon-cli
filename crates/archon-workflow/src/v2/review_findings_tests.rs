// The scenarios below were the prelude's JavaScript tests, ported verbatim to
// the host now that the host owns the rules. Each one was a live failure once.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::*;

/// A fanout map envelope whose findings carry NO task key of any kind -- the
/// exact shape all 43 adversarial findings had on one live run.
fn unattributed_map() -> Value {
    json!({"data":{"outcomes":[
        {"item_id":"review-map-0","result":{"data":{"findings":[
            {"id":"F1","claim":"registry write is not atomic","severity":"high"},
            {"id":"F2","claim":"no fsync on manifest","severity":"medium"}]}}},
        {"item_id":"review-map-1","result":{"data":{"findings":[
            {"id":"F3","claim":"validation report omits gaps","severity":"high"}]}}}
    ]}})
}

fn item_task_ids() -> BTreeMap<String, Vec<String>> {
    BTreeMap::from([
        ("review-map-0".to_string(), vec!["TASK-A-010".to_string()]),
        ("review-map-1".to_string(), vec!["TASK-A-020".to_string()]),
    ])
}

fn ids(finding: &Value) -> Vec<String> {
    task_ids_of(finding)
}

// --- the walk ---------------------------------------------------------------

/// Every named array, at every depth, through `data`/`result`, and through
/// `outcomes` rather than `items` when both views of a fan-out are present.
#[test]
fn the_walk_unions_every_findings_array_and_reads_one_fanout_view() {
    let envelope = json!({
        "data": {
            "findings": [{"id": "top"}],
            "adversarial_findings": [{"id": "adv"}],
            "outcomes": [{"result": {"data": {"uncovered_requirements": ["REQ-1"]}}}],
            "items": [{"result": {"data": {"findings": [{"id": "duplicate-view"}]}}}]
        }
    });
    let keys: Vec<String> = collect_findings(&envelope).iter().map(finding_key).collect();
    assert_eq!(keys, vec!["id:top", "id:adv", "\"REQ-1\""]);
}

#[test]
fn the_walk_falls_back_to_items_when_there_are_no_outcomes() {
    let envelope = json!({"data": {"items": [{"result": {"data": {"findings": [{"id": "from-items"}]}}}]}});
    assert_eq!(collect_findings(&envelope), vec![json!({"id": "from-items"})]);
}

// --- identity ---------------------------------------------------------------

#[test]
fn identity_is_any_shared_field_and_anonymous_findings_compare_exactly() {
    let a = json!({"id": "F1", "claim": "x", "severity": "high"});
    let b = json!({"claim": "x", "note": "reworded"});
    assert!(finding_identities(&a).iter().any(|k| finding_identities(&b).contains(k)));
    let bare = json!("REQ-SYN-001");
    assert!(finding_identities(&bare).is_empty());
    assert_eq!(finding_key(&bare), "\"REQ-SYN-001\"");
}

// --- attribution ------------------------------------------------------------

/// The headline defect: without stamping, every finding routes to
/// `unassigned` and remediation returns them untouched.
#[test]
fn map_findings_are_attributed_to_the_task_whose_branch_produced_them() {
    let stamped = attributed_map_findings(&unattributed_map()["data"], &item_task_ids());
    assert_eq!(stamped.len(), 3);
    assert_eq!(ids(&stamped[0]), vec!["TASK-A-010"]);
    assert_eq!(ids(&stamped[1]), vec!["TASK-A-010"]);
    assert_eq!(ids(&stamped[2]), vec!["TASK-A-020"]);
}

/// A reviewer that DID name its tasks keeps exactly what it named.
#[test]
fn reviewer_supplied_attribution_is_never_overwritten() {
    let map = json!({"data":{"outcomes":[{"item_id":"review-map-0","result":{"data":{"findings":[
        {"id":"F1","claim":"shared invariant broken","canonical_task_ids":["TASK-A-010","TASK-A-020"]}]}}}]}});
    let stamped = attributed_map_findings(&map["data"], &item_task_ids());
    assert_eq!(ids(&stamped[0]), vec!["TASK-A-010", "TASK-A-020"]);
}

/// The branch's own declaration wins over the host's item table: a branch that
/// reports which task it reviewed is believed.
#[test]
fn a_branch_that_declares_its_task_is_believed_over_the_item_table() {
    let map = json!({"data":{"outcomes":[{"item_id":"review-map-0","canonical_task_ids":["TASK-A-030"],
        "result":{"data":{"findings":[{"id":"F1","claim":"c"}]}}}]}});
    let stamped = attributed_map_findings(&map["data"], &item_task_ids());
    assert_eq!(ids(&stamped[0]), vec!["TASK-A-030"]);
}

/// A finding the host genuinely cannot place is still RETURNED. Dropping it
/// would trade a silent routing failure for a silent data loss.
#[test]
fn findings_from_an_unmappable_branch_are_kept_unattributed() {
    let map = json!({"data":{"outcomes":[{"item_id":"review-unknown","result":{"data":{"findings":[
        {"id":"F9","claim":"orphan finding"}]}}}]}});
    let stamped = attributed_map_findings(&map["data"], &item_task_ids());
    assert_eq!(stamped.len(), 1);
    assert!(ids(&stamped[0]).is_empty());
}

/// Not a fan-out at all: the plain reader applies, nothing is dropped.
#[test]
fn a_non_fanout_envelope_is_read_plainly() {
    let envelope = json!({"data": {"findings": [{"id": "F1"}]}});
    assert_eq!(
        attributed_map_findings(&envelope["data"], &BTreeMap::new()),
        vec![json!({"id": "F1"})]
    );
}

/// A bare-string finding (a requirement id, which the coverage contract
/// invites) must not be shredded into a character map.
#[test]
fn a_bare_string_finding_survives_stamping_and_merging() {
    let stamped = stamp_task_ids(json!("REQ-SYN-001"), &["TASK-A-010".to_string()]);
    assert_eq!(stamped, json!("REQ-SYN-001"));
    let merged = merge_map_and_reduce(vec![json!({"id": "F1"})], vec![json!("REQ-SYN-001")]);
    assert_eq!(merged, vec![json!({"id": "F1"}), json!("REQ-SYN-001")]);
}

// --- reattribution and merge ------------------------------------------------

/// `preserveMapFindings` is an instruction to a model, not a guarantee. A
/// reduce that returns the same findings stripped of attribution is repaired
/// from the stamped map set.
#[test]
fn a_reduce_that_drops_attribution_is_repaired_by_identity() {
    let stamped = attributed_map_findings(&unattributed_map()["data"], &item_task_ids());
    let reduce = vec![
        json!({"id":"F1","claim":"registry write is not atomic","severity":"high"}),
        json!({"id":"F3","claim":"validation report omits gaps","severity":"high"}),
    ];
    let repaired = reattribute(reduce, &stamped);
    assert_eq!(ids(&repaired[0]), vec!["TASK-A-010"]);
    assert_eq!(ids(&repaired[1]), vec!["TASK-A-020"]);
}

/// A reducer that RENAMES the attribution field has not violated anything it
/// was told; each alias still routes, and a dropped field is repaired.
#[test]
fn reattribution_survives_a_reducer_that_renames_or_drops_the_field() {
    let stamped = attributed_map_findings(&unattributed_map()["data"], &item_task_ids());
    let reduce = vec![
        json!({"id":"F1","claim":"registry write is not atomic","task_ids":["TASK-A-010"]}),
        json!({"id":"F2","claim":"no fsync on manifest","taskIds":["TASK-A-010"]}),
        json!({"id":"F3","claim":"validation report omits gaps","task_id":"TASK-A-020"}),
    ];
    let repaired = reattribute(reduce, &stamped);
    assert_eq!(ids(&repaired[0]), vec!["TASK-A-010"]);
    assert_eq!(ids(&repaired[1]), vec!["TASK-A-010"]);
    assert_eq!(ids(&repaired[2]), vec!["TASK-A-020"]);
}

/// Map findings are carried through structurally; a reduce finding that
/// restates one contributes nothing however it is reworded; a genuinely new
/// one is marked cross-cutting.
#[test]
fn merge_keeps_every_map_finding_and_only_new_reduce_findings() {
    let map = vec![json!({"id":"F1","claim":"registry write is not atomic"})];
    let reduce = vec![
        json!({"claim":"registry write is not atomic","note":"restated"}),
        json!({"id":"X1","claim":"tasks disagree about the manifest schema"}),
    ];
    let merged = merge_map_and_reduce(map, reduce);
    assert_eq!(merged.len(), 2);
    assert_eq!(merged[0]["id"], "F1");
    assert_eq!(merged[1]["id"], "X1");
    assert_eq!(merged[1]["finding_scope"], CROSS_CUTTING_SCOPE);
}

/// A bare-string map finding echoed by the reducer is a restatement too, and
/// must not be doubled in the merged set.
#[test]
fn a_reducer_echoing_a_bare_string_finding_does_not_double_it() {
    let merged = merge_map_and_reduce(
        vec![json!("REQ-SYN-001")],
        vec![json!("REQ-SYN-001"), json!("REQ-SYN-002")],
    );
    assert_eq!(merged, vec![json!("REQ-SYN-001"), json!("REQ-SYN-002")]);
}

// --- multisets --------------------------------------------------------------

#[test]
fn multiset_difference_honours_multiplicity() {
    let have = vec![json!("a"), json!("a"), json!({"id": "F1"})];
    let want = vec![json!("a"), json!({"id": "F1"})];
    assert_eq!(multiset_difference(&have, &want), vec![json!("a")]);
    assert!(multiset_difference(&want, &have).is_empty());
}

// --- attachment -------------------------------------------------------------

fn host_call(id: &str, contract: Value) -> WorkflowV2CallExecution {
    let extra = serde_json::from_value(json!({"reviewContract": contract})).expect("options");
    WorkflowV2CallExecution {
        call: crate::WorkflowV2HostCall {
            id: id.to_string(),
            method: crate::WorkflowV2HostMethod::Reduce,
            write_mode: None,
            options: crate::WorkflowV2HostOptions {
                extra,
                ..Default::default()
            },
        },
        input: Value::Null,
        depends_on: Vec::new(),
    }
}

fn reduce_result(findings: Value) -> WorkflowV2Result {
    WorkflowV2Result {
        status: crate::WorkflowV2Status::Accepted,
        summary: "reduced".to_string(),
        data: json!({"findings": findings}),
        ..WorkflowV2Result::default()
    }
}

/// A call without a review contract is left exactly as it was.
#[test]
fn calls_without_a_review_contract_are_untouched() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let mut execution = host_call("plain", json!({}));
    execution.call.options.extra.clear();
    let mut result = reduce_result(json!([{"id": "F1"}]));
    let before = result.clone();
    attach_host_review_findings(&execution, &mut result, &store).unwrap();
    assert_eq!(result, before);
}

/// The reduce attachment is the merged set: the maps' attributed findings plus
/// the reduce's own new ones, with the maps it named recorded -- and a named
/// map with no record is listed as missing rather than silently skipped.
#[test]
fn a_reduce_attaches_the_merged_set_and_names_missing_maps() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let map_execution = host_call("review-map", json!({"kind": "adversarial_findings", "stage": "map"}));
    let mut map_result = WorkflowV2Result {
        status: crate::WorkflowV2Status::Accepted,
        summary: "mapped".to_string(),
        data: unattributed_map()["data"].clone(),
        ..WorkflowV2Result::default()
    };
    // The map's attachment is what a reduce reads; write it as the host would.
    let stamped = attributed_map_findings(&map_result.data, &item_task_ids());
    attach(
        &mut map_result.data,
        &HostReviewFindings {
            kind: "adversarial_findings".to_string(),
            stage: "map".to_string(),
            map_finding_count: stamped.len(),
            reduce_finding_count: 0,
            findings: stamped,
            source_map_call_ids: Vec::new(),
            missing_source_map_call_ids: Vec::new(),
            baseline_finding_count: 0,
        },
    );
    store
        .save_call_record(&crate::WorkflowV2CallRecord::new(
            store.run_id(),
            map_execution.call.clone(),
            1,
            "input".to_string(),
            map_result,
            Vec::new(),
        ))
        .unwrap();

    let execution = host_call(
        "review-reduce",
        json!({"kind": "adversarial_findings", "stage": "reduce_final",
               "sourceMapCallIds": ["review-map", "never-recorded"]}),
    );
    let mut result = reduce_result(json!([
        {"id":"F1","claim":"registry write is not atomic"},
        {"id":"X1","claim":"tasks disagree about the manifest schema"}
    ]));
    attach_host_review_findings(&execution, &mut result, &store).unwrap();

    let findings = attached(&result.data).expect("host attachment");
    let keys: Vec<String> = findings.iter().map(|f| f["id"].as_str().unwrap().to_string()).collect();
    assert_eq!(keys, vec!["F1", "F2", "F3", "X1"]);
    assert_eq!(ids(&findings[0]), vec!["TASK-A-010"]);
    assert_eq!(findings[3]["finding_scope"], CROSS_CUTTING_SCOPE);
    assert_eq!(attached_missing_sources(&result.data), vec!["never-recorded"]);
    // The reduce's own `findings` array is untouched; the attachment sits beside it.
    assert_eq!(result.data["findings"].as_array().unwrap().len(), 2);
    // And the walk never reads the attachment as findings of its own.
    assert_eq!(collect_findings(&result.data).len(), 2);
}

/// A contract that names neither stage nor kind is refused, not guessed.
#[test]
fn a_review_contract_missing_stage_or_kind_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let execution = host_call("broken", json!({"kind": "adversarial_findings"}));
    let mut result = reduce_result(json!([]));
    let error = attach_host_review_findings(&execution, &mut result, &store).unwrap_err();
    assert!(error.to_string().contains("without both `stage` and `kind`"), "{error}");
}

/// Obs-31: a baseline failure routed to a task reaches remediation through
/// the first mandated review's final set, attributed to the owner, and only
/// there — the coverage audit's set and a map stage never carry it.
#[test]
fn routed_baseline_findings_join_the_adversarial_final_set_once_attributed_to_their_owner() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let finding = json!({
        "id": "baseline_regression_plan__tests__theirs",
        "canonical_task_ids": ["TASK-A-020"],
        "test_id": "plan::tests::theirs",
        "file": "crates/engine/src/plan/mod.rs",
    });
    crate::v2::write::test_baseline::route_finding(&store, "TASK-A-020", finding.clone());
    crate::v2::write::test_baseline::route_finding(&store, "TASK-A-020", finding.clone());

    let adversarial = host_call(
        "adversarial-review-reduce",
        json!({"kind": "adversarial_findings", "stage": "reduce_final", "sourceMapCallIds": []}),
    );
    let mut result = reduce_result(json!([{"id": "X1", "claim": "cross-task"}]));
    attach_host_review_findings(&adversarial, &mut result, &store).unwrap();
    let findings = attached(&result.data).expect("attachment");
    assert_eq!(findings.len(), 2, "{findings:?}");
    assert_eq!(findings[1]["id"], finding["id"]);
    assert_eq!(ids(&findings[1]), vec!["TASK-A-020"]);
    assert_eq!(result.data[HOST_REVIEW_FINDINGS_KEY]["baseline_finding_count"], 1);

    let coverage = host_call(
        "coverage-audit-reduce",
        json!({"kind": "uncovered_requirements", "stage": "reduce_final", "sourceMapCallIds": []}),
    );
    let mut result = reduce_result(json!([]));
    attach_host_review_findings(&coverage, &mut result, &store).unwrap();
    assert!(attached(&result.data).unwrap().is_empty());
}
