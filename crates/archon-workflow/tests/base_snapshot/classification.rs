use super::*;
use archon_workflow::baseline_results::{BaselineOutcome, BaselineResults};
use archon_workflow::task_set_contract::AcceptanceContract;
use archon_workflow::task_skeleton::TaskSkeleton;
use archon_workflow::v2::requirement_classes::{RequirementClass, classify};

pub(super) fn assert_class(
    snapshot: &BaseSnapshot,
    keys: &[&str],
    class: RequirementClass,
    reason: Option<&str>,
) {
    let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({
        "schema_version":1,"prd":{"path":"input","digest":"anchor"},"gap_policy":{},"supplementary":[],
        "acceptance":[{"id":"check","criterion":"item","check":{"kind":"command","command":"verify","cwd":"repo_root"},"covers":["item"],"judgment":{"verdict":"accepted","counterexample":"","reason":"","host_call_id":"test"}}]
    })).unwrap();
    let deliverables: Vec<_> = keys.iter().map(|key| serde_json::json!({"kind":"output","artifact_path":key,"min_instances":1,"implements":["item"],"artifact_format":"text"})).collect();
    let skeleton: TaskSkeleton = serde_json::from_value(serde_json::json!({
        "schema_version":1,"acceptance_digest":"anchor","tasks":[{"task_id":"task","file_name":"task","implements":["item"],"deliverable_contracts":deliverables}]
    })).unwrap();
    let results = BaselineResults::observed(
        Some(snapshot.commit.clone()),
        &[("check".into(), BaselineOutcome::Passed)].into(),
        &contract,
    );
    let entry = classify(&skeleton, &contract, &results.outcomes, |key| {
        let entry = &snapshot.entries[key];
        (
            matches!(entry, SnapshotEntry::Tracked { .. }),
            matches!(entry, SnapshotEntry::Unverified { .. }),
        )
    })
    .remove(0);
    assert_eq!(entry.class, class);
    assert_eq!(entry.reason.as_deref(), reason);
    assert_eq!(
        entry.passing_check_without_deliverable,
        class == RequirementClass::Missing
    );
    assert!(entry.text_item);
}
