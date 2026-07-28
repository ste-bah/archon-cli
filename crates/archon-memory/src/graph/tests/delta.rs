use super::*;

#[test]
fn tag_policy_accepts_sixteen_nontrend_tags_and_one_trend_tag() {
    let graph = make_graph();
    let mut tags: Vec<String> = (0..16).map(|index| format!("tag:{index}")).collect();
    tags.push("trend:stable".into());

    let id = graph
        .store_memory("rule", "", MemoryType::Rule, 50.0, &tags, "test", "")
        .expect("store max non-trend tags plus trend");

    assert_eq!(graph.read_memory(&id).expect("read stored tags").tags, tags);
}

#[test]
fn tag_policy_rejects_duplicate_trends_and_more_than_sixteen_nontrend_tags() {
    let graph = make_graph();
    let duplicate_trends = vec!["tag".into(), "trend:stable".into(), "trend:rising".into()];
    assert!(
        graph
            .store_memory(
                "rule",
                "",
                MemoryType::Rule,
                50.0,
                &duplicate_trends,
                "test",
                ""
            )
            .is_err()
    );

    let too_many_nontrend: Vec<String> = (0..17).map(|index| format!("tag:{index}")).collect();
    assert!(
        graph
            .store_memory(
                "rule",
                "",
                MemoryType::Rule,
                50.0,
                &too_many_nontrend,
                "test",
                ""
            )
            .is_err()
    );
}

#[test]
fn atomic_delta_preserves_sixteen_nontrend_tags_and_replaces_the_one_trend_tag() {
    let graph = make_graph();
    let mut tags: Vec<String> = (0..16).map(|index| format!("tag:{index}")).collect();
    tags.push("trend:stable".into());
    let id = graph
        .store_memory("rule", "", MemoryType::Rule, 50.0, &tags, "test", "")
        .expect("store max tags");

    let updated = graph
        .apply_importance_delta(&id, 10.0, "max-tags-delta")
        .expect("apply delta");

    assert_eq!(updated.importance, 60.0);
    for index in 0..16 {
        assert!(updated.tags.contains(&format!("tag:{index}")));
    }
    assert_eq!(updated.tags.len(), 17);
    assert_eq!(
        updated
            .tags
            .iter()
            .filter(|tag| tag.starts_with("trend:"))
            .count(),
        1
    );
    assert!(updated.tags.contains(&"trend:rising".to_string()));
}

#[test]
fn atomic_delta_rejects_legacy_rows_over_tag_capacity_without_mutation() {
    let graph = make_graph();
    let id = graph
        .store_memory("rule", "", MemoryType::Rule, 50.0, &[], "test", "")
        .expect("store rule");
    let legacy_tags: Vec<String> = (0..18).map(|index| format!("legacy:{index}")).collect();
    graph
        .db
        .run_script(
            "?[id, content, title, memory_type, importance, tags, source_type,
                project_path, created_at, updated_at, access_count, last_accessed] :=
                *memories{id, content, title, memory_type, importance, source_type,
                    project_path, created_at, updated_at, access_count, last_accessed},
                id = $id, tags = $tags
             :put memories { id => content, title, memory_type, importance, tags, source_type,
                project_path, created_at, updated_at, access_count, last_accessed }",
            std::collections::BTreeMap::from([
                ("id".to_string(), cozo::DataValue::from(id.as_str())),
                (
                    "tags".to_string(),
                    cozo::DataValue::from(
                        serde_json::to_string(&legacy_tags).expect("serialize tags"),
                    ),
                ),
            ]),
            cozo::ScriptMutability::Mutable,
        )
        .expect("seed legacy row");

    assert!(
        graph
            .apply_importance_delta(&id, 10.0, "legacy-over-cap")
            .is_err()
    );
    let stored = graph.read_memory(&id).expect("read unchanged row");
    assert_eq!(stored.importance, 50.0);
    assert_eq!(stored.tags, legacy_tags);
}

