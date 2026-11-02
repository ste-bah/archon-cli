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
