//! Old total-clock strikes are not evidence against a check's author.
use super::super::probe_tests::trees;
use super::*;
use archon_workflow::task_set_contract::TrustedCwd;

#[test]
fn issue356_legacy_total_clock_strikes_do_not_authorize_a_finding() {
    let trees = trees(&[("AC-X-001", "test -f feature.txt", TrustedCwd::RepoRoot)]);
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let contract = trees.contract();
    let path = strike(&probe, &trees.base, &contract, "AC-X-001");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    for bound in [1_800, 7_200, 20_000] {
        std::fs::write(
            &path,
            format!("it ran past the probe's per-check bound of {bound}s"),
        )
        .unwrap();
        let finding = strike_or_unproven(
            &probe,
            &trees.base,
            &contract,
            "AC-X-001",
            None,
            ("a tool could not run", UnprovenCause::Host),
            || ("author finding".into(), "host unproven".into()),
        );
        assert!(
            finding.is_none(),
            "the old clock alone cannot authorize an author finding"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "a tool could not run",
            "only current non-clock evidence gets a strike"
        );
    }
}

/// #356: one check stalling on one commit, under one window.
struct Stall {
    trees: super::super::probe_tests::Trees,
}
impl Stall {
    fn new(text: &str) -> Self {
        Self {
            trees: trees(&[("AC-X-001", text, TrustedCwd::RepoRoot)]),
        }
    }
    fn probe(&self, window: u64) -> HostProbe {
        HostProbe::for_task_set(self.trees.set.project.path(), &self.trees.set.tasks)
            .with_check_cap(window)
    }
    fn stall(&self, probe: &HostProbe) -> Option<String> {
        settle_timed_out(probe, &self.trees.base, &self.trees.contract(), "AC-X-001")
    }
}

#[test]
fn issue356_first_stall_is_the_hosts_and_earns_no_credit() {
    let stall = Stall::new("check-with-no-progress");
    let probe = stall.probe(60);
    assert!(stall.stall(&probe).is_none(), "the host may be at fault");
    let unproven = probe.take_unproven();
    let why = unproven.get("AC-X-001").expect("unproven");
    assert!(
        why.contains("unproven (timed out)") && why.contains("60s"),
        "{why}"
    );
    assert_eq!(probe.resume.progress.total(), 0, "a stall is no progress");
    let path = stall_strike(
        &probe,
        &stall.trees.base,
        &stall.trees.contract(),
        "AC-X-001",
    );
    assert!(path.is_file(), "the first stall is remembered durably");
    assert!(!path.with_extension("strike.tmp").exists(), "written whole");
}

#[test]
fn issue356_same_check_stalling_again_after_resume_goes_to_its_author() {
    let stall = Stall::new("check-with-no-progress");
    assert!(stall.stall(&stall.probe(60)).is_none());
    // A resume is a new process: a new probe over the same durable cache.
    let resumed = stall.probe(60);
    let finding = stall.stall(&resumed).expect("a check defect");
    assert!(
        finding.contains("made no progress") && finding.contains("Repair or replace"),
        "{finding}"
    );
    assert!(
        resumed.take_unproven().is_empty(),
        "the author's, not the host's"
    );
    assert_eq!(resumed.resume.progress.total(), 1, "routing it is progress");
}

#[test]
fn issue356_a_changed_check_window_or_commit_is_a_first_stall_again() {
    let stall = Stall::new("check-with-no-progress");
    let contract = stall.trees.contract();
    assert!(stall.stall(&stall.probe(60)).is_none());
    assert!(
        stall.stall(&stall.probe(120)).is_none(),
        "a longer window is a fresh trial"
    );
    let other = "0".repeat(40);
    let probe = stall.probe(60);
    assert!(settle_timed_out(&probe, &other, &contract, "AC-X-001").is_none());
    // A re-authored check has new text: a new identity.
    let reauthored = Stall::new("changed-check-with-no-progress");
    assert!(reauthored.stall(&reauthored.probe(60)).is_none());
}

#[test]
fn issue356_a_check_that_ran_to_an_end_forgets_its_stall() {
    let stall = Stall::new("check-with-no-progress");
    assert!(stall.stall(&stall.probe(60)).is_none());
    ran(
        &stall.probe(60),
        &stall.trees.base,
        &stall.trees.contract(),
        "AC-X-001",
    );
    assert!(
        stall.stall(&stall.probe(60)).is_none(),
        "an intermittent stall is the host's again"
    );
}