#[test]
fn atomic_delta_rejects_legacy_rows_with_seventeen_nontrend_tags() {
    let graph = make_graph();
    let id = graph
        .store_memory("rule", "", MemoryType::Rule, 50.0, &[], "test", "")
        .expect("store rule");
    let legacy_tags: Vec<String> = (0..17).map(|index| format!("legacy:{index}")).collect();
    graph
        .db
        .run_script(
            "?[id, content, title, memory_type, importance, tags, source_type,
                project_path, created_at, updated_at, access_count, last_accessed] :=
                *memories{id, content, title, memory_type, importance, source_type,
                    project_path, created_at, updated_at, access_count, last_accessed},
                id = $id, tags = $tags
             :put memories { id => content, title, memory_type, importance, tags, source_type,
                project_path, created_at, updated_at, access_count, last_accessed }",
            std::collections::BTreeMap::from([
                ("id".to_string(), cozo::DataValue::from(id.as_str())),
                (
                    "tags".to_string(),
                    cozo::DataValue::from(
                        serde_json::to_string(&legacy_tags).expect("serialize tags"),
                    ),
                ),
            ]),
            cozo::ScriptMutability::Mutable,
        )
        .expect("seed legacy row");

    assert!(
        graph
            .apply_importance_delta(&id, 10.0, "legacy-nontrend-over-cap")
            .is_err()
    );
    let stored = graph.read_memory(&id).expect("read unchanged row");
    assert_eq!(stored.importance, 50.0);
    assert_eq!(stored.tags, legacy_tags);
}

#[test]
fn atomic_delta_is_additive_under_concurrency() {
    let graph = Arc::new(make_graph());
    let id = graph
        .store_memory("rule", "", MemoryType::Rule, 50.0, &[], "test", "")
        .expect("store rule");
    let barrier = Arc::new(Barrier::new(3));
    let (tx, rx) = mpsc::channel();

    for provenance_id in ["first", "second"] {
        let graph = Arc::clone(&graph);
        let barrier = Arc::clone(&barrier);
        let id = id.clone();
        let tx = tx.clone();
        thread::spawn(move || {
            barrier.wait();
            tx.send(graph.apply_importance_delta(&id, 10.0, provenance_id))
                .expect("send result");
        });
    }

    barrier.wait();
    for _ in 0..2 {
        rx.recv().expect("receive result").expect("apply delta");
    }

    assert_eq!(graph.get_memory(&id).expect("read rule").importance, 70.0);
}

#[test]
fn atomic_delta_is_idempotent_by_provenance() {
    let graph = make_graph();
    let id = graph
        .store_memory("rule", "", MemoryType::Rule, 50.0, &[], "test", "")
        .expect("store rule");

    graph
        .apply_importance_delta(&id, 10.0, "correction-1")
        .expect("first apply");
    let retry = graph
        .apply_importance_delta(&id, 10.0, "correction-1")
        .expect("retry");

    assert_eq!(retry.importance, 60.0);
    assert_eq!(graph.get_memory(&id).expect("read rule").importance, 60.0);
}

#[test]
fn atomic_delta_applies_duplicate_concurrent_provenance_once() {
    let graph = Arc::new(make_graph());
    let id = graph
        .store_memory("rule", "", MemoryType::Rule, 50.0, &[], "test", "")
        .expect("store rule");
    let barrier = Arc::new(Barrier::new(3));
    let (tx, rx) = mpsc::channel();

    for _ in 0..2 {
        let graph = Arc::clone(&graph);
        let barrier = Arc::clone(&barrier);
        let id = id.clone();
        let tx = tx.clone();
        thread::spawn(move || {
            barrier.wait();
            tx.send(graph.apply_importance_delta(&id, 10.0, "correction-1"))
                .expect("send result");
        });
    }

    barrier.wait();
    for _ in 0..2 {
        rx.recv().expect("receive result").expect("apply delta");
    }

    assert_eq!(graph.get_memory(&id).expect("read rule").importance, 60.0);
}

