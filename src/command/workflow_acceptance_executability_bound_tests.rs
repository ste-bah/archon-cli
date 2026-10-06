//! Every probe site uses the site's full no-progress window. A stalled check
//! remains unproven across retries, and completed checks stay saved and reusable.

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

/// A real stall stays resumable on repeated attempts; the other verdicts survive.
#[tokio::test]
async fn issue356_repeated_silent_check_stays_resumable() {
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
    assert!(
        first.incomplete().is_none(),
        "the stall is local to its check"
    );
    assert!(
        first.resume.progress.saved_count() > 0,
        "the other check is durably saved"
    );
    assert!(elapsed < QUICK, "{elapsed:?}");

    let retry = freeze();
    let findings = retry.script_defects(&trees.contract(), &trees.ids()).await;
    let unproven = retry.take_unproven();
    assert!(
        findings.is_empty(),
        "a repeated stall is never the author's defect: {findings:?}"
    );
    assert!(
        timed_out(&unproven, "AC-B-005"),
        "still resumable: {unproven:?}"
    );
    assert!(
        retry.resume.progress.reused_count() > 0,
        "the saved check is reused"
    );
    assert!(!findings.contains_key("AC-B-006"), "{findings:?}");
}

fn allowance(probe: &HostProbe) -> CheckAllowance {
    let written = Arc::new(Mutex::new(Vec::new()));
    let hooks = probe.observe_hooks(&BTreeMap::new(), None, &written);
    (hooks.allowance.expect("a freeze is bounded"))()
}

/// The configured site gets its entire no-progress window, without a share.
#[test]
fn a_freeze_uses_the_full_site_no_progress_window() {
    let trees = trees(&[("AC-B-007", "true", REPO)]);
    super::cap_tests::configure(&trees, 7_200);
    let resume = FreezeResume::saving(FreezeBudget::within(7_200, Arc::new(Instant::now)), true);
    let probe =
        HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks).with_resume(&resume);
    assert_eq!(probe.check_cap_secs, 7_200, "the configured limit");
    assert_eq!(
        allowance(&probe),
        CheckAllowance::Run {
            timeout_secs: 7_200,
            cut: false
        },
        "the full per-site no-progress window"
    );
}

/// Unconfigured, every site uses the documented direct no-progress default.
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
            timeout_secs: 1_800,
            cut: false
        }
    );
}
