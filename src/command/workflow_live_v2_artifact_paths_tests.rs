use super::*;

#[test]
fn focused_verification_expands_unlisted_dot_archon_path() {
    let mut object = serde_json::json!({
        "focused_verification": "inspect .archon/data/report.json, then verify it"
    })
    .as_object()
    .cloned()
    .expect("object");

    stamp_project_artifact_paths(&mut object, "/project");

    assert_eq!(
        object["focused_verification"],
        "inspect /project/.archon/data/report.json, then verify it"
    );
}

#[test]
fn parent_traversal_is_not_stamped() {
    let mut object = serde_json::json!({
        "artifact_requirements": ["../outside.json"]
    })
    .as_object()
    .cloned()
    .expect("object");

    let resolved = stamp_project_artifact_paths(&mut object, "/project");

    assert!(resolved.is_empty());
    assert_eq!(object["artifact_requirements"][0], "../outside.json");
}

#[test]
fn absolute_dot_archon_path_is_not_prefixed_twice() {
    let mut object = serde_json::json!({
        "artifact_requirements": [".archon/data/report.json"],
        "focused_verification": "inspect .archon/data/report.json"
    })
    .as_object()
    .cloned()
    .expect("object");

    stamp_project_artifact_paths(&mut object, "/project");

    assert_eq!(
        object["focused_verification"],
        "inspect /project/.archon/data/report.json"
    );
}
