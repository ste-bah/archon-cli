use archon_workflow::task_universe::WorkflowV2DeliverableContract;
use archon_workflow::verifier_strength::verifier_strength_defect;

fn contract(path: &str, min_instances: usize) -> WorkflowV2DeliverableContract {
    WorkflowV2DeliverableContract {
        kind: "artifact".into(),
        artifact_path: path.into(),
        min_instances,
        ..WorkflowV2DeliverableContract::default()
    }
}

#[test]
fn commands_that_cannot_fail_are_refused_but_predicates_are_allowed() {
    let cases = [
        ("test -f out.json", Some("out.json"), true),
        ("bash -c 'test -f out.json'", Some("out.json"), true),
        ("sh -lc \"test -f out.json\"", Some("out.json"), true),
        ("`test -f out.json`", Some("out.json"), true),
        ("test -f {artifact_path}", Some("out.json"), true),
        ("test -f {artifact_path};", Some("out.json"), true),
        ("test -f {artifact_path} && echo ok", Some("out.json"), true),
        (
            "test -f {artifact_path} && grep -q required {artifact_path}",
            Some("out.json"),
            false,
        ),
        ("test -s out.json", Some("out.json"), true),
        ("test -d out.json/", Some("out.json"), true),
        ("ls out.json", Some("out.json"), true),
        ("stat out.json", Some("out.json"), true),
        ("ls out.json.backup-*", Some("out.json.backup"), false),
        ("grep -q required out.json", Some("out.json"), false),
        (
            "bash -c 'grep -q required out.json'",
            Some("out.json"),
            false,
        ),
        ("true", None, true),
        (":", None, true),
        ("sh -c 'exit 0'", None, true),
        ("cargo --version", None, true),
        ("cargo -Vv", None, true),
        ("cargo --version && echo done", None, true),
        ("cargo --version && grep -q required out.json", None, false),
        ("python -c 'print(1)'", None, true),
        ("echo done", None, true),
        ("printf done", None, true),
        ("yes | head -1", None, true),
        ("find . | wc -l", None, true),
        ("probe || true", None, true),
        ("probe || :", None, true),
        ("jq -e '.ready == true' out.json; true", None, true),
        ("jq -e '.ready == true' out.json; :", None, true),
        ("jq -e '.ready == true' out.json; true;", None, true),
        ("jq -e '.ready == true' out.json; :;", None, true),
        ("jq -e '.ready == true' out.json; echo ok", None, true),
        ("jq -e '.ready == true' out.json; echo ok;", None, true),
        (
            "jq -e '.ready == true' out.json; grep -q required out.json",
            None,
            false,
        ),
        ("jq -e '.ready == true' out.json", None, false),
    ];

    for (command, own_artifact, refused) in cases {
        let defect = verifier_strength_defect(Some(command), own_artifact, None);
        assert_eq!(
            defect.is_some(),
            refused,
            "command: {command}; defect: {defect:?}"
        );
    }
}

#[test]
fn verifier_or_positive_instance_obligation_is_mandatory() {
    let no_floor = contract("out.json", 0);
    let positive_floor = contract("out.json", 1);

    let missing = verifier_strength_defect(None, Some("out.json"), Some(&no_floor))
        .expect("missing verifier without floor must be refused");
    assert!(
        missing
            .to_string()
            .contains("deleting the verifier does not satisfy")
    );
    assert!(
        verifier_strength_defect(None, Some("out.json"), Some(&positive_floor)).is_none(),
        "a positive floor is a falsifiable execution obligation"
    );
}

#[test]
fn own_artifact_existence_finding_demands_replacement_not_deletion() {
    let defect = verifier_strength_defect(Some("test -f out.json"), Some("out.json"), None)
        .expect("duplicate existence test must be refused")
        .to_string();
    assert!(defect.contains("replace"), "{defect}");
    assert!(
        defect.contains("deleting the verifier does not satisfy"),
        "{defect}"
    );
}
