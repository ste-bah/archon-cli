//! Reuse rows, produced by a real injection and read back from the store.
//!
//! Every test here drives `MemoryInjector::inject` — the function the agent
//! calls on the prompt path — and then reads the cognitive metric relation.
//! Nothing calls the observer directly: an emitter proven only against a helper
//! is the failure this work exists to close.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use archon_cognitive::PersistentCognitiveStore;
use archon_cognitive::metrics::derive::derive_snapshot;
use archon_cognitive::metrics::event::CognitiveMetricEvent;
use archon_cognitive::metrics::event_store::MetricEventStore;
use archon_memory::garden::{
    SemanticConsolidationCandidate, apply_semantic_consolidation, rollback_semantic_consolidation,
};
use archon_memory::injection::{MemoryInjector, clear_injection_observer};
use archon_memory::types::MemoryType;
use archon_memory::{MemoryGraph, MemoryTrait};

use super::install;
use crate::command::garden_metrics::GardenMetricContext;

/// The injection observer is process-wide, so these must not overlap.
fn exclusive() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn metrics(dir: &std::path::Path) -> GardenMetricContext {
    GardenMetricContext {
        working_dir: dir.to_path_buf(),
        model_id: "test-model".to_string(),
        session_id: "session-1".to_string(),
        turn_number: 1,
    }
}

fn recorded(dir: &std::path::Path) -> Vec<CognitiveMetricEvent> {
    let root = dir.join(".archon").join("cognitive");
    let store = PersistentCognitiveStore::open(&root).expect("open store");
    MetricEventStore::new(store.db(), &root)
        .expect("event store")
        .events()
        .expect("read events")
}

/// Rows this metric wrote, as `(memory_id, cited)`.
fn reuse_rows(dir: &std::path::Path) -> Vec<(String, bool)> {
    recorded(dir)
        .into_iter()
        .filter(|event| event.identity("consolidated_memory") == Some("true"))
        .map(|event| {
            (
                event.identity("lesson_id").unwrap_or_default().to_string(),
                event.identity("cited") == Some("true"),
            )
        })
        .collect()
}

/// Store one ordinary memory and apply a consolidation over it, returning the
/// consolidated memory's id.
fn consolidate(graph: &MemoryGraph, content: &str) -> String {
    let source = graph
        .store_memory(content, "t", MemoryType::Fact, 0.5, &[], "extraction", "/p")
        .expect("store source");
    let candidate = SemanticConsolidationCandidate {
        candidate_id: format!("cand-{}", &source[..8.min(source.len())]),
        proposed_content: content.to_string(),
        proposed_title: "t".into(),
        memory_type: MemoryType::Fact,
        project_path: "/p".into(),
        source_type: "extraction".into(),
        proposed_importance: 0.8,
        representative_id: source.clone(),
        sources: vec![archon_memory::garden::ConsolidationSource {
            memory_id: source,
            excerpt: content.chars().take(80).collect(),
            importance: 0.5,
            created_at: chrono::Utc::now(),
        }],
    };
    let (derived, _) = apply_semantic_consolidation(graph, &candidate, "run-1").expect("apply");
    derived
}

#[test]
fn a_consolidated_memory_that_reaches_the_prompt_is_recorded_as_used() {
    let _guard = exclusive();
    let dir = tempfile::tempdir().expect("tempdir");
    let graph = MemoryGraph::in_memory().expect("graph");
    let derived = consolidate(
        &graph,
        "reusemarker always run the formatter before committing",
    );
    let memory: Arc<dyn MemoryTrait> = Arc::new(graph);

    install(Arc::clone(&memory), metrics(dir.path()));
    let mut injector = MemoryInjector::new();
    let block = injector
        .inject(
            memory.as_ref(),
            &["reusemarker formatter".to_string()],
            5000,
        )
        .expect("inject");
    clear_injection_observer();

    assert!(
        block.contains("reusemarker"),
        "the consolidated memory should have reached the prompt: {block}"
    );
    let rows = reuse_rows(dir.path());
    assert_eq!(rows, vec![(derived, true)], "one row, recorded as used");
}

#[test]
fn a_consolidated_memory_that_is_not_recalled_is_recorded_as_unused() {
    // The half that keeps the rate honest. Without it the denominator would be
    // whatever else happened to emit a retrieval row, and the rate would climb
    // toward 1.0 as each consolidation was used once.
    let _guard = exclusive();
    let dir = tempfile::tempdir().expect("tempdir");
    let graph = MemoryGraph::in_memory().expect("graph");
    let derived = consolidate(&graph, "zzzunrelated topic nobody will ask about");
    // Something else for the recall to actually find, so an injection happens.
    graph
        .store_memory(
            "missmarker deployment notes for the release",
            "t",
            MemoryType::Fact,
            0.9,
            &[],
            "test",
            "/p",
        )
        .expect("store");
    let memory: Arc<dyn MemoryTrait> = Arc::new(graph);

    install(Arc::clone(&memory), metrics(dir.path()));
    let mut injector = MemoryInjector::new();
    let block = injector
        .inject(
            memory.as_ref(),
            &["missmarker deployment".to_string()],
            5000,
        )
        .expect("inject");
    clear_injection_observer();

    assert!(
        !block.contains("zzzunrelated"),
        "the consolidated memory should NOT have been recalled: {block}"
    );
    let rows = reuse_rows(dir.path());
    assert_eq!(
        rows,
        vec![(derived, false)],
        "a consolidated memory that was not recalled must still be a row"
    );
}

