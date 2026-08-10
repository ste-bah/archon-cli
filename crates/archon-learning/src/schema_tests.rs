use super::*;

fn test_db() -> crate::cozo_guard::TestDb {
    crate::cozo_guard::test_sqlite_db_bare("test-learning-schema")
}

#[test]
fn test_ensure_schema_idempotent() {
    let db = test_db();
    ensure_learning_schema(&db).expect("first ensure must succeed");
    ensure_learning_schema(&db).expect("second ensure must succeed (idempotent)");
}

#[test]
fn test_learning_event_query_indices_exist() {
    let db = test_db();
    ensure_learning_schema(&db).expect("ensure schema");

    let result = db
        .run_script(
            "::indices learning_events",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .expect("list learning event indices");
    let name_column = result
        .headers
        .iter()
        .position(|header| header == "name")
        .expect("index listing includes name");
    let names: std::collections::HashSet<_> = result
        .rows
        .iter()
        .filter_map(|row| row[name_column].get_str())
        .collect();

    assert!(names.contains("by_created_at"));
    assert!(names.contains("by_type_created_at"));

    let by_time = db
        .run_script(
            "::explain { ?[event_id] := *learning_events:by_created_at{created_at, event_id}, created_at >= '2026-01-01T00:00:00Z' }",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .expect("explain time query");
    assert!(
        plan_uses_index(&by_time.rows, ":learning_events:by_created_at"),
        "time query plan: {:?}",
        by_time.rows,
    );

    let by_type_and_time = db
        .run_script(
            "::explain { ?[event_id] := *learning_events:by_type_created_at{event_type: 'GatePassed', created_at, event_id}, created_at >= '2026-01-01T00:00:00Z' }",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .expect("explain type and time query");
    assert!(plan_uses_index(
        &by_type_and_time.rows,
        ":learning_events:by_type_created_at",
    ));
}

fn plan_uses_index(rows: &[Vec<cozo::DataValue>], index: &str) -> bool {
    rows.iter()
        .any(|row| row.get(5).and_then(cozo::DataValue::get_str) == Some(index))
}

#[test]
fn test_relation_not_found_marker() {
    let db = test_db();
    let result = db.run_script(
        "?[event_id] := *nonexistent_xyz{event_id}",
        Default::default(),
        cozo::ScriptMutability::Immutable,
    );
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains(COZO_RELATION_NOT_FOUND),
        "Cozo error must contain COZO_RELATION_NOT_FOUND.\nActual: {msg}",
    );
}
