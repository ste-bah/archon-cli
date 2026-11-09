use super::*;
#[test]
fn fileless_failure_requires_the_recorded_identity() {
    let root = tempfile::tempdir().unwrap();
    let before = include_str!("../../../../../tests/fixtures/diagnostic_outputs/go-subcases.txt");
    let after =
        include_str!("../../../../../tests/fixtures/diagnostic_outputs/pytest-parameters.txt");
    let stamp = BaselineStamp {
        pre_existing_diagnostics: vec![PreExistingCommand {
            command: "check".into(),
            files: vec![],
            repository_root: root.path().into(),
            identity: Some((Some(1), before.into())),
        }],
        ..Default::default()
    };
    assert!(stamp.pre_existing_diagnostics_cover("check", before, Some(1)));
    for (output, exit) in [(after, Some(1)), (before, Some(2)), ("", Some(1))] {
        assert!(!stamp.pre_existing_diagnostics_cover("check", output, exit));
    }
}
#[test]
fn file_tokens_resolve_from_the_recorded_repository_root() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("owned"), "content").unwrap();
    std::fs::write(root.path().join("other"), "content").unwrap();
    let stamp = BaselineStamp {
        pre_existing_diagnostics: vec![PreExistingCommand {
            command: "check owned".into(),
            files: vec!["owned".into()],
            repository_root: root.path().into(),
            identity: Some((Some(1), "error: old failure".into())),
        }],
        ..Default::default()
    };
    assert!(stamp.pre_existing_diagnostics_cover(
        "check owned",
        "error: old failure\nowned",
        Some(1)
    ));
    for output in ["other", "missing", ""] {
        assert!(
            !stamp.pre_existing_diagnostics_cover("check owned", output, Some(1)),
            "{output}"
        );
    }
}
#[test]
fn legacy_fileless_records_prove_no_failure_identity() {
    let entry: PreExistingCommand =
        serde_json::from_value(serde_json::json!({"command":"check", "files":[]})).unwrap();
    let stamp = BaselineStamp {
        pre_existing_diagnostics: vec![entry],
        ..Default::default()
    };
    for output in ["old failure", "new failure", ""] {
        assert!(!stamp.pre_existing_diagnostics_cover("check", output, Some(1)));
    }
}
