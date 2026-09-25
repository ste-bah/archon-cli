//! The run store admits only what the artifact rules name for this run.

use super::super::{WorkflowV2ProjectArtifactContext, project_artifact_context_from_v2_root};
use super::project_artifact_write_admitted;
use serde_json::json;

struct Project {
    _temp: tempfile::TempDir,
    root: std::path::PathBuf,
}

fn context_for(run_id: &str) -> (Project, WorkflowV2ProjectArtifactContext) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let v2_root = root.join(".archon/workflows").join(run_id).join("v2");
    std::fs::create_dir_all(&v2_root).unwrap();
    let context = project_artifact_context_from_v2_root(&v2_root);
    (Project { _temp: temp, root }, context)
}

fn admitted(project: &Project, context: &WorkflowV2ProjectArtifactContext, rel: &str) -> bool {
    project_artifact_write_admitted(context, &project.root.join(rel).display().to_string())
}

/// The live shape: a branch wrote its audit report beside the run directory,
/// named with the run's id — admitted by the run-prefix rule.
#[test]
fn the_wf139_run_prefixed_report_is_admitted_and_its_neighbours_are_not() {
    let fixture: serde_json::Value = serde_json::from_str(
        archon_test_support::fixtures::WF139_PROJECT_ARTIFACT_WRITE_FALSE_SAFETY,
    )
    .unwrap();
    let run_id = fixture["run_id"].as_str().unwrap();
    let reported = fixture["reported_changed_file"].as_str().unwrap();
    let (project, context) = context_for(run_id);

    assert!(admitted(&project, &context, reported), "{reported}");
    assert!(
        project_artifact_write_admitted(&context, reported),
        "relative spelling"
    );
    for refused in [
        format!(".archon/workflows/{run_id}/v2/branches/b/record.json"),
        format!(".archon/workflows/{run_id}/state.json"),
        format!(".archon/workflows/{run_id}-nested/report.json"),
        ".archon/workflows/wf-another-run-report.json".to_string(),
        ".archon/workflows/state.json".to_string(),
    ] {
        assert!(!admitted(&project, &context, &refused), "{refused}");
    }
}

#[test]
fn a_requirement_inside_the_run_admits_its_path_not_the_run_directory() {
    let (project, mut context) = context_for("run-1");
    context.add_artifact_requirements(&json!({
        "artifact_requirements": [".archon/workflows/run-1/reports/summary.md"]
    }));

    assert!(
        !context
            .artifact_roots
            .iter()
            .any(|root| root.starts_with(".archon/workflows/run-1/reports")
                || root == ".archon/workflows/run-1"),
        "{:?}",
        context.artifact_roots
    );
    assert!(admitted(
        &project,
        &context,
        ".archon/workflows/run-1/reports/summary.md"
    ));
    assert!(!admitted(
        &project,
        &context,
        ".archon/workflows/run-1/reports/other.md"
    ));
    assert!(!admitted(
        &project,
        &context,
        ".archon/workflows/run-1/state.json"
    ));
}

#[test]
fn a_requirement_directly_under_archon_does_not_open_the_store() {
    let (project, mut context) = context_for("run-1");
    context.add_artifact_requirements(&json!({ "artifact_requirements": [".archon/report.md"] }));

    assert!(
        !context.artifact_roots.iter().any(|root| root == ".archon"),
        "{:?}",
        context.artifact_roots
    );
    assert!(admitted(&project, &context, ".archon/report.md"));
    assert!(!admitted(
        &project,
        &context,
        ".archon/workflows/run-1/v2/branches/b/record.json"
    ));
}

#[test]
fn a_requirement_in_another_run_admits_nothing() {
    let (project, mut context) = context_for("run-1");
    context.add_artifact_requirements(&json!({
        "artifact_requirements": [".archon/workflows/run-0/reports/summary.md"]
    }));

    assert!(!admitted(
        &project,
        &context,
        ".archon/workflows/run-0/reports/summary.md"
    ));
}

#[test]
fn a_requirement_outside_the_store_still_admits_its_directory() {
    let (project, mut context) = context_for("run-1");
    context.add_artifact_requirements(&json!({
        "artifact_requirements": [".archon/reports/summary.md"]
    }));

    assert!(
        context
            .artifact_roots
            .iter()
            .any(|root| root == ".archon/reports"),
        "{:?}",
        context.artifact_roots
    );
    assert!(admitted(&project, &context, ".archon/reports/other.md"));
}
