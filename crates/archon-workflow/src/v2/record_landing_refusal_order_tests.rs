//! Issue-630: a refusal never asks for a reply that a later check refuses.
//! Pending-work refusals (open findings, empty store) come before the count
//! refusal, so the live R18 author-3 shape (repair call, 0 new landings,
//! reply `{"records_landed":19}`) is told to land a change, not to reply 0.
use super::record_landing::{RecordKind, RecordLanding};
use serde_json::{Value, json};

fn skeleton() -> (tempfile::TempDir, RecordLanding) {
    let temp = tempfile::tempdir().unwrap();
    let landing = RecordLanding::open(
        temp.path().join("records"),
        "identity".into(),
        RecordKind::Skeleton,
        Vec::new(),
        false,
    )
    .unwrap();
    (temp, landing)
}

fn task(id: &str) -> Value {
    json!({"task_id":id,"file_name":format!("{id}.md"),"implements":["REQ-1"]})
}

#[test]
fn repair_call_with_no_landing_is_told_to_land_not_to_reply_zero() {
    let (_temp, landing) = skeleton();
    let seed = json!({"schema_version":1,"acceptance_digest":"","tasks":[task("TASK-X-001")]});
    landing.seed_skeleton(&seed, false).unwrap();
    landing
        .require_finding_landings(&[json!({"task_ids":["TASK-X-001"]})])
        .unwrap();
    let error = landing
        .compose_reply(&json!({"records_landed":19}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("byte-changing landing"), "{error}");
    assert!(!error.contains("reply must be exactly"), "{error}");
}

#[test]
fn empty_store_with_a_wrong_count_is_told_to_land_a_task() {
    let (_temp, landing) = skeleton();
    let error = landing
        .compose_reply(&json!({"records_landed":5}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("task store is empty"), "{error}");
    assert!(!error.contains("reply must be exactly"), "{error}");
}