#[test]
fn hits_and_misses_together_produce_a_rate_between_zero_and_one() {
    // The two cases in one population, derived through the real definition
    // rather than asserted on raw rows.
    let _guard = exclusive();
    let dir = tempfile::tempdir().expect("tempdir");
    let graph = MemoryGraph::in_memory().expect("graph");
    let used = consolidate(
        &graph,
        "bothmarker always run the formatter before committing",
    );
    let unused = consolidate(&graph, "qqqsilent topic that will never match a query");
    let memory: Arc<dyn MemoryTrait> = Arc::new(graph);

    install(Arc::clone(&memory), metrics(dir.path()));
    let mut injector = MemoryInjector::new();
    let _ = injector
        .inject(memory.as_ref(), &["bothmarker formatter".to_string()], 5000)
        .expect("inject");
    clear_injection_observer();

    let rows = reuse_rows(dir.path());
    assert_eq!(rows.len(), 2, "every consolidated memory is accounted for");
    assert!(rows.contains(&(used, true)));
    assert!(rows.contains(&(unused, false)));

    let snapshot = derive_snapshot(None, &recorded(dir.path()));
    let reuse = snapshot
        .pooled("consolidated_memory_reuse_rate")
        .expect("the metric must derive from these rows");
    assert_eq!(reuse.sample_count, 2);
    assert!((reuse.value.expect("value") - 0.5).abs() < 1e-9);
}

#[test]
fn reuse_rows_stay_out_of_the_lesson_metrics() {
    // `lesson_citation_rate` selects on `rule_injected = true`. These rows are
    // memories, not injected rules; folding them in would make one rate quietly
    // describe two different things.
    let _guard = exclusive();
    let dir = tempfile::tempdir().expect("tempdir");
    let graph = MemoryGraph::in_memory().expect("graph");
    consolidate(&graph, "isolationmarker always run the formatter");
    let memory: Arc<dyn MemoryTrait> = Arc::new(graph);

    install(Arc::clone(&memory), metrics(dir.path()));
    let mut injector = MemoryInjector::new();
    let _ = injector
        .inject(
            memory.as_ref(),
            &["isolationmarker formatter".to_string()],
            5000,
        )
        .expect("inject");
    clear_injection_observer();

    assert!(
        recorded(dir.path())
            .iter()
            .all(|event| event.identity("rule_injected") == Some("false")),
        "a reuse row claimed to be an injected rule"
    );
    let snapshot = derive_snapshot(None, &recorded(dir.path()));
    assert!(
        snapshot.pooled("lesson_citation_rate").is_none(),
        "reuse rows leaked into the lesson citation population"
    );
}

#[test]
fn a_store_with_no_consolidated_memories_writes_nothing() {
    let _guard = exclusive();
    let dir = tempfile::tempdir().expect("tempdir");
    let graph = MemoryGraph::in_memory().expect("graph");
    graph
        .store_memory(
            "plainmarker deployment notes",
            "t",
            MemoryType::Fact,
            0.9,
            &[],
            "test",
            "/p",
        )
        .expect("store");
    let memory: Arc<dyn MemoryTrait> = Arc::new(graph);

    install(Arc::clone(&memory), metrics(dir.path()));
    let mut injector = MemoryInjector::new();
    let _ = injector
        .inject(
            memory.as_ref(),
            &["plainmarker deployment".to_string()],
            5000,
        )
        .expect("inject");
    clear_injection_observer();

    assert!(reuse_rows(dir.path()).is_empty());
}

#[test]
fn repeating_one_prompt_does_not_add_a_second_row() {
    // One prompt situation is one observation. Counting a cached re-injection
    // again would let a long conversation inflate whichever way the first
    // observation happened to fall.
    let _guard = exclusive();
    let dir = tempfile::tempdir().expect("tempdir");
    let graph = MemoryGraph::in_memory().expect("graph");
    consolidate(&graph, "repeatmarker always run the formatter");
    let memory: Arc<dyn MemoryTrait> = Arc::new(graph);

    install(Arc::clone(&memory), metrics(dir.path()));
    let mut injector = MemoryInjector::new();
    let context = vec!["repeatmarker formatter".to_string()];
    let _ = injector
        .inject(memory.as_ref(), &context, 5000)
        .expect("first");
    let _ = injector
        .inject(memory.as_ref(), &context, 5000)
        .expect("cached");
    // And after an invalidation, which makes the next call a genuine cache miss
    // over the same context.
    injector.invalidate_cache();
    let _ = injector
        .inject(memory.as_ref(), &context, 5000)
        .expect("re-run");
    clear_injection_observer();

    assert_eq!(
        reuse_rows(dir.path()).len(),
        1,
        "one prompt context, one observation per consolidated memory"
    );
}

#[test]
fn a_rolled_back_consolidation_stops_being_counted() {
    // The denominator is re-read per injection precisely so this holds. A cached
    // list would keep counting a withdrawn memory as unused for ever, dragging
    // the rate down on the strength of something that no longer exists.
    let _guard = exclusive();
    let dir = tempfile::tempdir().expect("tempdir");
    let graph = MemoryGraph::in_memory().expect("graph");
    let derived = consolidate(&graph, "rollbackmarker always run the formatter");
    graph
        .store_memory(
            "othermarker deployment notes",
            "t",
            MemoryType::Fact,
            0.9,
            &[],
            "test",
            "/p",
        )
        .expect("store");
    rollback_semantic_consolidation(&graph, &derived).expect("rollback");
    let memory: Arc<dyn MemoryTrait> = Arc::new(graph);

    install(Arc::clone(&memory), metrics(dir.path()));
    let mut injector = MemoryInjector::new();
    let _ = injector
        .inject(
            memory.as_ref(),
            &["othermarker deployment".to_string()],
            5000,
        )
        .expect("inject");
    clear_injection_observer();

    assert!(
        reuse_rows(dir.path()).is_empty(),
        "a withdrawn consolidation must leave the denominator"
    );
}
