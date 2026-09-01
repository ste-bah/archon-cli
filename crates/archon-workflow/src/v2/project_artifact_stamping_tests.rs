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

/// A repository deliverable must not be stamped with the project root.
///
/// Stamping resolved EVERY relative artifact requirement against the project
/// root, including repository source deliverables. A write item running in a
/// sealed worktree was therefore handed two incompatible instructions: its
/// `repository_root` was the worktree, and its required artifact was an
/// absolute path in the project tree. The agent satisfied both -- the only way
/// to obey -- and write-ownership then rejected it for touching the project
/// root. Both waves of run wf-0b0ccf0b died on this identically, and the
/// orphaned project-root file went on to break the retry with StaleBaseline.
///
/// The agent prompt already states the rule this restores: resolve `.archon/...`
/// paths under `project_artifact_root`, not `repository_root`.
#[test]
fn a_repository_deliverable_is_not_resolved_against_the_project_root() {
    let mut object = serde_json::Map::new();
    object.insert(
        "artifact_requirements".to_string(),
        serde_json::json!(["src/alpha.txt"]),
    );

    let resolved = super::stamp_project_artifact_paths(&mut object, "/project");

    assert!(
        resolved.is_empty(),
        "a repository source deliverable is not a project artifact: {resolved:?}"
    );
    assert_eq!(
        object.get("artifact_requirements"),
        Some(&serde_json::json!(["src/alpha.txt"])),
        "it must be left relative so it resolves against the item's own repository_root"
    );
}

/// A genuine project artifact is still stamped.
#[test]
fn a_dot_archon_artifact_is_still_resolved_against_the_project_root() {
    let mut object = serde_json::Map::new();
    object.insert(
        "artifact_requirements".to_string(),
        serde_json::json!([".archon/proof/report.json"]),
    );

    let resolved = super::stamp_project_artifact_paths(&mut object, "/project");

    assert_eq!(resolved.len(), 1, "{resolved:?}");
    assert_eq!(
        resolved[0].get("absolute_path").and_then(|v| v.as_str()),
        Some("/project/.archon/proof/report.json"),
        "{resolved:?}"
    );
}
