use super::*;

fn entry(command: &str) -> AcceptanceCriterion {
    serde_json::from_value(serde_json::json!({
        "id": "AC-TEST-001",
        "criterion": "the command checks its input",
        "check": {"kind": "command", "command": command, "cwd": "project_root"},
        "judgment": {"verdict": "accepted", "counterexample": "", "reason": "", "host_call_id": "test"}
    }))
    .unwrap()
}

#[test]
fn variable_values_follow_directory_flow_and_joins() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("a")).unwrap();
    std::fs::write(project.path().join("a/data"), "x\n").unwrap();
    let cases = [
        ("D=a; grep x \"$D/data\"", true),
        ("if true; then D=a; else D=a; fi; grep x \"$D/data\"", true),
        ("if true; then D=a; else D=b; fi; grep x \"$D/data\"", false),
        ("( D=a ); grep x \"$D/data\"", false),
        ("grep x \"$FROM_ENV/a/data\"", false),
        ("if false; then D=a; fi; cd \"$D\"; grep x data", false),
        ("if D=a; then true; fi; grep x \"$D/data\"", false),
        ("if true; then D=a; fi; grep x \"$D/data\"", false),
        ("true && D=a; grep x \"$D/data\"", false),
        ("false || D=a; grep x \"$D/data\"", false),
        ("for item in one; do D=a; done; grep x \"$D/data\"", false),
        ("D=a; unset D; grep x \"$D/data\"", false),
    ];
    for (command, expected) in cases {
        let found = !named_inputs(&entry(command), project.path(), project.path()).is_empty();
        assert_eq!(found, expected, "{command}");
    }
}
