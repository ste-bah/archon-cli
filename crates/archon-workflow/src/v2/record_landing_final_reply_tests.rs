//! Issue-550: a call that lands skeleton records has one final-answer
//! contract. Every receipt ends with the exact final reply, the host composes
//! the whole artifact from the retained records, and a session that cannot
//! give that reply itself can still be completed from what it retained.
use super::record_landing::{RecordKind, RecordLanding};
use serde_json::{Value, json};

fn open(kind: RecordKind, subjects: &[&str]) -> (tempfile::TempDir, RecordLanding) {
    let temp = tempfile::tempdir().unwrap();
    let landing = RecordLanding::open(
        temp.path().join("records"),
        "identity".into(),
        kind,
        subjects.iter().map(|s| s.to_string()).collect(),
        kind != RecordKind::Skeleton,
    )
    .unwrap();
    (temp, landing)
}
fn task_record(id: &str) -> Value {
    json!({"subject":id,"summary":"one unit of work","evidence":[{"kind":"inspection","summary":"read the source"}],
        "task":{"task_id":id,"file_name":format!("{id}.md"),"implements":["REQ-1"]}})
}

#[test]
fn issue550_skeleton_receipt_ends_with_the_exact_final_reply() {
    let (_temp, landing) = open(RecordKind::Skeleton, &[]);
    landing.land(task_record("TASK-X-001")).unwrap();
    landing.land(task_record("TASK-X-002")).unwrap();
    let hint = landing.hint().unwrap();
    assert!(hint.ends_with(r#"{"records_landed":2}"#), "{hint}");
    assert!(
        hint.contains("acceptance_digest") && hint.contains("host composes"),
        "the receipt says the host fills the artifact fields: {hint}"
    );
    assert!(!hint.contains("final raw skeleton may contain"), "{hint}");
    assert!(
        !hint.contains("unions with the earlier record"),
        "a skeleton re-landing replaces its task: {hint}"
    );
    let review = open(RecordKind::Review, &["S"]).1.hint().unwrap();
    assert!(
        review.contains("Final data.records_landed must equal the complete saved count."),
        "{review}"
    );
}

#[test]
fn issue550_host_composes_the_whole_skeleton_from_retained_records() {
    let (_temp, landing) = open(RecordKind::Skeleton, &[]);
    landing.land(task_record("TASK-X-001")).unwrap();
    // The live reply: a count, a schema version, an unknown key, no digest.
    let reply = json!({"records_landed":1,"schema_version":1,"subjects":[{"summary":"x"}]});
    let artifact = landing.compose(&reply).unwrap();
    let mut keys = artifact.as_object().unwrap().keys().collect::<Vec<_>>();
    keys.sort();
    assert_eq!(keys, ["acceptance_digest", "schema_version", "tasks"]);
    assert_eq!(artifact["schema_version"], 1);
    assert_eq!(artifact["acceptance_digest"], "");
    assert_eq!(artifact["tasks"][0]["task_id"], "TASK-X-001");
    let error = landing
        .compose(&json!({"records_landed":2}))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("records_landed=2 but host retained 1 record(s)"),
        "{error}"
    );
}

#[test]
fn issue550_only_a_skeleton_with_retained_records_offers_a_host_completion() {
    let (_temp, skeleton) = open(RecordKind::Skeleton, &[]);
    assert_eq!(skeleton.retained_completion().unwrap(), None);
    skeleton.land(task_record("TASK-X-001")).unwrap();
    let completion = skeleton.retained_completion().unwrap().unwrap();
    assert_eq!(completion, json!({"records_landed":1}));
    skeleton.assemble(&completion).unwrap();
    let (_temp, review) = open(RecordKind::Review, &["S"]);
    review
        .land(json!({"subject":"S","evidence":[{"kind":"inspection","summary":"read"}]}))
        .unwrap();
    assert_eq!(review.retained_completion().unwrap(), None);
}

fn reject_duplicate(landing: &RecordLanding, id: &str) -> String {
    let mut record = task_record(id);
    record["task"]["source_sections"] = json!(["2", "2"]);
    let error = landing.land(record).unwrap_err().to_string();
    assert!(
        error.contains(id) && error.contains("lists '2' twice"),
        "{error}"
    );
    error
}
fn refused(landing: &RecordLanding, count: u64, earlier: &str) -> String {
    let error = landing
        .compose(&json!({"records_landed":count}))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(earlier),
        "must retain the exact rejection: {error}"
    );
    error
}