#[test]
fn atomic_delta_missing_memory_does_not_write_ledger_and_allows_later_creation() {
    let graph = make_graph();
    let missing_id = "missing-memory";

    let error = graph
        .apply_importance_delta(missing_id, 10.0, "same-provenance")
        .expect_err("missing memory must fail");
    assert!(matches!(error, MemoryError::NotFound(_)));
    let ledger = graph
        .db
        .run_script(
            "?[memory_id, provenance_id] := *score_applications{memory_id, provenance_id}, memory_id = $id",
            std::collections::BTreeMap::from([(
                "id".to_string(),
                cozo::DataValue::from(missing_id),
            )]),
            cozo::ScriptMutability::Immutable,
        )
        .expect("read ledger");
    assert!(
        ledger.rows.is_empty(),
        "missing IDs must not create ledger rows"
    );

    let created_id = graph
        .store_memory("rule", "", MemoryType::Rule, 50.0, &[], "test", "")
        .expect("create memory after failed delta");
    let updated = graph
        .apply_importance_delta(&created_id, 10.0, "same-provenance")
        .expect("same provenance applies to an existing memory");
    assert_eq!(updated.importance, 60.0);
}

#[test]
fn access_metadata_and_delta_stress_preserve_current_score_tags_and_provenance() {
    const ITERATIONS: usize = 64;

    let graph = Arc::new(make_graph());
    let id = graph
        .store_memory(
            "rule",
            "",
            MemoryType::Rule,
            0.0,
            &["unrelated".into(), "trend:stable".into()],
            "test",
            "",
        )
        .expect("store rule");

    for iteration in 0..ITERATIONS {
        let barrier = Arc::new(Barrier::new(3));
        let (tx, rx) = mpsc::channel();
        for operation in 0..2 {
            let graph = Arc::clone(&graph);
            let barrier = Arc::clone(&barrier);
            let id = id.clone();
            let tx = tx.clone();
            thread::spawn(move || {
                barrier.wait();
                let result = if operation == 0 {
                    graph.get_memory(&id).map(|_| ())
                } else {
                    graph
                        .apply_importance_delta(&id, 1.0, &format!("access-race-{iteration}"))
                        .map(|_| ())
                };
                tx.send(result).expect("send result");
            });
        }
        barrier.wait();
        for _ in 0..2 {
            rx.recv()
                .expect("receive result")
                .expect("concurrent operation");
        }
    }

    let stored = graph.read_memory(&id).expect("read source of truth");
    assert_eq!(stored.importance, ITERATIONS as f64);
    assert!(stored.tags.iter().any(|tag| tag == "unrelated"));
    assert_eq!(
        stored
            .tags
            .iter()
            .filter(|tag| tag.starts_with("trend:"))
            .count(),
        1
    );
    let ledger = graph
        .db
        .run_script(
            "?[provenance_id] := *score_applications{memory_id, provenance_id}, memory_id = $id",
            std::collections::BTreeMap::from([(
                "id".to_string(),
                cozo::DataValue::from(id.as_str()),
            )]),
            cozo::ScriptMutability::Immutable,
        )
        .expect("read ledger");
    assert_eq!(ledger.rows.len(), ITERATIONS);
}

#[test]
fn atomic_delta_tag_updates_have_two_valid_serial_orders() {
    for delta_last in [false, true] {
        let graph = make_graph();
        let id = graph
            .store_memory(
                "rule",
                "",
                MemoryType::Rule,
                50.0,
                &["existing".into(), "trend:stable".into()],
                "test",
                "",
            )
            .expect("store rule");
        let replacement = ["existing".into(), "new-tag".into(), "trend:stable".into()];

        if delta_last {
            graph
                .update_memory(&id, None, Some(&replacement))
                .expect("update tags");
            graph
                .apply_importance_delta(&id, 10.0, "serial-delta-last")
                .expect("apply delta");
        } else {
            graph
                .apply_importance_delta(&id, 10.0, "serial-update-last")
                .expect("apply delta");
            graph
                .update_memory(&id, None, Some(&replacement))
                .expect("update tags");
        }

        let stored = graph.read_memory(&id).expect("read source of truth");
        assert!(stored.tags.iter().any(|tag| tag == "existing"));
        assert!(stored.tags.iter().any(|tag| tag == "new-tag"));
        let trends: Vec<_> = stored
            .tags
            .iter()
            .filter(|tag| tag.starts_with("trend:"))
            .collect();
        assert_eq!(trends.len(), 1);
        assert_eq!(
            trends[0],
            if delta_last {
                "trend:rising"
            } else {
                "trend:stable"
            }
        );
    }
}

