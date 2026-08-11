//! The observer seam, exercised through a real injection.
//!
//! Nothing here calls `notify` directly. An observer proven only against a
//! helper the production path never reaches is the failure mode this whole
//! change exists to avoid, so every test drives `MemoryInjector::inject` and
//! reads what arrived.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use super::{
    InjectionObserver, InjectionOutcome, clear_injection_observer, has_injection_observer,
    set_injection_observer,
};
use crate::graph::MemoryGraph;
use crate::injection::MemoryInjector;
use crate::types::MemoryType;

/// The observer slot is process-wide, so these tests must not overlap.
fn exclusive() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What one injection reported, flattened for assertions.
#[derive(Debug, Clone, PartialEq)]
struct Seen {
    recalled: Vec<String>,
    injected: Vec<String>,
    from_cache: bool,
    context_hash: u64,
}

/// Records injections, ignoring any that are not this test's.
///
/// The observer slot is process-wide, so it sees EVERY injection in the test
/// binary — including the sibling injection tests running in parallel, which do
/// not take the lock above because they know nothing about observers. Filtering
/// on a per-test marker is what makes an assertion about "one injection" mean
/// this test's one injection.
///
/// That is a real property of the design rather than a testing inconvenience: a
/// process-wide sink observes the whole process.
struct Recorder {
    marker: String,
    seen: Mutex<Vec<Seen>>,
}

