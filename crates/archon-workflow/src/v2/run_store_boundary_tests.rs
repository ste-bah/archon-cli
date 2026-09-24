use super::is_run_store_path;

#[test]
fn the_store_itself_and_everything_under_it_is_the_store() {
    assert!(is_run_store_path(".archon/workflows"));
    assert!(is_run_store_path(".archon/workflows/"));
    assert!(is_run_store_path(".archon/workflows/some-run"));
    assert!(is_run_store_path(
        ".archon/workflows/some-run/v2/branches/b/x.json"
    ));
    // A run-prefixed file beside the run directories: a host record too, and
    // one the artifact-root prefix test never matched.
    assert!(is_run_store_path(".archon/workflows/some-run-report.json"));
}

#[test]
fn the_project_artifact_area_and_ordinary_source_are_not_the_store() {
    assert!(!is_run_store_path(".archon/artifacts/out.json"));
    assert!(!is_run_store_path(".archon/workflowsomething/x"));
    assert!(!is_run_store_path("crates/thing/src/lib.rs"));
    assert!(!is_run_store_path("docs/report.md"));
}
