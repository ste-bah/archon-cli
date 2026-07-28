#[test]
fn concurrent_correction_boosts_are_additive() {
    use std::sync::{Arc, Barrier};

    let graph = Arc::new(MemoryGraph::in_memory().expect("in-memory graph"));
    let engine = RulesEngine::new(graph.as_ref());
    let rule = engine
        .add_rule("keep user data safe", RuleSource::CorrectionDerived)
        .expect("add rule");
    let barrier = Arc::new(Barrier::new(2));

    std::thread::scope(|scope| {
        for provenance_id in ["correction:one", "correction:two"] {
            let graph = Arc::clone(&graph);
            let barrier = Arc::clone(&barrier);
            let rule_id = rule.id.clone();
            scope.spawn(move || {
                barrier.wait();
                RulesEngine::new(graph.as_ref())
                    .boost_rule_by(&rule_id, 10.0, provenance_id)
                    .expect("boost rule");
            });
        }
    });

    assert_eq!(graph.get_memory(&rule.id).expect("rule").importance, 70.0);
}

#[test]
fn boost_and_decay_racing_both_apply_once() {
    use std::sync::{Arc, Barrier};

    let graph = Arc::new(MemoryGraph::in_memory().expect("in-memory graph"));
    let engine = RulesEngine::new(graph.as_ref());
    let rule = engine
        .add_rule("keep user data safe", RuleSource::CorrectionDerived)
        .expect("add rule");
    let barrier = Arc::new(Barrier::new(2));

    std::thread::scope(|scope| {
        let boost_graph = Arc::clone(&graph);
        let boost_barrier = Arc::clone(&barrier);
        let boost_rule_id = rule.id.clone();
        scope.spawn(move || {
            boost_barrier.wait();
            RulesEngine::new(boost_graph.as_ref())
                .boost_rule_by(&boost_rule_id, 10.0, "correction:one")
                .expect("boost rule");
        });

        let decay_graph = Arc::clone(&graph);
        let decay_barrier = Arc::clone(&barrier);
        scope.spawn(move || {
            decay_barrier.wait();
            RulesEngine::new(decay_graph.as_ref())
                .decay_scores(5.0)
                .expect("decay rules");
        });
    });

    assert_eq!(graph.get_memory(&rule.id).expect("rule").importance, 55.0);
}

#[test]
fn import_scores_sets_snapshot_target_idempotently_and_preserves_source_tag() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);
    let rule = engine
        .add_rule("keep user data safe", RuleSource::UserDefined)
        .expect("add rule");
    engine
        .boost_rule_by(&rule.id, 10.0, "correction:one")
        .expect("boost rule");
    let snapshot = crate::persistence::RuleScoreEntry {
        rule_id: rule.id.clone(),
        rule_text: rule.text.clone(),
        score: 40.0,
    };

    assert_eq!(
        engine.import_scores(std::slice::from_ref(&snapshot)).expect("import"),
        1
    );
    let imported = graph.get_memory(&rule.id).expect("read imported rule");
    assert_eq!(imported.importance, 40.0);
    assert!(imported.tags.contains(&RuleSource::UserDefined.as_tag()));
    assert!(imported.tags.contains(&Trend::Declining.as_tag()));
    assert_eq!(engine.import_scores(&[snapshot]).expect("retry import"), 1);
    assert_eq!(
        graph.get_memory(&rule.id).expect("read retry").importance,
        40.0
    );
}

#[test]
fn import_delta_preserves_boost_applied_after_snapshot_read() {
    let graph = Arc::new(MemoryGraph::in_memory().expect("in-memory graph"));
    let rule = RulesEngine::new(graph.as_ref())
        .add_rule("keep user data safe", RuleSource::UserDefined)
        .expect("add rule");
    let search_completed = Arc::new(Barrier::new(2));
    let resume_import = Arc::new(Barrier::new(2));
    let memory = BlockingSearchMemory {
        graph: Arc::clone(&graph),
        search_completed: Arc::clone(&search_completed),
        resume_import: Arc::clone(&resume_import),
    };
    let snapshot = crate::persistence::RuleScoreEntry {
        rule_id: rule.id.clone(),
        rule_text: rule.text.clone(),
        score: 40.0,
    };

    std::thread::scope(|scope| {
        scope.spawn(|| {
            RulesEngine::new(&memory)
                .import_scores(&[snapshot])
                .expect("import score");
        });
        search_completed.wait();
        RulesEngine::new(graph.as_ref())
            .boost_rule_by(&rule.id, 10.0, "correction:one")
            .expect("boost rule");
        resume_import.wait();
    });

    assert_eq!(
        graph.get_memory(&rule.id).expect("read rule").importance,
        50.0
    );
}

#[test]
fn update_rule_text() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);

    let rule = engine
        .add_rule("old text", RuleSource::UserDefined)
        .expect("add");
    engine.update_rule(&rule.id, "new text").expect("update");

    let rules = engine.get_rules_sorted().expect("list");
    assert_eq!(rules[0].text, "new text");
}