impl Recorder {
    fn new(marker: &str) -> Self {
        Self {
            marker: marker.to_string(),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn taken(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl InjectionObserver for Recorder {
    fn observed(&self, outcome: &InjectionOutcome<'_>) {
        if !outcome
            .recalled
            .iter()
            .any(|memory| memory.content.contains(&self.marker))
        {
            return;
        }
        let mut seen = self
            .seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        seen.push(Seen {
            recalled: outcome.recalled.iter().map(|m| m.content.clone()).collect(),
            injected: outcome.injected.iter().map(|m| m.content.clone()).collect(),
            from_cache: outcome.from_cache,
            context_hash: outcome.context_hash,
        });
    }
}

fn graph_with(contents: &[&str]) -> MemoryGraph {
    let graph = MemoryGraph::in_memory().expect("graph");
    for content in contents {
        graph
            .store_memory(content, "t", MemoryType::Fact, 0.6, &[], "test", "/p")
            .expect("store");
    }
    graph
}

#[test]
fn no_observer_is_the_default_and_injection_still_works() {
    let _guard = exclusive();
    clear_injection_observer();

    assert!(!has_injection_observer());
    let graph = graph_with(&["rust edition must be 2024"]);
    let mut injector = MemoryInjector::new();
    let block = injector
        .inject(&graph, &["rust edition".to_string()], 500)
        .expect("inject");

    assert!(block.contains("rust edition must be 2024"));
}

#[test]
fn a_real_injection_reports_what_reached_the_prompt() {
    let _guard = exclusive();
    let recorder = Arc::new(Recorder::new("markerreport"));
    set_injection_observer(Arc::clone(&recorder) as Arc<dyn InjectionObserver>);

    let graph = graph_with(&["markerreport edition must be 2024"]);
    let mut injector = MemoryInjector::new();
    let block = injector
        .inject(&graph, &["markerreport edition".to_string()], 5000)
        .expect("inject");

    clear_injection_observer();
    let seen = recorder.taken();
    assert_eq!(seen.len(), 1, "one injection, one observation");
    assert_eq!(
        seen[0].injected,
        vec!["markerreport edition must be 2024".to_string()]
    );
    assert!(!seen[0].from_cache);
    assert!(
        block.contains("markerreport edition must be 2024"),
        "the observation must describe the block the caller received"
    );
}

#[test]
fn a_memory_crowded_out_by_the_budget_is_recalled_but_not_injected() {
    // The distinction the reuse rate rests on. A memory that was retrieved and
    // then dropped for space is a miss, and reporting it as a hit would make
    // the rate describe relevance rather than use.
    let _guard = exclusive();
    let recorder = Arc::new(Recorder::new("markercrowded"));
    set_injection_observer(Arc::clone(&recorder) as Arc<dyn InjectionObserver>);

    let graph = graph_with(&[
        "markercrowded edition must be 2024 and this line is deliberately long enough to fill it",
        "markercrowded edition notes continue here with more text that will not fit at all",
    ]);
    let mut injector = MemoryInjector::new();
    // Enough for the header, the footer and one line only.
    let _ = injector
        .inject(&graph, &["markercrowded edition".to_string()], 34)
        .expect("inject");

    clear_injection_observer();
    let seen = recorder.taken();
    assert_eq!(seen.len(), 1);
    assert!(
        seen[0].recalled.len() > seen[0].injected.len(),
        "the budget should have dropped at least one recalled memory: {seen:#?}"
    );
    assert!(
        seen[0]
            .injected
            .iter()
            .all(|content| seen[0].recalled.contains(content)),
        "injected must be a subset of recalled"
    );
}

#[test]
fn a_cache_hit_is_reported_as_an_injection_too() {
    // The cached block still enters the prompt. Reporting only uncached calls
    // would make a memory used on every turn of a long conversation read as
    // used once.
    let _guard = exclusive();
    let recorder = Arc::new(Recorder::new("markercached"));
    set_injection_observer(Arc::clone(&recorder) as Arc<dyn InjectionObserver>);

    let graph = graph_with(&["markercached edition must be 2024"]);
    let mut injector = MemoryInjector::new();
    let context = vec!["markercached edition".to_string()];
    let first = injector.inject(&graph, &context, 5000).expect("first");
    let second = injector.inject(&graph, &context, 5000).expect("second");

    clear_injection_observer();
    let seen = recorder.taken();
    assert_eq!(first, second);
    assert_eq!(seen.len(), 2, "both injections must be observed");
    assert!(!seen[0].from_cache);
    assert!(seen[1].from_cache, "the second call came from the cache");
    assert_eq!(
        seen[0].context_hash, seen[1].context_hash,
        "one context, one identity -- so a metric store can dedupe the replay"
    );
    assert_eq!(seen[0].injected, seen[1].injected);
}

#[test]
fn an_injection_that_recalled_nothing_reports_nothing() {
    // No memory was considered, so no memory was passed over. Reporting it would
    // add rows to the denominator for turns where recall never ran.
    let _guard = exclusive();
    let recorder = Arc::new(Recorder::new("markerempty"));
    set_injection_observer(Arc::clone(&recorder) as Arc<dyn InjectionObserver>);

    let graph = MemoryGraph::in_memory().expect("graph");
    let mut injector = MemoryInjector::new();
    let _ = injector
        .inject(&graph, &["nothing matches this".to_string()], 500)
        .expect("inject");
    // And a context with no keywords at all.
    let _ = injector.inject(&graph, &[], 500).expect("inject");

    clear_injection_observer();
    assert!(recorder.taken().is_empty());
}

struct Panicking;

impl InjectionObserver for Panicking {
    fn observed(&self, _outcome: &InjectionOutcome<'_>) {
        panic!("observer blew up");
    }
}

#[test]
fn an_observer_that_panics_does_not_change_the_injection() {
    // A prompt must not change shape because telemetry had a bad day, and an
    // injection that succeeded must not become an error after the fact.
    let _guard = exclusive();
    let graph = graph_with(&["rust edition must be 2024"]);
    let mut injector = MemoryInjector::new();
    let context = vec!["rust edition".to_string()];

    clear_injection_observer();
    let without = injector.inject(&graph, &context, 5000).expect("baseline");

    injector.invalidate_cache();
    set_injection_observer(Arc::new(Panicking));
    let with = injector
        .inject(&graph, &context, 5000)
        .expect("a panicking observer must not fail the injection");
    clear_injection_observer();

    assert_eq!(with, without);
}

#[test]
fn installing_a_second_observer_replaces_the_first() {
    let _guard = exclusive();
    let first = Arc::new(Recorder::new("markerreplaced"));
    let second = Arc::new(Recorder::new("markerreplaced"));
    set_injection_observer(Arc::clone(&first) as Arc<dyn InjectionObserver>);
    set_injection_observer(Arc::clone(&second) as Arc<dyn InjectionObserver>);

    let graph = graph_with(&["markerreplaced edition must be 2024"]);
    let mut injector = MemoryInjector::new();
    let _ = injector
        .inject(&graph, &["markerreplaced edition".to_string()], 5000)
        .expect("inject");

    clear_injection_observer();
    assert!(first.taken().is_empty());
    assert_eq!(second.taken().len(), 1);
}
