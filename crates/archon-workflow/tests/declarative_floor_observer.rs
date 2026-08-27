use std::collections::BTreeMap;

use archon_workflow::task_universe::WorkflowV2DeliverableContract;
use archon_workflow::{
    DeclarativeFloorEvaluation, DeclarativeFloorFacts, collect_declarative_floor_facts,
    evaluate_declarative_floor,
};

fn contract() -> WorkflowV2DeliverableContract {
    WorkflowV2DeliverableContract {
        kind: "matrix".into(),
        artifact_path: "out.json".into(),
        required_universe: true,
        universe_fields: vec!["axes.names".into()],
        cells_field: Some("cells".into()),
        cell_identity_fields: vec!["name".into()],
        required_true_fields: vec!["ready".into()],
        required_nonempty_fields: vec!["path".into()],
        positive_count_fields: vec!["count".into()],
        minimum_count_fields: BTreeMap::from([("count".into(), 2)]),
        ..WorkflowV2DeliverableContract::default()
    }
}

#[test]
fn commandless_floor_passes_from_typed_facts() {
    let evaluation = evaluate_declarative_floor(
        &contract(),
        &DeclarativeFloorFacts {
            artifact_present: true,
            artifact_byte_len: 10,
            artifact_json: Some(serde_json::json!({
                "axes": {"names": ["one", "two"]},
                "cells": [
                    {"name": "one", "ready": true, "path": "a", "count": 2},
                    {"name": "two", "ready": true, "path": "b", "count": 3}
                ]
            })),
            registry_json: None,
            instance_count: 1,
        },
    );
    assert_eq!(evaluation, DeclarativeFloorEvaluation::Passed);
}

#[test]
fn commandless_floor_reports_exact_declared_predicate_failures() {
    let evaluation = evaluate_declarative_floor(
        &contract(),
        &DeclarativeFloorFacts {
            artifact_present: true,
            artifact_byte_len: 10,
            artifact_json: Some(serde_json::json!({
                "axes": {"names": ["one"]},
                "cells": [{"name": "one", "ready": false, "path": "", "count": 1}]
            })),
            registry_json: None,
            instance_count: 1,
        },
    );
    let DeclarativeFloorEvaluation::Failed { findings } = evaluation else {
        panic!("expected failed floor")
    };
    assert!(
        findings
            .iter()
            .any(|finding| finding.contains("required true"))
    );
    assert!(
        findings
            .iter()
            .any(|finding| finding.contains("required non-empty"))
    );
    assert!(
        findings
            .iter()
            .any(|finding| finding.contains("below declared minimum"))
    );
}

#[test]
fn typed_command_is_deferred_without_exposing_executable_text() {
    let mut contract = contract();
    contract.typed_verifier_command = Some("secret-tool --token never-run".into());
    let evaluation = evaluate_declarative_floor(
        &contract,
        &DeclarativeFloorFacts {
            artifact_present: true,
            artifact_byte_len: 1,
            artifact_json: Some(serde_json::json!({})),
            registry_json: None,
            instance_count: 1,
        },
    );
    let DeclarativeFloorEvaluation::Deferred { reason } = evaluation else {
        panic!("expected deferred floor")
    };
    assert!(reason.contains("deferred in R2a"));
    assert!(!reason.contains("secret-tool"));
    assert!(!reason.contains("never-run"));
}

fn shell_verifier_passes(root: &std::path::Path, contract: &WorkflowV2DeliverableContract) -> bool {
    let value = serde_json::to_value(contract).expect("contract json");
    let script = archon_workflow::v2::deliverable_contract::verification_command(
        root.to_str().expect("root"),
        &value,
    );
    let mut child = std::process::Command::new(archon_shell::resolve_posix_shell())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn verifier");
    use std::io::Write as _;
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("write verifier");
    child
        .wait_with_output()
        .expect("verifier output")
        .status
        .success()
}

fn rust_floor_passes(root: &std::path::Path, contract: &WorkflowV2DeliverableContract) -> bool {
    let facts = collect_declarative_floor_facts(root, contract).expect("collect facts");
    matches!(
        evaluate_declarative_floor(contract, &facts),
        DeclarativeFloorEvaluation::Passed
    )
}

#[test]
fn shared_floor_matches_existing_verifier_for_present_and_missing_text() {
    let project = tempfile::tempdir().expect("project");
    let path = project.path().join("artifacts/report.md");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    std::fs::write(&path, "verified prose\n").expect("artifact");
    let contract = WorkflowV2DeliverableContract {
        kind: "report".into(),
        artifact_path: "artifacts/report.md".into(),
        artifact_format: Some("TEXT".into()),
        ..WorkflowV2DeliverableContract::default()
    };

    assert!(shell_verifier_passes(project.path(), &contract));
    assert!(rust_floor_passes(project.path(), &contract));

    std::fs::remove_file(path).expect("remove artifact");
    assert!(!shell_verifier_passes(project.path(), &contract));
    assert!(!rust_floor_passes(project.path(), &contract));
}

#[test]
fn shared_floor_matches_existing_verifier_for_json_and_registry_predicates() {
    let project = tempfile::tempdir().expect("project");
    std::fs::create_dir_all(project.path().join("artifacts")).expect("dir");
    let artifact = project.path().join("artifacts/matrix.json");
    let registry = project.path().join("artifacts/registry.json");
    std::fs::write(
        &artifact,
        serde_json::to_vec(&serde_json::json!({
            "axes": {"names": ["One"]},
            "cells": [{"name": "One", "ready": true, "path": "a", "count": 2}]
        }))
        .expect("artifact json"),
    )
    .expect("artifact");
    std::fs::write(
        &registry,
        serde_json::to_vec(&serde_json::json!({
            "records": {"One": {"ready": true, "status": "complete", "count": 2, "name": "One"}}
        }))
        .expect("registry json"),
    )
    .expect("registry");
    let mut contract = contract();
    contract.artifact_path = "artifacts/matrix.json".into();
    contract.registry_path = Some("artifacts/registry.json".into());
    contract.registry_records_field = Some("records".into());
    contract.registry_key_fields = vec!["name".into()];
    contract.registry_required_true_fields = vec!["ready".into()];
    contract.registry_status_field = Some("status".into());
    contract.registry_allowed_statuses = vec!["complete".into()];
    contract.registry_count_field = Some("count".into());
    contract.registry_minimum_count = 2;
    contract.registry_identity_fields = BTreeMap::from([("name".into(), "name".into())]);

    assert!(shell_verifier_passes(project.path(), &contract));
    assert!(rust_floor_passes(project.path(), &contract));

    std::fs::write(&registry, r#"{"records":{}}"#).expect("broken registry");
    assert!(!shell_verifier_passes(project.path(), &contract));
    assert!(!rust_floor_passes(project.path(), &contract));
}

#[test]
fn malformed_json_has_the_same_failed_floor_in_both_paths() {
    let project = tempfile::tempdir().expect("project");
    std::fs::write(project.path().join("out.json"), "not json").expect("artifact");
    let contract = WorkflowV2DeliverableContract {
        kind: "json".into(),
        artifact_path: "out.json".into(),
        ..WorkflowV2DeliverableContract::default()
    };

    assert!(!shell_verifier_passes(project.path(), &contract));
    assert!(!rust_floor_passes(project.path(), &contract));
}
