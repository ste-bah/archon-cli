//! Each row checks classification against real shell status provenance.
use super::*;

fn check_status(script: &str, expected: bool) {
    let tree = tempfile::tempdir().unwrap();
    std::fs::write(tree.path().join("file.txt"), "ready").unwrap();
    let status = |script: &str| {
        std::process::Command::new("/bin/sh")
            .args(["-c", script])
            .current_dir(tree.path())
            .output()
            .unwrap()
            .status
    };
    let failed = status(script);
    let present = status(&script.replace("../absent", "."));
    if expected {
        assert!(
            !failed.success() && present.success(),
            "{script}: {failed}, {present}"
        );
    } else {
        assert_eq!(failed.code(), present.code(), "{script}");
    }
}

fn check(script: &str, expected: bool) {
    check_status(script, expected);
    assert_eq!(
        decisive(script, "cd", &["../absent".into()]),
        expected,
        "{script}"
    );
}

macro_rules! row {
    ($name:ident, $script:literal, $expected:literal) => {
        #[cfg(unix)]
        #[test]
        #[allow(non_snake_case)]
        fn $name() {
            check($script, $expected);
        }
    };
}
macro_rules! compound_row {
    ($name:ident, $script:literal, $expected:literal) => {
        #[test]
        #[allow(non_snake_case)]
        fn $name() {
            check_status($script, $expected);
            assert!(decisive($script, "cd", &["../absent".into()]));
        }
    };
}
// Input script -> expected: setup determines a failure exit (true), or masked (false).
row!(
    B3_bare_exit_preserves_failure,
    "cd ../absent || exit; true",
    true
);
row!(
    B3_nonzero_exit_preserves_failure,
    "cd ../absent || exit 7; true",
    true
);
row!(
    B3_false_fallback_preserves_failure,
    "cd ../absent || false",
    true
);
row!(
    B3_braced_exit_preserves_failure,
    "cd ../absent || { echo no; exit 7; }; true",
    true
);
#[test]
#[allow(non_snake_case)]
fn B3_return_preserves_failure() {
    // Top-level return differs between shells: no proof is safe on either.
    let tree = tempfile::tempdir().unwrap();
    let output = std::process::Command::new("/bin/sh")
        .args(["-c", "cd ../absent || return 7; true"])
        .current_dir(tree.path())
        .output()
        .unwrap();
    assert!(matches!(output.status.code(), Some(0 | 7)));
    assert!(decisive(
        "cd ../absent || return 7; true",
        "cd",
        &["../absent".into()]
    ));
}
row!(
    B3_negated_condition_exits_on_failure,
    "if ! cd ../absent; then exit 7; fi; true",
    true
);
row!(
    B3_final_and_list_preserves_failure,
    "cd ../absent && true",
    true
);
row!(
    B3_errexit_preserves_failure,
    "set -e; cd ../absent; true",
    true
);
row!(
    B3_true_masks_failure,
    "cd ../absent || true; test -f missing",
    false
);
row!(
    B3_colon_masks_failure,
    "cd ../absent || :; test -f missing",
    false
);
row!(
    B3_echo_then_test_masks_failure,
    "cd ../absent || echo missing; test -f missing",
    false
);
compound_row!(
    B3_if_then_later_test_masks_failure,
    "if cd ../absent; then true; fi; test -f missing",
    false
);
compound_row!(
    B3_else_test_runs_on_failure,
    "if cd ../absent; then test -f missing; else test -f missing; fi",
    false
);
compound_row!(
    B3_else_zero_masks_failure,
    "if cd ../absent; then true; else exit 0; fi",
    false
);

#[test]
#[allow(non_snake_case)]
fn S4_compounds_never_supply_setup_proof_and_keywords_are_whole_words() {
    // Input scripts -> real shell dependency; every unmodeled compound withholds proof.
    for (script, dependency) in [
        (
            "if cd ../absent; then test -f file.txt; else exit 1; fi",
            true,
        ),
        (
            "if ! cd ../absent; then echo 'missing file'; exit 1; fi; true",
            true,
        ),
        ("until cd ../absent; do exit 4; done; true", true),
        ("while ! cd ../absent; do exit 4; done; true", true),
        ("for x in one; do cd ../absent || exit 1; done", true),
        ("case x in x) cd ../absent || exit 1;; esac", true),
        ("(cd ../absent || exit 1)", true),
        ("f() { cd ../absent || exit 1; }; f", true),
        ("if cd ../absent; then true; else exit 0; fi", false),
    ] {
        check_status(script, dependency);
        assert!(decisive(script, "cd", &["../absent".into()]), "{script}");
    }
    for name in ["file", "elsewhere", "config"] {
        let script = format!("echo {name}; cd ../absent; true");
        check(&script, false);
    }
}

#[test]
#[allow(non_snake_case)]
fn N1_redirections_pipelines_groups_and_saved_status_withhold_proof() {
    // Input -> expected decisive verdict; real sh checks failure provenance.
    for script in [
        "cd ../absent && true 2>&1 | tail -1",
        "cd ../absent 2>&1 && true",
        "cd ../absent && true | cat",
        "{ cd ../absent && true; }",
        "cd ../absent && true; status=$?; exit $status",
        "cd ../absent && true; exit $?",
        "cd ../absent >&2 && true",
        "cd ../absent 0<&0 && true",
    ] {
        check(script, true);
    }
    // &> is a redirection in shells that support it, never a background split.
    assert_eq!(segments("cd X &>log && true")[0].1, "&&");
    for (script, expected) in [
        ("cd ../absent || exit 1; test -f file.txt", true),
        ("cd ../absent || true; test -f file.txt", false),
        ("cd ../absent && test -f file.txt", true),
        ("cd ../absent && test -f missing", true),
        ("cd ../absent && test -f fixture.txt", true),
        ("cd '../absent' && test -f file.txt", true),
        ("set -e; cd ../absent || true; false", false),
        ("set -e; cd ../absent || :; false", false),
        ("set -e; cd ../absent && true; false", false),
        ("cd ../absent; set -e; false", false),
    ] {
        // Some preserved #420 forms also fail with setup present.
        let tree = tempfile::tempdir().unwrap();
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", script])
            .current_dir(tree.path())
            .output()
            .unwrap();
        assert!(!output.status.success(), "{script}");
        assert_eq!(
            decisive(script, "cd", &["../absent".into()]),
            expected,
            "{script}"
        );
    }
    let script = "set -e; if cd ../absent; then true; fi; false";
    check_status(script, false);
    assert!(decisive(script, "cd", &["../absent".into()]));
}

#[test]
#[allow(non_snake_case)]
fn R1_status_reads_and_braces_belong_to_the_setup_list() {
    // Domain map: script -> whether setup can decide the real shell's exit.
    for (script, decisive) in [
        ("cd ../absent || true; test -f f; exit $?", false),
        ("{ test -f f; }; cd ../absent || true; test -f g", false),
        ("cd ../absent; test -f f; status=$?; exit $status", false),
        ("cd ../absent; test -f f; exit $?", false),
        ("cd ../absent && true 2>&1 | tail -1", true),
        ("cd ../absent 2>&1 && true", true),
        ("cd ../absent && true | cat", true),
        ("{ cd ../absent && true; }", true),
        ("cd ../absent && true; status=$?; exit $status", true),
        ("cd ../absent && true; exit $?", true),
        ("cd ../absent && true; status=$?; true; exit $status", true),
        ("cd ../absent; status=$?; status=0; exit $status", false),
        ("cd ../absent || exit 1; test -f file.txt", true),
        ("cd ../absent || true; test -f file.txt", false),
    ] {
        check(script, decisive);
    }
}