#[cfg(unix)]
#[test]
fn issue356_an_unsaveable_stall_strike_stays_the_hosts() {
    use std::os::unix::fs::PermissionsExt;
    let stall = Stall::new("check-with-no-progress");
    let probe = stall.probe(60);
    let path = stall_strike(
        &probe,
        &stall.trees.base,
        &stall.trees.contract(),
        "AC-X-001",
    );
    let dir = path.parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    for _ in 0..3 {
        let probe = stall.probe(60);
        assert!(
            stall.stall(&probe).is_none(),
            "never a finding without its strike"
        );
        let why = probe.take_unproven().remove("AC-X-001").unwrap();
        assert!(
            why.contains("could not be saved"),
            "said, not hidden: {why}"
        );
        assert!(!path.exists() && !path.with_extension("strike.tmp").exists());
    }
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn issue356_old_clock_or_no_verdict_strikes_never_count_as_a_stall() {
    let stall = Stall::new("check-with-no-progress");
    let probe = stall.probe(60);
    let legacy = stall_strike(
        &probe,
        &stall.trees.base,
        &stall.trees.contract(),
        "AC-X-001",
    );
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, "it ran past the probe's per-check bound of 1800s").unwrap();
    assert!(
        stall.stall(&probe).is_none(),
        "an old total clock is no stall"
    );
    std::fs::write(&legacy, "a tool could not run").unwrap();
    let fresh = Stall::new("different-check-with-no-progress");
    let probe = fresh.probe(60);
    let verdictless = strike(
        &probe,
        &fresh.trees.base,
        &fresh.trees.contract(),
        "AC-X-001",
    );
    std::fs::create_dir_all(verdictless.parent().unwrap()).unwrap();
    std::fs::write(&verdictless, "a tool could not run").unwrap();
    assert!(
        fresh.stall(&probe).is_none(),
        "a no-verdict strike is not a stall"
    );
}

fn set_command(contract: &mut AcceptanceContract, command: &str) {
    match &mut contract.acceptance[0].check {
        archon_workflow::task_set_contract::AcceptanceCheck::Command { command: text, .. } => {
            *text = command.to_string();
        }
        _ => panic!("command check fixture"),
    }
}

fn failed_result(id: &str, stderr: &str) -> CheckResult {
    CheckResult {
        acceptance_id: id.into(),
        exit_code: Some(101),
        quota_walk_count: 0,
        stdout: Vec::new(),
        stderr: stderr.as_bytes().to_vec(),
        environment_note: None,
        operational_error: None,
        classification: None,
    }
}

#[test]
fn issue475_same_missing_setup_path_after_reauthoring_goes_to_author() {
    let trees = trees(&[("AC-X-001", "verify", TrustedCwd::ProjectRoot)]);
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let mut contract = trees.contract();
    set_command(&mut contract, "cd ../../repo/specification.txt && verify");
    let first_text = "cd ../../repo/a/../b/ && verify";
    set_command(&mut contract, first_text);
    let result = failed_result("AC-X-001", "../../repo/a/../b does not exist");
    let at = Context::on_host_path(&contract).within(1, &["repo", "project"]);
    let why = super::super::verdict::no_verdict(first_text, &result, &at).expect("no verdict");
    assert!(why.contains("setup failure of its site"), "{why}");
    assert!(settle(&probe, &trees.base, &contract, &result, &why, &at,).is_none());
    set_command(&mut contract, "cd ../../repo/b && verify");
    let text = check_text(&contract, "AC-X-001");
    let result = failed_result("AC-X-001", "../../repo/b does not exist");
    let why = super::super::verdict::no_verdict(text, &result, &at).expect("no verdict");
    let finding = settle(&probe, &trees.base, &contract, &result, &why, &at)
        .expect("same setup failure routes to the author after resume");
    assert!(finding.contains("It failed so again"), "{finding}");
}

#[test]
fn issue475_different_missing_path_and_commit_are_fresh_strikes() {
    let trees = trees(&[("AC-X-001", "verify", TrustedCwd::ProjectRoot)]);
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let mut contract = trees.contract();
    set_command(&mut contract, "cd ../../repo/specification.txt && verify");
    let result = failed_result("AC-X-001", "../../repo/specification.txt does not exist");
    let at = Context::on_host_path(&contract).within(1, &["repo", "project"]);
    let text = check_text(&contract, "AC-X-001");
    let why = super::super::verdict::no_verdict(text, &result, &at).unwrap();
    assert!(settle(&probe, &trees.base, &contract, &result, &why, &at,).is_none());
    assert!(
        settle(
            &probe,
            &trees.base,
            &contract,
            &failed_result("AC-X-001", "../../repo/other.toml does not exist"),
            &why,
            &at,
        )
        .is_none(),
        "a different missing path starts a fresh strike"
    );
    assert!(
        settle(&probe, &trees.head, &contract, &result, &why, &at,).is_none(),
        "a different commit starts a fresh strike"
    );
    let mut other_root = contract.clone();
    match &mut other_root.acceptance[0].check {
        archon_workflow::task_set_contract::AcceptanceCheck::Command { cwd, .. } => {
            *cwd = TrustedCwd::RepoRoot
        }
        _ => panic!("command check fixture"),
    }
    let setup_a = setup_path_signature(&contract, "AC-X-001", &result, &why, &at).unwrap();
    let at_other = Context::on_host_path(&other_root).within(1, &["repo", "project"]);
    let setup_b = setup_path_signature(&other_root, "AC-X-001", &result, &why, &at_other).unwrap();
    assert_ne!(
        setup_a, setup_b,
        "the selected root is part of the setup identity"
    );
}

