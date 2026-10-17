use super::*;

/// Malformed YAML used to be swallowed into an empty mapping.
#[test]
fn unparseable_yaml_fails_instead_of_becoming_an_empty_mapping() {
    let error = parse_failure("```yaml\ntask_id: [unclosed\n```\n");
    assert!(
        error.contains("could not parse the task YAML block"),
        "{error}"
    );
    assert!(error.contains("TASK-DEMO-017-loud.md"), "{error}");
}

#[test]
fn malformed_task_body_has_a_typed_error_distinct_from_workflow_setup() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("TASK-DEMO-017.md");
    let body = "not a task body\n";
    fs::write(&path, body).expect("write task");
    let error = parse_task_file(&path, body).expect_err("body syntax is invalid");
    assert!(
        matches!(error, crate::WorkflowError::TaskBodyInvalid(_)),
        "{error}"
    );
    assert!(error.to_string().contains("no fenced"));
}

#[test]
fn an_unterminated_yaml_fence_is_not_a_block() {
    let error = parse_failure("# Loud\n\n```yaml\ntask_id: TASK-DEMO-017\n");
    assert!(error.contains("no fenced ```yaml task block"), "{error}");
}
