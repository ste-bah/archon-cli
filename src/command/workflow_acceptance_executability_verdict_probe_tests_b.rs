//! Issue 328 round 2 through a real probe on real trees: the verdict is
//! decided from the check's own text and search path, never from stderr
//! alone, and a check that stays unproven on the same base goes to its
//! author instead of blocking the task set for good.

use super::probe_tests::trees;
use super::verdict_probe_tests::{BUILDS_THEN_ASSERTS, REPO, commit, round_at};
use super::{ExecutabilityProbe, HostProbe};

/// The probe's verdicts for `checks` on the tree `files` make, committed on
/// top of the fixture: (findings, unproven, ids that failed there).
async fn on_base(
    checks: &[(&str, &str)],
    files: &[(&str, &str)],
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let with_cwd: Vec<_> = checks.iter().map(|(id, c)| (*id, *c, REPO)).collect();
    let trees = trees(&with_cwd);
    for (path, _) in files {
        if let Some(parent) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(trees.repo.join(parent)).unwrap();
        }
    }
    let base = commit(&trees.repo, files, "base");
    let copies = tempfile::tempdir().unwrap();
    let probe = round_at(&trees, &base, copies.path());
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    let unproven = probe.take_unproven();
    let failed = probe.take_baseline_runs().expect("a baseline").failures;
    trees.assert_live_untouched(copies.path());
    (
        findings.into_keys().collect(),
        unproven
            .into_iter()
            .map(|(id, why)| format!("{id}: {why}"))
            .collect(),
        failed.into_keys().collect(),
    )
}

/// (a) A script of the tree not there yet, its stderr discarded: exit 127
/// with no output is the deliverable's absence, a proof.
#[tokio::test]
async fn a_missing_script_of_the_tree_is_a_proof_even_with_its_stderr_discarded() {
    let check = "bash scripts/new.sh 2>/dev/null";
    let (findings, unproven, failed) = on_base(&[("AC-A", check)], &[("README", "x\n")]).await;
    assert_eq!((findings, unproven), (vec![], vec![]));
    assert_eq!(failed, vec!["AC-A"]);
}

/// (b) A check that runs no build tool never failed on a build: a grep
/// match shaped like `path:line: error` is its own verdict.
#[tokio::test]
async fn a_grep_match_shaped_like_a_compile_error_is_a_proof() {
    let check = "! grep -rn 'error!' src";
    let files = [("src/log.rs", "fn f() {\n    error!(\"x\");\n}\n")];
    let (findings, unproven, failed) = on_base(&[("AC-B", check)], &files).await;
    assert_eq!((findings, unproven), (vec![], vec![]));
    assert_eq!(failed, vec!["AC-B"]);
}

/// (c) When the build is the check's assertion, a base that does not build
/// is the verdict it asserts.
#[tokio::test]
async fn a_check_whose_assertion_is_the_build_is_proven_by_a_broken_base() {
    let check = "rustc --crate-type lib --emit=metadata --out-dir \"$(mktemp -d)\" src/lib.rs";
    let files = [
        ("src/lib.rs", "mod parser;\n"),
        ("src/parser.rs", "pub fn f() -> u32 { \"x\" }\n"),
    ];
    let (findings, unproven, failed) = on_base(&[("AC-C", check)], &files).await;
    assert_eq!((findings, unproven), (vec![], vec![]));
    assert_eq!(failed, vec!["AC-C"]);
}

/// TDD: the base already holds the check's own test, which calls code the
/// implementation adds; its compile errors (E0432, E0425) are a proof.
#[tokio::test]
async fn a_base_test_calling_missing_code_is_a_proof() {
    let check = "CARGO_TARGET_DIR=\"$(mktemp -d)\" cargo test --offline -q --test tdd";
    let files = [
        (".gitignore", "Cargo.lock\n"),
        (
            "Cargo.toml",
            "[package]\nname = \"lake\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
        ),
        ("src/lib.rs", "pub fn present() {}\n"),
        (
            "tests/tdd.rs",
            "use lake::missing;\n\n#[test]\nfn t() {\n    missing();\n    lake::absent();\n}\n",
        ),
    ];
    let (findings, unproven, failed) = on_base(&[("AC-T", check)], &files).await;
    assert_eq!((findings, unproven), (vec![], vec![]));
    assert_eq!(failed, vec!["AC-T"]);
}

/// The round-1 shape stays unproven: a build-then-assert check whose base
/// fails to compile outside its own files.
#[tokio::test]
async fn a_build_step_broken_outside_the_checks_files_is_still_unproven() {
    let files = [
        ("src/lib.rs", "mod parser;\n"),
        ("src/parser.rs", "pub fn f() -> u32 { \"x\" }\n"),
    ];
    let (findings, unproven, failed) = on_base(&[("AC-U", BUILDS_THEN_ASSERTS)], &files).await;
    assert!(
        findings.is_empty() && failed.is_empty(),
        "{findings:?} {failed:?}"
    );
    assert!(
        unproven.len() == 1 && unproven[0].contains("did not build"),
        "{unproven:?}"
    );
}

/// (d) Unproven once, the check is retried; unproven again on the same base,
/// it is its author's finding -- never a block for good. A re-authored text
/// starts afresh.
#[tokio::test]
async fn a_check_unproven_again_on_the_same_base_goes_to_its_author() {
    let trees = trees(&[("AC-D", "archon-issue-328-absent-tool --verify x", REPO)]);
    let copies = tempfile::tempdir().unwrap();
    let freeze = || {
        HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
            .with_copy_parent(copies.path().to_path_buf())
    };
    let first = freeze();
    let findings = first.script_defects(&trees.contract(), &trees.ids()).await;
    assert!(findings.is_empty(), "{findings:?}");
    assert!(first.take_unproven().contains_key("AC-D"));
    let second = freeze();
    let findings = second.script_defects(&trees.contract(), &trees.ids()).await;
    assert!(second.take_unproven().is_empty(), "no longer the host's");
    let finding = findings.get("AC-D").expect("the author's finding");
    assert!(
        finding.contains("cannot be proven on the base commit")
            && finding.contains("fail by its own assertion")
            && finding.contains("archon-issue-328-absent-tool"),
        "{finding}"
    );
    let mut rewritten = trees.contract();
    if let archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } =
        &mut rewritten.acceptance[0].check
    {
        command.push_str(" # re-authored");
    }
    let third = freeze();
    let findings = third.script_defects(&rewritten, &trees.ids()).await;
    assert!(findings.is_empty(), "{findings:?}");
    assert!(
        third.take_unproven().contains_key("AC-D"),
        "a new text starts afresh"
    );
    trees.assert_live_untouched(copies.path());
}
