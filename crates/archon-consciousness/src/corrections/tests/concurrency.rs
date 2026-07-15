#[test]
fn concurrent_equivalent_corrections_create_one_derived_rule() {
    use std::sync::{Arc, Barrier};

    let graph = Arc::new(MemoryGraph::in_memory().expect("in-memory graph"));
    let barrier = Arc::new(Barrier::new(2));
    let requests = [
        (
            "correction:concurrent-one",
            "Use Edit before modifying config files",
        ),
        (
            "correction:concurrent-two",
            "  use   edit BEFORE modifying config files  ",
        ),
    ];

    let handles: Vec<_> = requests
        .into_iter()
        .map(|(correction_id, content)| {
            let graph = Arc::clone(&graph);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                CorrectionTracker::new(graph.as_ref()).record_correction_with_id(
                    correction_id,
                    CorrectionType::ApproachCorrection,
                    content,
                    "concurrent session",
                    None,
                )
            })
        })
        .collect();

    let corrections: Vec<_> = handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .expect("correction worker panicked")
                .expect("record")
        })
        .collect();
    let rule_id = corrections[0]
        .rule_id
        .as_deref()
        .expect("first derived rule");
    assert!(
        corrections
            .iter()
            .all(|correction| correction.rule_id.as_deref() == Some(rule_id)),
        "equivalent concurrent corrections must converge on one rule"
    );

    let rule = graph.get_memory(rule_id).expect("read shared rule");
    assert_eq!(
        rule.importance, 70.0,
        "both distinct corrections boost once"
    );
    assert_eq!(graph.memory_count().expect("count memories"), 3);
    assert_eq!(
        RulesEngine::new(graph.as_ref())
            .get_rules_sorted()
            .expect("list rules")
            .iter()
            .filter(|rule| rule.source == RuleSource::CorrectionDerived)
            .count(),
        1
    );
}

#[test]
fn concurrent_same_id_divergent_corrections_leave_no_losing_derived_rule() {
    use std::sync::Arc;

    let graph = Arc::new(MemoryGraph::in_memory().expect("in-memory graph"));
    let correction_id = "correction:concurrent-collision";
    let synchronized = Arc::new(SynchronizedMemory::new(Arc::clone(&graph), correction_id));
    let requests = [
        "Use Edit before modifying config files",
        "Use a dry run before modifying config files",
    ];

    let handles: Vec<_> = requests
        .into_iter()
        .map(|content| {
            let synchronized = Arc::clone(&synchronized);
            std::thread::spawn(move || {
                CorrectionTracker::new(synchronized.as_ref()).record_correction_with_id(
                    correction_id,
                    CorrectionType::ApproachCorrection,
                    content,
                    "concurrent session",
                    None,
                )
            })
        })
        .collect();
    let outcomes: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().expect("correction worker panicked"))
        .collect();

    assert_eq!(
        outcomes.iter().filter(|outcome| outcome.is_ok()).count(),
        1,
        "exactly one concurrent correction request claims the stable ID"
    );
    assert_eq!(
        outcomes.iter().filter(|outcome| outcome.is_err()).count(),
        1,
        "the incompatible request must fail the stable-ID collision"
    );

    let winning_correction = outcomes
        .iter()
        .find_map(|outcome| outcome.as_ref().ok())
        .expect("winning correction");
    let winning_rule_id = winning_correction
        .rule_id
        .as_deref()
        .expect("winning derived rule");
    assert_eq!(
        graph.memory_count().expect("count memories"),
        2,
        "only the winning correction and its derived rule remain"
    );
    let rules = RulesEngine::new(graph.as_ref())
        .get_rules_sorted()
        .expect("list rules");
    assert_eq!(
        rules
            .iter()
            .filter(|rule| rule.source == RuleSource::CorrectionDerived)
            .count(),
        1,
        "the losing request must not orphan a distinct derived rule"
    );
    assert_eq!(rules[0].id, winning_rule_id);
    assert_eq!(rules[0].score, 60.0, "only the winner applies one boost");
    let related = graph
        .get_related_memories(correction_id, 1)
        .expect("read correction relationship");
    assert_eq!(related.len(), 1, "only one caused-by relationship exists");
    assert_eq!(related[0].id, winning_rule_id);
}

#[test]
fn failed_non_owner_does_not_delete_winning_correction_or_provenance() {
    use std::sync::Arc;

    let graph = Arc::new(MemoryGraph::in_memory().expect("in-memory graph"));
    let correction_id = "correction:ownership-race";
    let memory = Arc::new(OwnershipRaceMemory::new(Arc::clone(&graph), correction_id));
    graph
        .store_memory_with_id(
            "rule:ownership-race",
            "Avoid ownership races",
            "",
            MemoryType::Rule,
            50.0,
            &["source:manual".into(), "trend:stable".into()],
            "test",
            "",
        )
        .expect("seed rule");

    let handles: Vec<_> = (0..2)
        .map(|_| {
            let memory = Arc::clone(&memory);
            std::thread::spawn(move || {
                CorrectionTracker::new(memory.as_ref()).record_correction_with_id(
                    correction_id,
                    CorrectionType::ApproachCorrection,
                    "Preserve correction ownership",
                    "concurrent session",
                    Some("rule:ownership-race"),
                )
            })
        })
        .collect();
    let outcomes: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().expect("correction worker panicked"))
        .collect();

    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_err()).count(), 1);
    let correction = graph
        .inspect_memory(correction_id)
        .expect("winning correction must survive losing failure");
    assert_eq!(correction.id, correction_id);
    let related = graph
        .get_related_memories(correction_id, 1)
        .expect("winning relationship must survive losing failure");
    assert_eq!(related.len(), 1);
    assert_eq!(related[0].id, "rule:ownership-race");
    assert!(graph
        .has_importance_application("rule:ownership-race", correction_id)
        .expect("winner provenance must survive"));
    assert_eq!(
        graph
            .inspect_memory("rule:ownership-race")
            .expect("winner score must survive")
            .importance,
        60.0
    );
}
