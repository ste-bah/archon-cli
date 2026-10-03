//! The freeze budget on an injected clock (Issue 255).

use std::sync::atomic::{AtomicU64, Ordering::SeqCst};

use super::*;

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
fn a_check_gets_its_cap_until_the_budget_is_nearly_spent_then_nothing_starts() {
    let (now, clock) = hand_clock();
    let budget = FreezeBudget::within(7_800, clock);
    let run = |timeout_secs, cut| CheckAllowance::Run { timeout_secs, cut };
    assert_eq!(budget.allowance(1_200), run(1_200, false));
    assert!(budget.allows_observation());
    // 7_200 usable: with 700 left, a check is cut to what remains.
    now.store(6_500, SeqCst);
    assert_eq!(budget.allowance(1_200), run(700, true));
    now.store(
        7_200 - MIN_CHECK_WINDOW_SECS - OBSERVATION_SETUP_SECS + 1,
        SeqCst,
    );
    assert!(!budget.allows_observation(), "too late to prepare a slot");
    assert!(matches!(
        budget.allowance(1_200),
        CheckAllowance::Run { .. }
    ));
    now.store(7_200 - MIN_CHECK_WINDOW_SECS + 1, SeqCst);
    assert_eq!(budget.allowance(1_200), CheckAllowance::Defer);
    now.store(99_999, SeqCst);
    assert_eq!(budget.allowance(1_200), CheckAllowance::Defer);
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
fn the_staged_budget_reads_the_catalog_wall_clock_and_keeps_the_margin() {
    let catalog = crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("")
        .expect("the fixed catalog");
    let outer = catalog.capabilities["freeze-acceptance"].timeout_secs;
    let budget = FreezeBudget::for_host_command("freeze-acceptance");
    assert_eq!(budget.outer_secs, outer);
    let left = budget.remaining().expect("a deadline").as_secs();
    assert!(
        left <= outer - FREEZE_SAFETY_MARGIN_SECS && left + 5 >= outer - FREEZE_SAFETY_MARGIN_SECS
    );
    assert!(
        FreezeBudget::for_host_command("no-such-command")
            .remaining()
            .is_none()
    );
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
