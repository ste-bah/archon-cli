//! Issue 554: a shell check whose text the shell that runs it cannot parse
//! is its author's before any run, and a mutation run that stops at a shell
//! syntax error, or repeats with no exit status on unchanged text, is never
//! left the host's.

use archon_workflow::acceptance_scratch::CheckResult;
use archon_workflow::task_set_contract::TrustedCwd;

use super::check_fault_tests::{author_finding, prove_once, staged};
use super::probe_tests::trees;
use super::*;

/// The R11 shape: `trap ... EXIT` runs first, then a `{ ... fi` block that
/// no POSIX shell parses. Under bash's POSIX mode it exits 0, so it used to
/// pass on the base and leave its mutation run without an exit status.
const TRAP_THEN_PARSE_ERROR: &str = "set -eu\nlog=$(mktemp)\ntrap 'rm -f \"$log\"' EXIT\nf() {\n grep -q x data/state.txt || {\n  echo b\n fi\n}\nf\n";

#[tokio::test]
async fn a_check_its_shell_cannot_parse_goes_to_its_author_under_a_staged_freeze_554() {
    let trees = trees(&[("AC-X-001", TRAP_THEN_PARSE_ERROR, TrustedCwd::ProjectRoot)]);
    let finding = author_finding(staged(&trees).await, "AC-X-001");
    assert!(
        finding.contains("parse-only mode") && finding.contains("yntax error"),
        "{finding}"
    );
}

#[tokio::test]
async fn a_check_its_shell_cannot_parse_is_never_run_and_never_unproven_554() {
    let trees = trees(&[
        ("AC-X-001", TRAP_THEN_PARSE_ERROR, TrustedCwd::ProjectRoot),
        (
            "AC-X-002",
            "test -f data/state.txt",
            TrustedCwd::ProjectRoot,
        ),
    ]);
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    let unproven = probe.take_unproven();
    let finding = findings
        .get("AC-X-001")
        .unwrap_or_else(|| panic!("no author finding: {findings:?}; unproven={unproven:?}"));
    assert!(finding.contains("never ran it"), "{finding}");
    assert!(!unproven.contains_key("AC-X-001"), "{unproven:?}");
    // Its sibling is still probed on its own merits.
    assert!(!unproven.contains_key("AC-X-002"), "{unproven:?}");
    let can_fail = probe
        .prove_can_fail(&trees.contract(), &BTreeSet::from(["AC-X-001".to_string()]))
        .await;
    assert!(
        can_fail.findings["AC-X-001"].contains("parse-only mode"),
        "{can_fail:?}"
    );
}

/// One injected mutation run of `id` that recorded every input restored
/// but no exit status, with `stderr` after the host's moved marker.
fn unfinished_run(id: &str, markers: &mutation::Markers, stderr: &str) -> repairs::TreeRun {
    let result = CheckResult {
        verdict_streams: Default::default(),
        fault: None,
        classification: None,
        acceptance_id: id.into(),
        exit_code: Some(2),
        quota_walk_count: 0,
        stdout: format!("archon-mutation-state-{}: 0:restored\n", markers.nonce()).into_bytes(),
        stderr: format!("{}0\n{stderr}", markers.moved_prefix()).into_bytes(),
        operational_error: None,
        environment_note: None,
    };
    let evidence = hermetic::CheckEvidence {
        result,
        copy_layout: None,
    };
    repairs::TreeRun::known(&[evidence], "already observed")
}

const WRAPPER_SYNTAX: &str =
    "/bin/sh: line 24: syntax error near unexpected token `fi'\n/bin/sh: line 24: `  fi'\n";

#[tokio::test]
async fn a_mutation_run_that_cannot_parse_its_check_text_is_the_authors_at_once_554() {
    let trees = trees(&[("AC-X-001", TRAP_THEN_PARSE_ERROR, TrustedCwd::ProjectRoot)]);
    let markers = mutation::Markers::new();
    let run = unfinished_run("AC-X-001", &markers, WRAPPER_SYNTAX);
    let (findings, unproven) = prove_once(&trees, &markers, run).await;
    let finding = findings
        .get("AC-X-001")
        .unwrap_or_else(|| panic!("no author finding: {findings:?}; unproven={unproven:?}"));
    // The shell's own error for the check's text, at the check's own line.
    assert!(
        finding.contains("yntax error") && finding.contains(" 7:"),
        "{finding}"
    );
    assert!(!unproven.contains_key("AC-X-001"), "{unproven:?}");
}

#[tokio::test]
async fn a_mutation_run_stopped_by_a_shell_syntax_error_is_the_authors_at_once_554() {
    let trees = trees(&[(
        "AC-X-001",
        "test -f data/state.txt",
        TrustedCwd::ProjectRoot,
    )]);
    let markers = mutation::Markers::new();
    let run = unfinished_run("AC-X-001", &markers, WRAPPER_SYNTAX);
    let (findings, unproven) = prove_once(&trees, &markers, run).await;
    let finding = findings
        .get("AC-X-001")
        .unwrap_or_else(|| panic!("no author finding: {findings:?}; unproven={unproven:?}"));
    assert!(finding.contains("line 24: syntax error"), "{finding}");
    assert!(finding.contains("mutation wrapper"), "{finding}");
}

/// The strike rule: no exit status and no syntax error may be the host's
/// the first time; the same unchanged check failing so again is its
/// author's.
#[tokio::test]
async fn a_mutation_run_with_no_exit_status_is_struck_then_goes_to_its_author_554() {
    let trees = trees(&[(
        "AC-X-001",
        "test -f data/state.txt",
        TrustedCwd::ProjectRoot,
    )]);
    let markers = mutation::Markers::new();
    let (findings, unproven) =
        prove_once(&trees, &markers, unfinished_run("AC-X-001", &markers, "")).await;
    assert!(
        findings.is_empty(),
        "the first may be the host's: {findings:?}"
    );
    assert!(unproven.contains_key("AC-X-001"), "{unproven:?}");
    let (findings, unproven) =
        prove_once(&trees, &markers, unfinished_run("AC-X-001", &markers, "")).await;
    let finding = findings.get("AC-X-001").unwrap_or_else(|| {
        panic!("the same failure again is the author's: {findings:?}; unproven={unproven:?}")
    });
    assert!(
        finding.contains("did not record its exit status"),
        "{finding}"
    );
}
