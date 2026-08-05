use super::*;
use cozo::{DataValue, ScriptMutability};
use std::collections::BTreeMap;

/// Backdate a memory's `created_at` by `days`, so decay has something to bill.
fn age_memory(graph: &crate::MemoryGraph, id: &str, days: i64) {
    let created_at = (Utc::now() - chrono::Duration::days(days)).to_rfc3339();
    graph
        .db
        .run_script(
            "?[id, content, title, memory_type, importance, tags, source_type,
                project_path, created_at, updated_at, access_count, last_accessed] :=
                *memories{id, content, title, memory_type, importance, tags, source_type,
                    project_path, updated_at, access_count, last_accessed},
                id = $id, created_at = $created_at
             :put memories { id => content, title, memory_type, importance, tags, source_type,
                project_path, created_at, updated_at, access_count, last_accessed }",
            BTreeMap::from([
                ("id".to_string(), DataValue::from(id)),
                (
                    "created_at".to_string(),
                    DataValue::from(created_at.as_str()),
                ),
            ]),
            ScriptMutability::Mutable,
        )
        .expect("age memory");
}

#[test]
fn consolidation_retry_with_same_run_id_decays_once() {
    let graph = crate::MemoryGraph::in_memory().expect("create graph");
    let id = graph
        .store_memory("old fact", "", MemoryType::Fact, 50.0, &[], "test", "")
        .expect("store fact");
    age_memory(&graph, &id, 2);
    let config = GardenConfig {
        staleness_days: 365,
        importance_decay_per_day: 1.0,
        ..GardenConfig::default()
    };

    consolidate_with_run_id(&graph, &config, "session:test").expect("first run");
    consolidate_with_run_id(&graph, &config, "session:test").expect("retry run");

    assert_eq!(graph.read_memory(&id).expect("read fact").importance, 48.0);
}

/// Decay is charged per RUN, not per age.
///
/// `last_accessed` only moves when a memory is actually recalled, so billing
/// the whole span since it on every run bills the same span repeatedly and
/// decay compounds. Measured on the old code, three consecutive sessions took
/// a 2-day-old memory 50 -> 48 -> 46 -> 44. At the shipped 0.01/day that turns
/// a fifty-day slide to zero into a ten-day one, and deletes hand-written
/// memories -- stored at a fixed 0.5 and rarely re-accessed -- a month in.
#[test]
fn consecutive_sessions_do_not_compound_decay() {
    let graph = crate::MemoryGraph::in_memory().expect("create graph");
    let id = graph
        .store_memory("old fact", "", MemoryType::Fact, 50.0, &[], "test", "")
        .expect("store fact");
    age_memory(&graph, &id, 2);
    let config = GardenConfig {
        staleness_days: 365,
        importance_decay_per_day: 1.0,
        ..GardenConfig::default()
    };

    // First run has no previous run to bill against, so it catches up from
    // creation: 2 days at 1.0/day.
    consolidate_with_run_id(&graph, &config, "session:one").expect("first session");
    assert_eq!(
        graph.read_memory(&id).expect("read fact").importance,
        48.0,
        "the first run catches up from creation"
    );

    // Same day, so the second and third sessions owe nothing.
    consolidate_with_run_id(&graph, &config, "session:two").expect("second session");
    consolidate_with_run_id(&graph, &config, "session:three").expect("third session");
    assert_eq!(
        graph.read_memory(&id).expect("read fact").importance,
        48.0,
        "later runs the same day must charge nothing, not the full age again"
    );
}
