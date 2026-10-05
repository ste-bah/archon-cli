//! Issue 323: every probe site -- the scratch observation, the live direct
//! site and the probe's own hermetic copy -- bounds each check by the one
//! per-check bound (`probe_check_cap_secs`), and a freeze never gives one
//! check its whole deadline. A check past its bound is unproven (timed
//! out): the host's, resumable, and the other checks still run; timing out
//! so again on the same base goes to its author (the Issue 328 strike).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use archon_workflow::acceptance_scratch::{CheckAllowance, DIRECT_DEFAULT_TIMEOUT_SECS};
use archon_workflow::task_set_contract::TrustedCwd;

use super::super::probe_tests::{Trees, trees};
use super::super::{Baseline, ExecutabilityProbe, HostProbe};
use crate::command::workflow_freeze_budget::{FreezeBudget, FreezeResume};

const REPO: TrustedCwd = TrustedCwd::RepoRoot;
/// The bound these tests give a check, and how long a slow one sleeps.
const BOUND: u64 = 2;
const SLOW: &str = "sleep 30; test -f feature.txt";
/// Well under one slow check's sleep: the bound, not the sleep, ended it.
const QUICK: Duration = Duration::from_secs(25);

/// A round's probe at the direct site (no scratch policy): the live tree
/// for the site run, its own hermetic copy for the base.
fn round(trees: &Trees, copies: &std::path::Path) -> HostProbe {
    HostProbe::at(
        trees.set.project.path().to_path_buf(),
        trees.repo.clone(),
        None,
    )
    .with_baseline(Baseline {
        commit: trees.base.clone(),
        repository: trees.repo.clone(),
    })
    .with_copy_parent(copies.to_path_buf())
    .with_check_cap(BOUND)
}

fn timed_out(unproven: &BTreeMap<String, String>, id: &str) -> bool {
    unproven
        .get(id)
        .is_some_and(|why| why.contains("unproven (timed out)"))
}

#[tokio::test]
async fn a_direct_site_check_past_its_bound_is_unproven_timed_out_and_the_others_are_proven() {
    let trees = trees(&[
        ("AC-B-001", SLOW, REPO),
        ("AC-B-002", "test -f feature.txt", REPO),
    ]);
    let copies = tempfile::tempdir().unwrap();
    let probe = round(&trees, copies.path());
    let started = Instant::now();
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    let elapsed = started.elapsed();
    let unproven = probe.take_unproven();
    assert!(timed_out(&unproven, "AC-B-001"), "{unproven:?}");
    assert!(findings.is_empty(), "never the author's: {findings:?}");
    assert!(!unproven.contains_key("AC-B-002"), "proven: {unproven:?}");
    assert!(elapsed < QUICK, "bounded at the site: {elapsed:?}");
    trees.assert_live_untouched(copies.path());
}

/// The base copy a round's probe makes (`hermetic::run_in_copy`) is bound
/// by the same bound: this check is quick at HEAD and slow at the base.
#[tokio::test]
async fn a_rounds_base_copy_bounds_each_check_by_the_same_bound() {
    let trees = trees(&[
        (
            "AC-B-003",
            "test -f feature.txt || { sleep 30; exit 1; }",
            REPO,
        ),
        ("AC-B-004", "test -f feature.txt", REPO),
    ]);
    let copies = tempfile::tempdir().unwrap();
    let probe = round(&trees, copies.path());
    let started = Instant::now();
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    let elapsed = started.elapsed();
    let unproven = probe.take_unproven();
    assert!(timed_out(&unproven, "AC-B-003"), "{unproven:?}");
    assert!(findings.is_empty(), "{findings:?}");
    assert!(!unproven.contains_key("AC-B-004"), "{unproven:?}");
    assert!(elapsed < QUICK, "bounded in the copy: {elapsed:?}");
}

