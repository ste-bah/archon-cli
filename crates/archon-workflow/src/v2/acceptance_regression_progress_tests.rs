//! Issue 263: the regression search is bounded by observations that make no
//! progress, never by a fixed count of observations or a total wall clock.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::tests::{FlagObserver, World, check};
use super::{CheckObserver, MAX_BARREN_OBSERVATIONS, SearchBudget, Verdict};

/// Twelve failing checks, each broken by its own landing among fillers: the
/// searches need more observations than the old fixed budget of 32, and
/// every check is still attributed to the landing that broke it.
#[tokio::test]
async fn every_failing_group_is_attributed_however_many_observations_it_takes() {
    let ids: Vec<String> = (0..12).map(|n| format!("c{n:02}")).collect();
    let base: Vec<(&str, &str)> = ids.iter().map(|id| (id.as_str(), "ok")).collect();
    let w = World::new(&base);
    let mut breaking = BTreeMap::new();
    for (n, id) in ids.iter().enumerate() {
        for filler in 0..3 {
            w.step(
                &format!("fill-{n}-{filler}.txt"),
                "1",
                &format!("implement-fill-{n}-{filler}"),
                "TASK-FILL",
            );
        }
        let commit = w.step(
            &format!("flags/{id}"),
            "bad",
            &format!("remediate-break-{n}"),
            &format!("TASK-BREAK-{n}"),
        );
        breaking.insert(id.clone(), commit);
    }
    let failing: Vec<_> = ids.iter().map(|id| check(id, "TASK-OWNER")).collect();
    let observer = FlagObserver::new(&w.repo);
    let found = w
        .attribute(&failing, &observer, SearchBudget::default())
        .await;
    assert!(
        observer.observations() > 32,
        "the scenario needs more than the old fixed budget: {}",
        observer.observations()
    );
    for id in &ids {
        let regression = found
            .regressions
            .get(id)
            .unwrap_or_else(|| panic!("{id} not attributed: {:#?}", found.searches.get(id)));
        assert_eq!(&regression.landing_commit, &breaking[id], "{id}");
    }
}

/// An observer that never gives a verdict: every observation is barren.
struct Barren(AtomicUsize);

#[async_trait::async_trait]
impl CheckObserver for Barren {
    async fn observe(&self, _: &str, _: &[String]) -> Option<BTreeMap<String, Verdict>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        None
    }
}

/// Observations that add no verdict are the stall: the search stops after
/// `MAX_BARREN_OBSERVATIONS` of them in a row, and nothing is dropped.
#[tokio::test]
async fn barren_observations_stop_the_search_and_drop_no_check() {
    let ids: Vec<String> = (0..6).map(|n| format!("b{n}")).collect();
    let base: Vec<(&str, &str)> = ids.iter().map(|id| (id.as_str(), "ok")).collect();
    let w = World::new(&base);
    for (n, id) in ids.iter().enumerate() {
        w.step(
            &format!("flags/{id}"),
            "bad",
            &format!("remediate-break-{n}"),
            &format!("TASK-OWNER-{n}"),
        );
    }
    let failing: Vec<_> = (ids.iter().enumerate())
        .map(|(n, id)| check(id, &format!("TASK-OWNER-{n}")))
        .collect();
    let observer = Barren(AtomicUsize::new(0));
    let found = w
        .attribute(&failing, &observer, SearchBudget::default())
        .await;
    assert_eq!(observer.0.load(Ordering::SeqCst), MAX_BARREN_OBSERVATIONS);
    assert!(found.regressions.is_empty(), "{found:#?}");
    assert_eq!(found.searches.len(), ids.len(), "{found:#?}");
}