#[test]
fn record_landing_rejected_a_fixed_then_final_is_accepted() {
    let (_temp, landing) = open(RecordKind::Skeleton, &[]);
    reject_duplicate(&landing, "TASK-X-001");
    landing.land(task_record("TASK-X-001")).unwrap();
    assert_eq!(
        landing.compose(&json!({"records_landed":1})).unwrap()["tasks"][0]["task_id"],
        "TASK-X-001"
    );
}

#[test]
fn record_landing_rejected_a_then_final_repeats_a_and_duplicate() {
    let (_temp, landing) = open(RecordKind::Skeleton, &[]);
    let earlier = reject_duplicate(&landing, "TASK-X-001");
    refused(&landing, 0, &earlier);
}

#[test]
fn record_landing_rejected_a_then_b_landed_final_names_only_a() {
    let (_temp, landing) = open(RecordKind::Skeleton, &[]);
    let earlier = reject_duplicate(&landing, "TASK-X-001");
    landing.land(task_record("TASK-X-002")).unwrap();
    assert!(!refused(&landing, 1, &earlier).contains("TASK-X-002"));
}

#[test]
fn record_landing_patch_rejected_subject_cannot_hide_behind_seed() {
    let (_temp, landing) = open(RecordKind::Skeleton, &[]);
    let seed = json!({"schema_version":1,"acceptance_digest":"", "tasks":[task_record("TASK-X-001")["task"], task_record("TASK-X-002")["task"]]});
    landing.seed_skeleton(&seed, true).unwrap();
    let earlier = reject_duplicate(&landing, "TASK-X-001");
    refused(&landing, 2, &earlier);
    landing.land(task_record("TASK-X-001")).unwrap();
    landing.compose(&json!({"records_landed":2})).unwrap();
}

#[test]
fn record_landing_retained_completion_refuses_rejected_unlanded_subject() {
    let (_temp, landing) = open(RecordKind::Skeleton, &[]);
    let earlier = reject_duplicate(&landing, "TASK-X-001");
    landing.land(task_record("TASK-X-002")).unwrap();
    assert!(
        landing
            .retained_completion()
            .unwrap_err()
            .to_string()
            .contains(&earlier)
    );
}

#[test]
fn record_landing_never_fixed_is_never_silently_completed() {
    let (_temp, landing) = open(RecordKind::Skeleton, &[]);
    let earlier = reject_duplicate(&landing, "TASK-X-001");
    landing.land(task_record("TASK-X-002")).unwrap();
    // The runner retries two incomplete final replies then takes its existing
    // `incomplete landed artifact` failure path (or reaches its turn budget).
    // Its retained-completion fallback must never manufacture success here.
    for _ in 0..3 {
        refused(&landing, 1, &earlier);
        assert!(landing.retained_completion().is_err());
    }
}

#[test]
fn record_landing_out_of_call_review_or_verify_is_not_pending() {
    for kind in [RecordKind::Review, RecordKind::Verify] {
        let (_temp, landing) = open(kind, &["A"]);
        let mut record = json!({"subject":"other", "status":"failed",
            "evidence":[{"kind":"inspection","summary":"read"}]});
        assert!(
            landing
                .land(record.clone())
                .unwrap_err()
                .to_string()
                .contains("outside this call")
        );
        record["subject"] = json!("A");
        landing.land(record).unwrap();
        landing.compose(&json!({"records_landed":1})).unwrap();
    }
}

#[test]
fn record_landing_unknown_draft_remove_is_not_pending_p5_31() {
    let (_temp, landing) = open(RecordKind::Skeleton, &[]);
    assert!(
        landing
            .land(json!({"subject":"TASK-X-999", "remove":true}))
            .unwrap_err()
            .to_string()
            .contains("cannot remove unknown draft task")
    );
    landing.land(task_record("TASK-X-001")).unwrap();
    landing.compose(&json!({"records_landed":1})).unwrap();
}

#[test]
fn record_landing_rejected_a_deliberately_removed_clears_pending() {
    for retained in [false, true] {
        let (_temp, landing) = open(RecordKind::Skeleton, &[]);
        if retained {
            landing.land(task_record("TASK-X-001")).unwrap();
        }
        reject_duplicate(&landing, "TASK-X-001");
        landing
            .land(json!({"subject":"TASK-X-001", "remove":true}))
            .unwrap();
        landing.land(task_record("TASK-X-002")).unwrap();
        landing.compose(&json!({"records_landed":1})).unwrap();
        assert_eq!(
            landing.retained_completion().unwrap(),
            Some(json!({"records_landed":1}))
        );
    }
}