/// At the freeze's hermetic site: the slow check is unproven (timed out)
/// and the others still proven; timing out so again on the same base is
/// its author's to fix, never a retry that meets it forever.
#[tokio::test]
async fn a_hermetic_freeze_check_past_its_bound_is_unproven_then_its_authors_on_the_same_base() {
    let trees = trees(&[
        ("AC-B-005", SLOW, REPO),
        ("AC-B-006", "test -f feature.txt", REPO),
    ]);
    let copies = tempfile::tempdir().unwrap();
    let freeze = || {
        HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
            .with_copy_parent(copies.path().to_path_buf())
            .with_resume(&FreezeResume::saving(FreezeBudget::unlimited(), true))
            .without_process_memo()
            .with_check_cap(BOUND)
    };
    let first = freeze();
    let started = Instant::now();
    let findings = first.script_defects(&trees.contract(), &trees.ids()).await;
    let elapsed = started.elapsed();
    let unproven = first.take_unproven();
    assert!(timed_out(&unproven, "AC-B-005"), "{unproven:?}");
    assert!(findings.is_empty(), "first time the host's: {findings:?}");
    assert!(!unproven.contains_key("AC-B-006"), "{unproven:?}");
    assert!(first.incomplete().is_none(), "a timeout is no spent budget");
    assert!(elapsed < QUICK, "{elapsed:?}");

    let retry = freeze();
    let findings = retry.script_defects(&trees.contract(), &trees.ids()).await;
    let unproven = retry.take_unproven();
    let finding = findings.get("AC-B-005").expect("the author's now");
    assert!(
        finding.contains(&format!("per-check bound of {BOUND}s"))
            && finding.contains(&trees.base[..12]),
        "{finding}"
    );
    assert!(unproven.is_empty(), "{unproven:?}");
    assert!(!findings.contains_key("AC-B-006"), "{findings:?}");
}

fn allowance(probe: &HostProbe) -> CheckAllowance {
    let written = Arc::new(Mutex::new(Vec::new()));
    let hooks = probe.observe_hooks(&BTreeMap::new(), None, &written);
    (hooks.allowance.expect("a freeze is bounded"))()
}

/// The live freeze's wall clock (7200 s, 600 s kept back) with the
/// operator's own 7200 s per-check limit: before, the first check was given
/// all 6600 s left, cut, and the freeze deferred everything behind it.
#[test]
fn a_freeze_gives_no_check_the_whole_deadline() {
    let trees = trees(&[("AC-B-007", "true", REPO)]);
    super::cap_tests::configure(&trees, 7_200);
    let resume = FreezeResume::saving(FreezeBudget::within(7_200, Arc::new(Instant::now)), true);
    let probe =
        HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks).with_resume(&resume);
    assert_eq!(probe.check_cap_secs, 7_200, "the configured limit");
    assert_eq!(
        allowance(&probe),
        CheckAllowance::Run {
            timeout_secs: 1_650,
            cut: false
        },
        "a quarter of the 6600 s usable"
    );
}

/// Unconfigured, every site's bound is the documented direct default; a
/// freeze bounds it further by its quarter of the usable window.
#[test]
fn the_default_bound_when_unconfigured() {
    let trees = trees(&[("AC-B-008", "true", REPO)]);
    let direct = HostProbe::at(
        trees.set.project.path().to_path_buf(),
        trees.repo.clone(),
        None,
    );
    assert_eq!(direct.check_cap_secs, DIRECT_DEFAULT_TIMEOUT_SECS);
    assert_eq!(DIRECT_DEFAULT_TIMEOUT_SECS, 1_800);
    let unlimited = FreezeResume::saving(FreezeBudget::unlimited(), true);
    let hermetic =
        HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks).with_resume(&unlimited);
    assert!(matches!(hermetic.site, super::super::Site::Hermetic));
    assert_eq!(
        allowance(&hermetic),
        CheckAllowance::Run {
            timeout_secs: 1_800,
            cut: false
        }
    );
    let live = FreezeResume::saving(FreezeBudget::within(7_200, Arc::new(Instant::now)), true);
    let hermetic =
        HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks).with_resume(&live);
    assert_eq!(
        allowance(&hermetic),
        CheckAllowance::Run {
            timeout_secs: 1_650,
            cut: false
        }
    );
}
