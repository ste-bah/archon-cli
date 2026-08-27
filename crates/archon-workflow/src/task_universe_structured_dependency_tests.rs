use super::*;

#[test]
fn structured_dependencies_survive_the_runtime_parser() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("TASK-DEMO-010-body.md");
    let raw = "# Body\n\n```yaml\ntask_id: TASK-DEMO-010\ntitle: Body\ncomplexity: medium\nstatus: ready\ndepends_on:\n  - task_id: TASK-DEMO-001\n    ordering_only: true\n  - task_id: TASK-DEMO-002\n    consumes:\n      - artifact_path: out.json\n        registry_records_field: records\nblocks: []\nimplements: [AC-DEMO-001]\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n## Acceptance Criteria\n- Done.\n";
    let task = parsing::parse_task_file(&path, raw).unwrap();
    assert_eq!(task.dependency_ids, vec!["TASK-DEMO-001", "TASK-DEMO-002"]);
    assert_eq!(task.dependencies.len(), 2);
    assert!(task.dependencies[0].ordering_only);
    assert_eq!(task.dependencies[1].consumes[0].artifact_path, "out.json");
    assert_eq!(
        task.dependencies[1].consumes[0]
            .registry_records_field
            .as_deref(),
        Some("records")
    );
}

#[test]
fn non_string_implements_fails_in_the_runtime_parser() {
    for value in ["null", "[{id: AC-DEMO-001}]", "[AC-DEMO-001, 7]"] {
        let error = parse_failure(&format!(
            "```yaml\ntask_id: TASK-DEMO-017\ntitle: Loud\ncomplexity: small\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: {value}\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n"
        ));
        assert!(error.contains("implements"), "{error}");
        assert!(error.contains("string or list of strings"), "{error}");
    }
}

#[test]
fn malformed_required_list_fields_fail_instead_of_becoming_empty() {
    for (field, value) in [
        ("blocks", "{task: TASK-DEMO-020}"),
        ("blocks", "[TASK-DEMO-020, 7]"),
        ("required_tools", "{runner: cargo}"),
        ("required_tools", "[cargo, 7]"),
        ("required_env_keys", "{key: API_TOKEN}"),
        ("required_env_keys", "[API_TOKEN, 7]"),
    ] {
        let raw = format!(
            "```yaml\ntask_id: TASK-DEMO-017\ntitle: Loud\ncomplexity: small\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n"
        )
        .replace(&format!("{field}: []"), &format!("{field}: {value}"));
        let error = parse_failure(&raw);
        assert!(error.contains(field), "{field}={value}: {error}");
        assert!(
            error.contains("string, list of strings, or null"),
            "{field}={value}: {error}"
        );
    }
}

#[test]
fn null_required_lists_are_explicit_empty_declarations() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("TASK-DEMO-017-body.md");
    let raw = "```yaml\ntask_id: TASK-DEMO-017\ntitle: Empty\ncomplexity: small\nstatus: ready\ndepends_on: []\nblocks: null\nimplements: []\nrequired_env_keys: null\nrequired_tools: null\ndeliverable_contracts: []\n```\n";
    let task = parsing::parse_task_file(&path, raw).unwrap();
    assert!(task.blocks_ids.is_empty());
    assert!(task.required_tools.is_empty());
    assert!(task.required_env_keys.is_empty());
}
