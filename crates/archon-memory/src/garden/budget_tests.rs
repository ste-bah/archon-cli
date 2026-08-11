use std::time::Duration;

use super::{BudgetLedger, GardenBudget};

#[test]
fn an_unbounded_ledger_never_refuses_and_never_reports_exhaustion() {
    // The interactive paths run under this. If it ever refused, `/garden` would
    // quietly start doing less than it used to.
    let mut ledger = BudgetLedger::new(GardenBudget::unbounded());

    for _ in 0..10_000 {
        assert!(ledger.take_reversible());
        assert!(ledger.take_deletion());
        assert!(ledger.take_proposal());
    }

    assert!(!ledger.exhausted());
    assert_eq!(ledger.spent_reversible(), 10_000);
    assert_eq!(ledger.spent_deletions(), 10_000);
    assert_eq!(ledger.spent_proposals(), 10_000);
}

#[test]
fn the_reversible_ceiling_refuses_the_unit_after_it_is_reached() {
    let mut ledger = BudgetLedger::new(GardenBudget {
        max_reversible_ops: Some(3),
        ..GardenBudget::unbounded()
    });

    assert!(ledger.take_reversible());
    assert!(ledger.take_reversible());
    assert!(ledger.take_reversible());
    assert!(
        !ledger.take_reversible(),
        "the fourth claim must be refused, not merely counted"
    );
    assert!(ledger.exhausted());
    assert_eq!(
        ledger.spent_reversible(),
        3,
        "a refused claim must not be charged"
    );
}

#[test]
fn a_scheduled_budget_cannot_delete_anything() {
    // The single most important assertion in this file. `GardenBudget::scheduled`
    // does not take a deletion count precisely so that no configuration, and no
    // later edit at a call site, can raise it above zero.
    let mut ledger = BudgetLedger::new(GardenBudget::scheduled(500, 100, Duration::from_secs(300)));

    assert!(
        !ledger.take_deletion(),
        "an unattended pass must not be able to destroy a single memory"
    );
    assert_eq!(ledger.spent_deletions(), 0);
    assert!(ledger.exhausted());
}

#[test]
fn a_scheduled_budget_still_allows_proposals_and_reversible_work() {
    // The point of refusing deletion is to redirect it, not to make the pass
    // inert. If this ever failed, the scheduler would be a no-op that looked
    // like a working safety mechanism.
    let mut ledger = BudgetLedger::new(GardenBudget::scheduled(500, 100, Duration::from_secs(300)));

    assert!(ledger.take_reversible());
    assert!(ledger.take_proposal());
    assert!(!ledger.exhausted());
}

#[test]
fn the_retirement_candidate_ceiling_bounds_the_review_pile() {
    let mut ledger = BudgetLedger::new(GardenBudget {
        max_retirement_candidates: Some(2),
        ..GardenBudget::unbounded()
    });

    assert!(ledger.take_proposal());
    assert!(ledger.take_proposal());
    assert!(!ledger.take_proposal());
    assert!(ledger.exhausted());
    assert_eq!(ledger.spent_proposals(), 2);
}

#[test]
fn the_two_allowances_are_not_interchangeable() {
    // A merge-heavy pass must not be able to eat the deletion allowance, nor the
    // reverse. Pooling them would make the ceiling on destruction depend on how
    // many duplicates happened to exist.
    let mut ledger = BudgetLedger::new(GardenBudget {
        max_reversible_ops: Some(1),
        max_deletions: Some(1),
        ..GardenBudget::unbounded()
    });

    assert!(ledger.take_reversible());
    assert!(
        ledger.take_deletion(),
        "spending the reversible allowance must not consume the deletion one"
    );
    assert!(!ledger.take_reversible());
    assert!(!ledger.take_deletion());
}

#[test]
fn an_expired_deadline_refuses_every_further_unit() {
    // Zero duration means every claim is already late, which is the cleanest way
    // to pin the deadline without sleeping in a test.
    let mut ledger = BudgetLedger::new(GardenBudget {
        max_duration: Some(Duration::ZERO),
        ..GardenBudget::unbounded()
    });

    assert!(!ledger.take_reversible());
    assert!(!ledger.take_deletion());
    assert!(!ledger.take_proposal());
    assert!(
        ledger.exhausted(),
        "running out of time must report as exhaustion even though no count was hit"
    );
}

#[test]
fn a_ledger_within_its_deadline_still_works() {
    let mut ledger = BudgetLedger::new(GardenBudget {
        max_duration: Some(Duration::from_secs(3600)),
        ..GardenBudget::unbounded()
    });

    assert!(ledger.take_reversible());
    assert!(!ledger.exhausted());
    assert!(ledger.elapsed() < Duration::from_secs(3600));
}

#[test]
fn two_passes_do_not_share_one_allowance() {
    // Ledgers are per pass. If they pooled, two overlapping consolidations would
    // each be bounded by half a budget -- and worse, the bound would depend on
    // what some other run had already spent.
    let budget = GardenBudget {
        max_reversible_ops: Some(2),
        ..GardenBudget::unbounded()
    };
    let mut first = BudgetLedger::new(budget);
    let mut second = BudgetLedger::new(budget);

    assert!(first.take_reversible());
    assert!(first.take_reversible());
    assert!(!first.take_reversible());

    assert!(
        second.take_reversible(),
        "a fresh pass must start with a full allowance"
    );
}
