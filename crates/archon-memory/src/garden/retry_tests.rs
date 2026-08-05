use super::*;
use cozo::{DataValue, ScriptMutability};
use std::collections::BTreeMap;

#[test]
fn consolidation_retry_with_same_run_id_decays_once() {
    let graph = crate::MemoryGraph::in_memory().expect("create graph");
    let id = graph
        .store_memory("old fact", "", MemoryType::Fact, 50.0, &[], "test", "")
        .expect("store fact");
    let created_at = (Utc::now() - chrono::Duration::days(2)).to_rfc3339();
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
                ("id".to_string(), DataValue::from(id.as_str())),
                (
                    "created_at".to_string(),
                    DataValue::from(created_at.as_str()),
                ),
            ]),
            ScriptMutability::Mutable,
        )
        .expect("age fact");
    let config = GardenConfig {
        staleness_days: 365,
        importance_decay_per_day: 1.0,
        ..GardenConfig::default()
    };

    consolidate_with_run_id(&graph, &config, "session:test").expect("first run");
    consolidate_with_run_id(&graph, &config, "session:test").expect("retry run");

    assert_eq!(graph.read_memory(&id).expect("read fact").importance, 48.0);
}