#[test]
fn atomic_delta_and_tag_updates_preserve_concurrent_invariants() {
    const ITERATIONS: usize = 32;

    for iteration in 0..ITERATIONS {
        let graph = Arc::new(make_graph());
        let id = graph
            .store_memory(
                "rule",
                "",
                MemoryType::Rule,
                50.0,
                &["existing".into(), "trend:stable".into()],
                "test",
                "",
            )
            .expect("store rule");
        let barrier = Arc::new(Barrier::new(3));
        let (tx, rx) = mpsc::channel();

        {
            let graph = Arc::clone(&graph);
            let barrier = Arc::clone(&barrier);
            let id = id.clone();
            let tx = tx.clone();
            thread::spawn(move || {
                barrier.wait();
                tx.send(graph.update_memory(
                    &id,
                    None,
                    Some(&["existing".into(), "new-tag".into(), "trend:stable".into()]),
                ))
                .expect("send tag update result");
            });
        }

        {
            let graph = Arc::clone(&graph);
            let barrier = Arc::clone(&barrier);
            let id = id.clone();
            thread::spawn(move || {
                barrier.wait();
                tx.send(
                    graph
                        .apply_importance_delta(&id, 10.0, &format!("concurrent-tag-{iteration}"))
                        .map(|_| ()),
                )
                .expect("send delta result");
            });
        }

        barrier.wait();
        for _ in 0..2 {
            rx.recv()
                .expect("receive result")
                .expect("concurrent mutation");
        }

        let stored = graph.read_memory(&id).expect("read source of truth");
        assert_eq!(stored.importance, 60.0);
        assert!(stored.tags.iter().any(|tag| tag == "existing"));
        assert!(stored.tags.iter().any(|tag| tag == "new-tag"));
        let trends: Vec<_> = stored
            .tags
            .iter()
            .filter(|tag| tag.starts_with("trend:"))
            .collect();
        assert_eq!(trends.len(), 1);
        assert!(matches!(
            trends[0].as_str(),
            "trend:stable" | "trend:rising"
        ));
    }
}

#[test]
fn atomic_trend_reconciliation_uses_authoritative_score() {
    let graph = make_graph();
    let id = graph
        .store_memory(
            "rule",
            "",
            MemoryType::Rule,
            50.0,
            &["existing".into(), "trend:stable".into()],
            "test",
            "",
        )
        .expect("store rule");
    graph
        .apply_importance_delta(&id, -20.0, "fixture:lower-before-reconcile")
        .expect("lower score");

    let reconciled = graph
        .reconcile_importance_trend(&id, 40.0)
        .expect("reconcile trend");

    assert_eq!(reconciled.importance, 30.0);
    assert!(reconciled.tags.contains(&"existing".to_string()));
    assert!(reconciled.tags.contains(&"trend:declining".to_string()));
}

#[test]
fn atomic_delta_rejects_nan_inf_and_empty_provenance() {
    let graph = make_graph();
    let id = graph
        .store_memory("rule", "", MemoryType::Rule, 50.0, &[], "test", "")
        .expect("store rule");

    for delta in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(graph.apply_importance_delta(&id, delta, "valid").is_err());
    }
    assert!(graph.apply_importance_delta(&id, 10.0, "").is_err());
    assert_eq!(graph.get_memory(&id).expect("read rule").importance, 50.0);
}
