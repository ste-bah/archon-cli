//! Old total-clock strikes are not evidence against a check's author.
use super::super::probe_tests::trees;
use super::*;
use archon_workflow::task_set_contract::TrustedCwd;

#[test]
fn issue356_legacy_total_clock_strikes_do_not_authorize_a_finding() {
    let trees = trees(&[("AC-B-001", "test -f feature.txt", TrustedCwd::RepoRoot)]);
    let contract = trees.contract();
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let path = strike(&probe, &trees.base, &contract, "AC-B-001");
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
            "AC-B-001",
            "a tool could not run",
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
            trees: trees(&[("AC-S-001", text, TrustedCwd::RepoRoot)]),
        }
    }
    fn probe(&self, window: u64) -> HostProbe {
        HostProbe::for_task_set(self.trees.set.project.path(), &self.trees.set.tasks)
            .with_check_cap(window)
    }
    fn stall(&self, probe: &HostProbe) -> Option<String> {
        settle_timed_out(probe, &self.trees.base, &self.trees.contract(), "AC-S-001")
    }
}

#[test]
fn issue356_first_stall_is_the_hosts_and_earns_no_credit() {
    let stall = Stall::new("sleep 99; test -f feature.txt");
    let probe = stall.probe(60);
    assert!(stall.stall(&probe).is_none(), "the host may be at fault");
    let unproven = probe.take_unproven();
    let why = unproven.get("AC-S-001").expect("unproven");
    assert!(
        why.contains("unproven (timed out)") && why.contains("60s"),
        "{why}"
    );
    assert_eq!(probe.resume.progress.total(), 0, "a stall is no progress");
    let path = stall_strike(
        &probe,
        &stall.trees.base,
        &stall.trees.contract(),
        "AC-S-001",
    );
    assert!(path.is_file(), "the first stall is remembered durably");
    assert!(!path.with_extension("strike.tmp").exists(), "written whole");
}

#[test]
fn issue356_same_check_stalling_again_after_resume_goes_to_its_author() {
    let stall = Stall::new("sleep 99; test -f feature.txt");
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
    let stall = Stall::new("sleep 99; test -f feature.txt");
    assert!(stall.stall(&stall.probe(60)).is_none());
    assert!(
        stall.stall(&stall.probe(120)).is_none(),
        "a longer window is a fresh trial"
    );
    let contract = stall.trees.contract();
    let other = "0".repeat(40);
    let probe = stall.probe(60);
    assert!(settle_timed_out(&probe, &other, &contract, "AC-S-001").is_none());
    // A re-authored check has new text: a new identity.
    let reauthored = Stall::new("sleep 98; test -f feature.txt");
    assert!(reauthored.stall(&reauthored.probe(60)).is_none());
}

#[test]
fn issue356_a_check_that_ran_to_an_end_forgets_its_stall() {
    let stall = Stall::new("sleep 99; test -f feature.txt");
    assert!(stall.stall(&stall.probe(60)).is_none());
    ran(
        &stall.probe(60),
        &stall.trees.base,
        &stall.trees.contract(),
        "AC-S-001",
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
    let stall = Stall::new("sleep 99; test -f feature.txt");
    let probe = stall.probe(60);
    let path = stall_strike(
        &probe,
        &stall.trees.base,
        &stall.trees.contract(),
        "AC-S-001",
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
        let why = probe.take_unproven().remove("AC-S-001").unwrap();
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
    let stall = Stall::new("sleep 99; test -f feature.txt");
    let probe = stall.probe(60);
    let contract = stall.trees.contract();
    let legacy = strike(&probe, &stall.trees.base, &contract, "AC-S-001");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, "it ran past the probe's per-check bound of 1800s").unwrap();
    assert!(
        stall.stall(&probe).is_none(),
        "an old total clock is no stall"
    );
    std::fs::write(&legacy, "a tool could not run").unwrap();
    let fresh = Stall::new("sleep 97; test -f feature.txt");
    let probe = fresh.probe(60);
    let contract = fresh.trees.contract();
    let verdictless = strike(&probe, &fresh.trees.base, &contract, "AC-S-001");
    std::fs::create_dir_all(verdictless.parent().unwrap()).unwrap();
    std::fs::write(&verdictless, "a tool could not run").unwrap();
    assert!(
        fresh.stall(&probe).is_none(),
        "a no-verdict strike is not a stall"
    );
}
