//! Batch J (b): the regression search's budget. Nothing is dropped when it
//! runs out, and a landing already pinned is confirmed for the checks
//! sharing it before a sweep can spend what is left.

use std::time::Duration;

use super::tests::{FlagObserver, World, check};
use super::{FailingCheck, SearchBudget, failure_signature};

/// Batch J (b): when the budget runs out, every unattributed check still
/// comes back, with a note saying so. Nothing is dropped.
#[tokio::test]
async fn a_spent_budget_leaves_every_check_with_a_note() {
    let w = World::new(&[("a", "ok"), ("b", "missing")]);
    w.step("x.txt", "1", "implement-x-1", "TASK-X");
    w.step("flags/a", "bad", "review-remediate-y-2", "TASK-Y");
    w.step("z.txt", "1", "implement-z-3", "TASK-Z");
    let failing = [check("a", "TASK-OWNER"), check("b", "TASK-OWNER")];
    let observer = FlagObserver::new(&w.repo);
    let budget = SearchBudget {
        observations: 1,
        time: Duration::from_secs(600),
    };
    let found = w.attribute(&failing, &observer, budget).await;
    assert_eq!(observer.observations(), 1);
    assert_eq!(
        found.regressions.len() + found.searches.len(),
        2,
        "{found:#?}"
    );
    // `a` held at the base and its bisection was cut short; `b` already
    // failed this way at the base, which the one observation showed.
    assert!(found.searches["a"].note.contains("budget"), "{found:#?}");
    assert!(
        found.searches["b"].note.contains("already fails this way"),
        "{found:#?}"
    );
    // No time left: nothing is observed, and nothing is dropped either.
    let none = SearchBudget {
        observations: 8,
        time: Duration::ZERO,
    };
    let fresh = World::new(&[("a", "ok")]);
    fresh.step("flags/a", "bad", "review-remediate-y-1", "TASK-Y");
    let observer = FlagObserver::new(&fresh.repo);
    let found = fresh.attribute(&failing, &observer, none).await;
    assert_eq!(observer.observations(), 0);
    assert_eq!(found.searches.len(), 2, "{found:#?}");
}

/// A pinned landing is confirmed for the checks sharing it before a check
/// that never held can spend the budget sweeping the run.
#[tokio::test]
async fn a_shared_landing_is_confirmed_before_a_sweep_spends_the_budget() {
    let w = World::new(&[("a", "ok"), ("b", "ok")]);
    for at in 0..4 {
        w.step(
            &format!("p{at}.txt"),
            "1",
            &format!("implement-p-{at}"),
            "TASK-P",
        );
    }
    std::fs::write(w.repo.join("flags/b"), "bad").unwrap();
    let breaking = w.step("flags/a", "bad", "review-remediate-y-5", "TASK-Y");
    for at in 6..10 {
        w.step(
            &format!("q{at}.txt"),
            "1",
            &format!("implement-q-{at}"),
            "TASK-Q",
        );
    }
    let same = failure_signature(Some(1), "Error: same\n", "");
    let mut failing: Vec<FailingCheck> = ["a", "b"]
        .iter()
        .map(|id| FailingCheck {
            signature: same.clone(),
            ..check(id, "TASK-OWNER")
        })
        .collect();
    failing.push(check("never", "TASK-OWNER"));
    let observer = FlagObserver::new(&w.repo);
    let budget = SearchBudget {
        observations: 7,
        time: Duration::from_secs(600),
    };
    let found = w.attribute(&failing, &observer, budget).await;
    assert_eq!(
        found.regressions["b"].landing_commit, breaking,
        "{found:#?}"
    );
    assert_eq!(found.regressions["b"].probed_as.as_deref(), Some("a"));
    assert!(
        found.searches["never"]
            .note
            .contains("already fails this way"),
        "{found:#?}"
    );
}
