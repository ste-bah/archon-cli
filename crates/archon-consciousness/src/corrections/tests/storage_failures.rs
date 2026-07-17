#[test]
fn severity_multipliers_are_ordered() {
    assert!(
        CorrectionType::FactualError.severity_multiplier()
            < CorrectionType::ApproachCorrection.severity_multiplier()
    );
    assert!(
        CorrectionType::ApproachCorrection.severity_multiplier()
            < CorrectionType::RepeatedInstruction.severity_multiplier()
    );
    assert!(
        CorrectionType::RepeatedInstruction.severity_multiplier()
            < CorrectionType::DidForbiddenAction.severity_multiplier()
    );
    assert!(
        CorrectionType::DidForbiddenAction.severity_multiplier()
            < CorrectionType::ActedWithoutPermission.severity_multiplier()
    );
}

#[test]
fn invalid_explicit_rule_does_not_store_a_correction() {
    let (graph, _) = make_tracker();
    let tracker = CorrectionTracker::new(&graph);

    let result = tracker.record_correction(
        CorrectionType::ApproachCorrection,
        "Use the safe path",
        "test",
        Some("missing-rule"),
    );

    assert!(result.is_err(), "missing explicit rule must fail");
    assert_eq!(graph.memory_count().expect("count memories"), 0);
}

#[test]
fn explicit_rule_lookup_error_propagates_and_compensates_new_correction() {
    let (graph, _) = make_tracker();
    let failing_graph = FailingMemory {
        inner: &graph,
        failure_point: FailurePoint::ExplicitRuleLookup,
    };
    let tracker = CorrectionTracker::new(&failing_graph);

    let error = tracker
        .record_correction_with_id(
            "correction:lookup-failure",
            CorrectionType::ApproachCorrection,
            "Use the safe path",
            "test",
            Some("rule-lookup-failure"),
        )
        .expect_err("infrastructure lookup failure must surface");

    assert!(matches!(
        error,
        CorrectionError::Memory(MemoryError::Database(message))
            if message == "injected explicit-rule lookup failure"
    ));
    assert!(matches!(
        graph.inspect_memory("correction:lookup-failure"),
        Err(MemoryError::NotFound(_))
    ));
    assert_eq!(graph.memory_count().expect("count unchanged graph"), 0);
}

#[test]
fn score_update_failure_removes_only_correction_and_keeps_derived_rule() {
    let (graph, _) = make_tracker();
    let failing_graph = FailingMemory {
        inner: &graph,
        failure_point: FailurePoint::ScoreUpdate,
    };
    let tracker = CorrectionTracker::new(&failing_graph);

    let result = tracker.record_correction(
        CorrectionType::ApproachCorrection,
        "Use the safe path",
        "test",
        None,
    );

    assert!(
        result.is_err(),
        "injected score update failure must surface"
    );
    let rules = RulesEngine::new(&graph)
        .get_rules_sorted()
        .expect("list derived rule");
    assert_eq!(
        rules.len(),
        1,
        "compensation must retain the shared derived rule"
    );
    assert_eq!(rules[0].source, RuleSource::CorrectionDerived);
    assert_eq!(
        rules[0].score, 50.0,
        "failed correction must not boost rule"
    );
    assert_eq!(graph.memory_count().expect("count memories"), 1);
}

#[test]
fn score_update_failure_removes_correction_relationship_from_existing_rule() {
    let (graph, _) = make_tracker();
    let rule = RulesEngine::new(&graph)
        .add_rule("Ask before modifying files", RuleSource::UserDefined)
        .expect("add existing rule");
    let failing_graph = FailingMemory {
        inner: &graph,
        failure_point: FailurePoint::ScoreUpdate,
    };
    let tracker = CorrectionTracker::new(&failing_graph);

    let result = tracker.record_correction(
        CorrectionType::ApproachCorrection,
        "Use the safe path",
        "test",
        Some(&rule.id),
    );

    assert!(
        result.is_err(),
        "injected score update failure must surface"
    );
    assert_eq!(graph.memory_count().expect("count memories"), 1);
    assert!(
        graph
            .get_related_memories(&rule.id, 1)
            .expect("read existing rule relationships")
            .is_empty(),
        "failed correction must not leave a relationship on its existing rule"
    );
}

#[test]
fn correction_store_failure_keeps_deterministic_derived_rule() {
    let (graph, _) = make_tracker();
    let derived_rule_id = CORRECTION_DERIVED_RULE_ID;
    RulesEngine::new(&graph)
        .add_rule_with_id(
            derived_rule_id,
            CORRECTION_DERIVED_RULE_TEXT,
            RuleSource::CorrectionDerived,
        )
        .expect("seed deterministic rule");
    let failing_graph = FailingMemory {
        inner: &graph,
        failure_point: FailurePoint::CorrectionStore,
    };
    let tracker = CorrectionTracker::new(&failing_graph);

    let result = tracker.record_correction(
        CorrectionType::ApproachCorrection,
        "Use the safe path",
        "test",
        None,
    );

    assert!(
        result.is_err(),
        "injected correction-store failure must surface"
    );
    let rules = RulesEngine::new(&graph)
        .get_rules_sorted()
        .expect("list derived rule");
    assert_eq!(
        rules.len(),
        1,
        "failed correction storage must retain deterministic rule"
    );
    assert_eq!(rules[0].score, 50.0);
    assert_eq!(graph.memory_count().expect("count memories"), 1);
}
