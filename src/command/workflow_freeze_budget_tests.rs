//! The freeze budget on an injected clock (Issue 255).

use std::sync::atomic::{AtomicU64, Ordering::SeqCst};

use super::*;
use std::time::Duration;

/// A clock the test moves by hand: `start + seconds`.
pub(crate) fn hand_clock() -> (Arc<AtomicU64>, Clock) {
    let start = Instant::now();
    let seconds = Arc::new(AtomicU64::new(0));
    let read = seconds.clone();
    (
        seconds,
        Arc::new(move || start + Duration::from_secs(read.load(SeqCst))),
    )
}

#[test]
fn orchestration_never_cuts_a_site_window_to_elapsed_time() {
    let (now, clock) = hand_clock();
    let budget = FreezeBudget::within(7_800, clock);
    for seconds in [0, 6_500, 7_201, 99_999] {
        now.store(seconds, SeqCst);
        assert_eq!(
            budget.allowance(1_200),
            CheckAllowance::Run {
                timeout_secs: 1_200,
                cut: false
            }
        );
        assert!(budget.allows_observation());
    }
}

#[test]
fn an_unlimited_budget_only_applies_the_cap() {
    let budget = FreezeBudget::unlimited();
    assert_eq!(
        budget.allowance(1_200),
        CheckAllowance::Run {
            timeout_secs: 1_200,
            cut: false
        }
    );
    assert!(budget.allows_observation());
}

#[test]
fn the_staged_budget_reads_the_catalog_no_progress_window() {
    let catalog =
        crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("").unwrap();
    for id in ["freeze-acceptance", "task-set-lint"] {
        let budget = FreezeBudget::for_host_command(id);
        assert_eq!(budget.outer_secs(), catalog.capabilities[id].timeout_secs);
        assert_eq!(budget.check_bound(20_000), 20_000);
    }
}

#[test]
fn the_incomplete_outcome_is_recognisable_and_says_what_was_saved() {
    let progress = FreezeProgress::default();
    for _ in 0..3 {
        progress.saved(false);
    }
    progress.reused(false);
    progress.reused(false);
    progress.saved(true);
    let incomplete = FreezeIncomplete::new(
        &FreezeBudget::within(7_800, Arc::new(Instant::now)),
        &progress,
        vec!["AC-1-004".into(), "AC-1-005".into()],
    );
    let text = incomplete.to_string();
    assert!(text.starts_with(FREEZE_INCOMPLETE_RESUMABLE), "{text}");
    for part in [
        "3 probe result(s) were saved",
        "2 reused",
        "AC-1-004, AC-1-005",
        "7800s",
    ] {
        assert!(text.contains(part), "{part}: {text}");
    }
    // The host reads the last progress line: every unit saved for the call.
    let report = incomplete.report();
    assert_eq!(
        report.lines().last(),
        Some("archon-host-progress: 6"),
        "{report}"
    );
    assert_eq!(
        crate::command::workflow_host_command_operational::reported_progress(report.as_bytes()),
        Some(6)
    );
    progress.withdrawn();
    assert_eq!(progress.total(), 5);
    let error = anyhow::Error::new(incomplete).context("preparing the freeze");
    assert!(FreezeIncomplete::caused(&error).is_some());
    assert!(FreezeIncomplete::caused(&anyhow::anyhow!("other")).is_none());
}

/// Round 3 (decision D): the progress total saturates; counters imported
/// from disk can never overflow it.
#[test]
fn the_progress_total_saturates() {
    let progress = FreezeProgress::default();
    progress.reused_judged(u64::MAX);
    progress.saved(false);
    progress.reused(false);
    assert_eq!(progress.total(), u64::MAX);
}

// Issue 356: orchestration must never ration active work by elapsed totals.
#[test]
fn issue356_freeze_runs_past_old_total() {
    let (now, clock) = hand_clock();
    let resume = FreezeResume::saving(FreezeBudget::within(7_800, clock), true);
    for seconds in [2_000, 6_000, 8_000, 20_000] {
        now.store(seconds, SeqCst);
        resume.progress.saved(false);
        assert!(resume.budget.allows_observation(), "active at {seconds}s");
        assert_eq!(
            resume.budget.allowance(7_200),
            CheckAllowance::Run {
                timeout_secs: 7_200,
                cut: false
            }
        );
    }
}

#[test]
fn issue356_check_keeps_its_site_window() {
    let (_, clock) = hand_clock();
    let budget = FreezeBudget::within(7_800, clock);
    for cap in [1_801, 7_200, 20_000] {
        assert_eq!(budget.check_bound(cap), cap);
    }
}

#[test]
fn issue356_lint_has_no_total_deadline() {
    let (now, clock) = hand_clock();
    let budget = FreezeBudget::within(7_800, clock);
    for seconds in [7_801, 16_000, 100_000] {
        now.store(seconds, SeqCst);
        assert_eq!(
            budget.allowance(7_200),
            CheckAllowance::Run {
                timeout_secs: 7_200,
                cut: false
            }
        );
    }
}