#[test]
fn issue475_no_exit_kills_with_changed_text_stay_fresh_host_strikes() {
    let trees = trees(&[("AC-X-001", "check-one", TrustedCwd::ProjectRoot)]);
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let mut contract = trees.contract();
    let at = Context::on_host_path(&contract).within(1, &["repo", "project"]);
    for text in ["check-one", "check-two"] {
        set_command(&mut contract, text);
        let result = failed_result("AC-X-001", "");
        let mut result = result;
        result.exit_code = None;
        let why = super::super::verdict::no_verdict(text, &result, &at).unwrap();
        assert!(settle(&probe, &trees.base, &contract, &result, &why, &at).is_none());
    }
}

#[test]
fn issue475_absolute_setup_paths_are_normalized_by_the_real_classifier() {
    let trees = trees(&[("AC-X-001", "verify", TrustedCwd::ProjectRoot)]);
    let mut contract = trees.contract();
    let at = Context::on_host_path(&contract).within(1, &["repo", "project"]);
    let mut signatures = Vec::new();
    for (text, output) in [
        ("cd /tmp/a/../b/ && verify", "/tmp/a/../b does not exist"),
        ("cd /tmp/b && verify", "/tmp/b does not exist"),
    ] {
        set_command(&mut contract, text);
        let result = failed_result("AC-X-001", output);
        let why = super::super::verdict::no_verdict(text, &result, &at).expect("no verdict");
        assert!(why.contains("setup failure of its site"), "{why}");
        signatures.push(setup_path_signature(&contract, "AC-X-001", &result, &why, &at).unwrap());
    }
    assert_eq!(signatures[0], signatures[1]);
    assert_eq!(normalize_path("/tmp/a/../b///"), "/tmp/b");
}

/// F6 across a resume: a site that lacks a program the host has is the
/// site's defect, so an existing strike never makes the check its author's;
/// it stays the host's, with no progress credit, on every attempt. A cause
/// the strike rule is for (a crash in its own code) still goes to its
/// author the second time.
#[test]
fn f6_a_site_lacking_a_program_the_host_has_stays_the_hosts_after_a_resume() {
    let text = "sh -c true";
    let site = trees(&[("AC-X-001", text, TrustedCwd::RepoRoot)]);
    let contract = site.contract();
    let empty = tempfile::tempdir().unwrap();
    let site_path = empty.path().display().to_string();
    let at = Context::new([("PATH".to_string(), site_path)].into(), &contract);
    let mut result = failed_result("AC-X-001", "sh: command not found");
    result.exit_code = Some(127);
    let (why, cause) =
        super::super::verdict::no_verdict_cause(text, &result, &at).expect("no verdict");
    assert_ne!(cause, UnprovenCause::ProgramAbsent, "the host has it");
    for attempt in 1..=3 {
        // A fresh probe per attempt: each is a resume of the run.
        let probe = HostProbe::for_task_set(site.set.project.path(), &site.set.tasks);
        let finding = settle_as(&probe, &site.base, &contract, &result, (&why, cause), &at);
        assert!(finding.is_none(), "attempt {attempt}: {finding:?}");
        let unproven = probe.take_unproven_causes();
        let (reason, recorded) = unproven.get("AC-X-001").expect("unproven");
        assert_eq!(*recorded, cause, "attempt {attempt}");
        assert!(!reason.contains("goes to its author"), "{reason}");
        if attempt > 1 {
            assert_eq!(probe.resume.progress.saved_count(), 0, "no credit");
        }
    }
    let crash = ("it crashed in its own shell code", UnprovenCause::Host);
    let other = trees(&[("AC-X-001", "sh -c 'exit 3'", TrustedCwd::RepoRoot)]);
    let contract = other.contract();
    let result = failed_result("AC-X-001", "syntax error");
    let first = HostProbe::for_task_set(other.set.project.path(), &other.set.tasks);
    assert!(settle_as(&first, &other.base, &contract, &result, crash, &at).is_none());
    let resumed = HostProbe::for_task_set(other.set.project.path(), &other.set.tasks);
    assert!(
        settle_as(&resumed, &other.base, &contract, &result, crash, &at)
            .is_some_and(|finding| finding.contains("It failed so again")),
        "the strike rule still applies to its own causes"
    );
}