#[test]
fn record_landing_subject_task_id_mismatch_is_not_pending() {
    for frozen in [false, true] {
        let (_temp, landing) = open(RecordKind::Skeleton, &[]);
        if frozen {
            landing
                .seed_skeleton(
                    &json!({"schema_version":1,"acceptance_digest":"",
                "tasks":[task_record("TASK-X-001")["task"]]}),
                    true,
                )
                .unwrap();
        }
        let mut record = task_record("TASK-X-001");
        record["subject"] = json!("typo");
        let error = landing.land(record).unwrap_err().to_string();
        assert!(
            error.contains(if frozen {
                "preserve every frozen task_id"
            } else {
                "skeleton task identity mismatch"
            }),
            "{error}"
        );
        landing.land(task_record("TASK-X-001")).unwrap();
        landing.compose(&json!({"records_landed":1})).unwrap();
    }
}

#[test]
fn record_landing_rejected_a_still_bad_reports_newest_rejection_only() {
    let (_temp, landing) = open(RecordKind::Skeleton, &[]);
    let old = reject_duplicate(&landing, "TASK-X-001");
    let mut record = task_record("TASK-X-001");
    record["task"]["source_sections"] = json!(["9", "9"]);
    let newest = landing.land(record).unwrap_err().to_string();
    assert!(newest.contains("lists '9' twice"));
    let error = refused(&landing, 0, &newest);
    assert!(!error.contains(&old));
    landing.land(task_record("TASK-X-002")).unwrap();
    let hint = landing.hint().unwrap();
    assert!(hint.contains("Completion is pending") && hint.contains("TASK-X-001"));
    assert!(!hint.contains("your final reply is exactly"));
}

#[test]
fn record_landing_string_encoded_task_rejection_is_pending_until_fixed() {
    let (_temp, landing) = open(RecordKind::Skeleton, &[]);
    let mut record = task_record("TASK-X-001");
    record["task"]["source_sections"] = json!(["2", "2"]);
    record["task"] = json!(record["task"].to_string());
    let earlier = landing.land(record).unwrap_err().to_string();
    assert!(earlier.contains("lists '2' twice"));
    refused(&landing, 0, &earlier);
    landing.land(task_record("TASK-X-001")).unwrap();
    landing.compose(&json!({"records_landed":1})).unwrap();
}

#[test]
fn record_landing_pending_new_task_hint_matches_draft_and_frozen_modes() {
    for frozen in [false, true] {
        let (_temp, landing) = open(RecordKind::Skeleton, &[]);
        landing
            .seed_skeleton(
                &json!({"schema_version":1,"acceptance_digest":"",
            "tasks":[task_record("TASK-X-001")["task"]]}),
                frozen,
            )
            .unwrap();
        let error = reject_duplicate(&landing, "TASK-X-002");
        let hint = landing.hint().unwrap();
        assert!(hint.contains(&error), "{hint}");
        let removal = landing.land(json!({"subject":"TASK-X-002", "remove":true}));
        if frozen {
            assert!(!hint.contains("withdraw"), "{hint}");
            assert!(
                hint.contains("Re-land valid tasks under these subject ids"),
                "{hint}"
            );
            assert!(removal.is_err());
            refused(&landing, 1, &error);
            landing.land(task_record("TASK-X-002")).unwrap();
            landing.compose(&json!({"records_landed":2})).unwrap();
        } else {
            assert!(hint.contains("Correct or withdraw"), "{hint}");
            removal.unwrap();
            landing.compose(&json!({"records_landed":1})).unwrap();
        }
    }
}

#[test]
fn record_landing_reopened_call_keeps_original_change_count_and_missing_removals_do_not_add_work() {
    let (temp, landing) = open(RecordKind::Skeleton, &[]);
    let record = task_record("TASK-X-001");
    landing.land(record.clone()).unwrap();
    let reopened = RecordLanding::open(
        temp.path().join("records"),
        "identity".into(),
        RecordKind::Skeleton,
        vec![],
        false,
    )
    .unwrap();
    // B1/B2: reopening the same call preserves its original pre-call snapshot.
    reopened.require_new_landings(true).unwrap();
    reopened.land(record.clone()).unwrap();
    assert_eq!(reopened.new_landing_count().unwrap(), 1);
    assert!(reopened.compose_reply(&json!({"records_landed":1})).is_ok());
    assert!(
        reopened
            .land(json!({"subject":"TASK-X-002","remove":true}))
            .is_err()
    );
    assert_eq!(reopened.new_landing_count().unwrap(), 1);
    assert!(
        reopened
            .land(json!({"subject":"TASK-X-002","task":{
        "task_id":"TASK-X-002","file_name":"TASK-X-002.md"}}))
            .is_err()
    );
    reopened
        .land(json!({"subject":"TASK-X-002","remove":true}))
        .unwrap();
    assert_eq!(reopened.new_landing_count().unwrap(), 1);
    let mut changed = record;
    changed["task"]["source_sections"] = json!(["2"]);
    reopened.land(changed).unwrap();
    assert_eq!(reopened.new_landing_count().unwrap(), 1);
    reopened
        .land(json!({"subject":"TASK-X-001","remove":true}))
        .unwrap();
    assert_eq!(reopened.new_landing_count().unwrap(), 0);
}

