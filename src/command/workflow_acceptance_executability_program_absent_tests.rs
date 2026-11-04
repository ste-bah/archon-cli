//! Issue 557: a check that starts a program neither its site nor the host
//! has goes to its author at once on the freeze probe path, as a site that
//! lacks a program the host has does (#548) and as the re-author path does.
use super::super::probe_tests::trees;
use super::*;
use archon_workflow::task_set_contract::TrustedCwd;

/// A heredoc whose terminator is repeated after the heredoc closed: the
/// shell accepts the stray word as a command (`sh -n` passes) and exits 127.
const STRAY_TERMINATOR: &str = "set -e\ncat <<'PY'\nprint('x')\nPY\nPY\ntest -f out.json";

fn absent_result(id: &str) -> CheckResult {
    CheckResult {
        verdict_streams: Default::default(),
        fault: None,
        acceptance_id: id.into(),
        exit_code: Some(127),
        quota_walk_count: 0,
        stdout: b"print('x')\n".to_vec(),
        stderr: b"/bin/sh: line 5: PY: command not found\n".to_vec(),
        environment_note: None,
        operational_error: None,
        classification: None,
    }
}

/// The R13 case: the stray word is absent on the site and on the host, so
/// no host action makes it available. Its author is asked on the first
/// attempt and on every resume, with no strike, no progress credit, and no
/// text that tells the host to install it.
#[test]
fn issue557_a_program_neither_site_nor_host_has_goes_to_its_author_without_a_strike() {
    let site = trees(&[("AC-X-001", STRAY_TERMINATOR, TrustedCwd::RepoRoot)]);
    let contract = site.contract();
    let at = Context::on_host_path(&contract);
    let result = absent_result("AC-X-001");
    let (why, cause) = super::super::verdict::no_verdict_cause(STRAY_TERMINATOR, &result, &at)
        .expect("no verdict");
    assert_eq!(cause, UnprovenCause::ProgramAbsent, "{why}");
    assert!(why.contains("`PY`"), "{why}");
    assert!(
        why.contains("neither the check's site nor the host has `PY`"),
        "{why}"
    );
    assert!(why.contains("must not depend on it"), "{why}");
    for attempt in 1..=3 {
        // A fresh probe per attempt: each is a resume of the run.
        let probe = HostProbe::for_task_set(site.set.project.path(), &site.set.tasks);
        let finding = settle_as(
            &probe,
            BaselineOrigin::RecordedBase,
            &site.base,
            &contract,
            &result,
            (&why, cause),
            &at,
        )
        .unwrap_or_else(|| panic!("attempt {attempt}: the author gets it at once"));
        assert!(finding.starts_with("check 'AC-X-001': "), "{finding}");
        assert!(finding.contains("PY: command not found"), "{finding}");
        assert!(
            finding.contains("no host action makes it available"),
            "{finding}"
        );
        assert!(
            finding.contains("Check the criterion with what the site provides"),
            "{finding}"
        );
        for false_claim in [
            "install",
            "on the host and in the check environment",
            "operational",
            "probed again on retry",
            "It failed so again",
        ] {
            assert!(!finding.contains(false_claim), "{false_claim}: {finding}");
        }
        assert!(
            probe.take_unproven_causes().is_empty(),
            "attempt {attempt}: not the host's"
        );
        assert_eq!(probe.resume.progress.saved_count(), 0, "no credit");
        assert!(
            !strike(&probe, &site.base, &contract, "AC-X-001").exists(),
            "attempt {attempt}: no strike is saved against its author"
        );
    }
}

/// The true host causes keep the strike rule: the first failure is the
/// host's (probed again on retry), the second is its author's.
#[test]
fn issue557_true_host_causes_keep_the_strike_rule() {
    let site = trees(&[("AC-X-001", "sh -c 'exit 3'", TrustedCwd::RepoRoot)]);
    let contract = site.contract();
    let at = Context::on_host_path(&contract);
    let mut result = absent_result("AC-X-001");
    result.exit_code = Some(3);
    for cause in [UnprovenCause::Host, UnprovenCause::PermanentHost] {
        let why = ("it crashed in its own shell code", cause);
        let first = HostProbe::for_task_set(site.set.project.path(), &site.set.tasks);
        assert!(
            settle_as(
                &first,
                BaselineOrigin::RecordedBase,
                &site.base,
                &contract,
                &result,
                why,
                &at
            )
            .is_none()
        );
        let unproven = first.take_unproven_causes();
        let (text, held) = &unproven["AC-X-001"];
        assert_eq!(*held, cause);
        assert!(text.contains("probed again on retry"), "{text}");
        let resumed = HostProbe::for_task_set(site.set.project.path(), &site.set.tasks);
        assert!(
            settle_as(
                &resumed,
                BaselineOrigin::RecordedBase,
                &site.base,
                &contract,
                &result,
                why,
                &at
            )
            .is_some_and(|finding| finding.contains("It failed so again")),
            "{cause:?}: the strike rule still applies to its own causes"
        );
        clear(&resumed, &site.base, &contract, "AC-X-001");
    }
    let probe = HostProbe::for_task_set(site.set.project.path(), &site.set.tasks);
    let transient = ("the listing gave no answer", UnprovenCause::TransientHost);
    assert!(
        settle_as(
            &probe,
            BaselineOrigin::RecordedBase,
            &site.base,
            &contract,
            &result,
            transient,
            &at
        )
        .is_none()
    );
    assert_eq!(
        probe.take_unproven_causes()["AC-X-001"].1,
        UnprovenCause::TransientHost
    );
    assert!(!strike(&probe, &site.base, &contract, "AC-X-001").exists());
}
