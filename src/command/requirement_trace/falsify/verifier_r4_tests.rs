use super::*;

noted_case!(
    r4_falsify_pytest_expectation,
    "E AssertionError: expected FIXTURE_API_KEY environment variable is not set in stderr"
);
#[test]
fn r4_falsify_actual_after_expectation() {
    operator("r4_falsify_actual_after_expectation", true, |root| {
        let script = "echo 'E AssertionError: expected FIXTURE_API_KEY environment variable is not set in stderr'; echo 'FIXTURE_API_KEY environment variable is not set' >&2; exit 3";
        match run_script(root, script) {
            Ran::Finished {
                code: Some(3),
                success: false,
                output,
                note,
            } => {
                assert!(output.contains("E AssertionError: expected FIXTURE_API_KEY environment variable is not set in stderr"));
                assert!(output.contains("\nFIXTURE_API_KEY environment variable is not set"));
                assert!(note.unwrap().contains("Note:"));
            }
            _ => panic!("output changed the verifier result"),
        }
    });
}
noted_case!(
    r4_falsify_actual_diagnostic,
    "FIXTURE_API_KEY environment variable is not set"
);

fn baseline_note_case(case: &str, message: &str) {
    operator(case, true, |_| {
        let (dir, path) =
            super::super::super::tests::repo_with_committed_file("fn a() {}\nfn b() {}\n");
        let script = dir.path().join("verify.sh");
        std::fs::write(
            &script,
            format!("#!/bin/sh\nprintf '%s\\n' '{}'; exit 3\n", message),
        )
        .unwrap();
        let plan = super::super::super::tests::plan_for(
            dir.path(),
            &format!("/bin/sh {}", script.display()),
        );
        let before = std::fs::read(&path).unwrap();
        let refused =
            super::super::super::attempt(dir.path(), &plan).expect_err("baseline refusal");
        assert!(
            matches!(refused, RefusedToRun::BaselineDidNotPass { .. }),
            "{refused:?}"
        );
        assert!(
            refused.describe().contains("Note:") && refused.describe().contains("FIXTURE_API_KEY"),
            "{}",
            refused.describe()
        );
        let persisted = serde_json::to_string(&refused).unwrap();
        assert!(persisted.contains("Note:"));
        let decoded: RefusedToRun = serde_json::from_str(&persisted).unwrap();
        assert_eq!(decoded, refused);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!super::super::super::guard::backup_path(&path).exists());
    });
}
#[test]
fn r4_falsify_baseline_pytest_expectation() {
    baseline_note_case(
        "r4_falsify_baseline_pytest_expectation",
        "E AssertionError: expected FIXTURE_API_KEY environment variable is not set in stderr",
    );
}
#[test]
fn r4_falsify_baseline_actual_diagnostic() {
    baseline_note_case(
        "r4_falsify_baseline_actual_diagnostic",
        "FIXTURE_API_KEY environment variable is not set",
    );
}
#[test]
fn r4_falsify_baseline_quoted_expectation() {
    baseline_note_case(
        "r4_falsify_baseline_quoted_expectation",
        "expected \"FIXTURE_API_KEY is not set\" in stderr",
    );
}

fn mutant_note_case(case: &str, message: &str) {
    operator(case, true, |_| {
        let (dir, path) =
            super::super::super::tests::repo_with_committed_file("fn a() {}\nfn b() {}\n");
        let script = dir.path().join("verify.sh");
        std::fs::write(&script, format!("#!/bin/sh\nif grep -q unreachable src/a.rs; then printf '%s\\n' '{}'; exit 3; fi\nexit 0\n", message)).unwrap();
        let plan = super::super::super::tests::plan_for(
            dir.path(),
            &format!("/bin/sh {}", script.display()),
        );
        let before = std::fs::read(&path).unwrap();
        let outcome = super::super::super::attempt(dir.path(), &plan).unwrap();
        assert_eq!(
            outcome.level_after(archon_knowledge::traceability::ProofLevel::Exercised),
            archon_knowledge::traceability::ProofLevel::Falsifiable,
            "{outcome:?}"
        );
        assert!(outcome.describe().contains("Note:"), "{outcome:?}");
        let persisted = serde_json::to_string(&outcome).unwrap();
        assert!(persisted.contains("Note:"));
        let decoded: archon_knowledge::traceability::FalsificationOutcome =
            serde_json::from_str(&persisted).unwrap();
        assert_eq!(decoded, outcome);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!super::super::super::guard::backup_path(&path).exists());
    });
}
#[test]
fn r4_falsify_mutant_pytest_expectation() {
    mutant_note_case(
        "r4_falsify_mutant_pytest_expectation",
        "E AssertionError: expected FIXTURE_API_KEY environment variable is not set in stderr",
    );
}
#[test]
fn r4_falsify_mutant_actual_after_expectation() {
    mutant_note_case(
        "r4_falsify_mutant_actual_after_expectation",
        "E AssertionError: expected FIXTURE_API_KEY environment variable is not set in stderr\nFIXTURE_API_KEY environment variable is not set",
    );
}
#[test]
fn r4_falsify_mutant_json_failures() {
    mutant_note_case(
        "r4_falsify_mutant_json_failures",
        r#"{"status":"failed","failures":["FIXTURE_API_KEY environment variable is not set"]}"#,
    );
}