#[test]
fn record_landing_repair_byte_changes_are_independent_of_finding_attribution() {
    let (_temp, landing) = open(RecordKind::Skeleton, &[]);
    let first = task_record("TASK-X-001");
    let second = task_record("TASK-X-002");
    landing.seed_skeleton(&json!({"schema_version":1,"acceptance_digest":"","tasks":[first["task"], second["task"]]}), false).unwrap();
    landing
        .require_finding_landings(&[
            json!({"task_ids":["TASK-X-001","TASK-X-002"]}),
            json!({"claim":"global repair"}),
        ])
        .unwrap();
    landing.land(first.clone()).unwrap();
    assert_eq!(
        landing.new_landing_count().unwrap(),
        0,
        "seed encoding cannot count as a repair"
    );
    assert!(landing.compose_reply(&json!({"records_landed":0})).is_err());
    let mut changed_first = first.clone();
    changed_first["task"]["source_sections"] = json!(["2"]);
    landing.land(changed_first).unwrap();
    landing.compose_reply(&json!({"records_landed":1})).unwrap();
    assert_eq!(
        landing.retained_reply().unwrap(),
        Some(json!({"records_landed":1}))
    );
    let mut changed_second = second;
    changed_second["task"]["source_sections"] = json!(["3"]);
    landing.land(changed_second).unwrap();
    landing.compose_reply(&json!({"records_landed":2})).unwrap();
    landing.land(first).unwrap();
    assert_eq!(
        landing.new_landing_count().unwrap(),
        1,
        "reverting a task removes its repair credit"
    );
    landing.compose_reply(&json!({"records_landed":1})).unwrap();
    landing.land(task_record("TASK-X-002")).unwrap();
    assert_eq!(landing.new_landing_count().unwrap(), 0);
    assert!(landing.compose_reply(&json!({"records_landed":0})).is_err());
}

#[test]
fn record_landing_non_skeleton_counts_changed_record_landings() {
    for kind in [RecordKind::Review, RecordKind::Verify] {
        let (_temp, landing) = open(kind, &["UNIT-1"]);
        let record = json!({"subject":"UNIT-1","status":"failed","evidence":[{"kind":"inspection","summary":"read source"}]});
        landing.land(record.clone()).unwrap();
        assert_eq!(landing.new_landing_count().unwrap(), 1);
        landing.land(record).unwrap();
        assert_eq!(landing.new_landing_count().unwrap(), 1);
    }
}

#[test]
fn record_landing_finding_coverage_belongs_to_the_gate() {
    let (_temp, landing) = open(RecordKind::Skeleton, &[]);
    let first = task_record("TASK-X-001");
    let second = task_record("TASK-X-002");
    landing.seed_skeleton(&json!({"schema_version":1,"acceptance_digest":"","tasks":[first["task"], second["task"]]}), false).unwrap();
    landing
        .require_finding_landings(&[
            json!({"task_id":"TASK-X-001","claim":"repair TASK-X-001"}),
            json!({"task_ids":["TASK-X-003"],"claim":"missing task"}),
        ])
        .unwrap();
    let mut changed = second;
    changed["task"]["source_sections"] = json!(["2"]);
    landing.land(changed).unwrap();
    assert!(landing.compose_reply(&json!({"records_landed":1})).is_ok());
    landing.land(task_record("TASK-X-002")).unwrap();
    assert!(landing.compose_reply(&json!({"records_landed":0})).is_err());
    landing
        .land(json!({"subject":"TASK-X-002","remove":true}))
        .unwrap();
    assert!(landing.compose_reply(&json!({"records_landed":1})).is_ok());
}

#[test]
fn record_landing_fresh_seed_identical_relanding_counts_zero() {
    let (_temp, landing) = open(RecordKind::Skeleton, &[]);
    let record = task_record("TASK-X-001");
    landing
        .seed_skeleton(
            &json!({"schema_version":1,"acceptance_digest":"","tasks":[record["task"]]}),
            false,
        )
        .unwrap();
    landing.land(record).unwrap();
    assert_eq!(landing.new_landing_count().unwrap(), 0);
    assert!(landing.compose_reply(&json!({"records_landed":0})).is_ok());
    assert!(landing.retained_reply().unwrap().is_none());
}
