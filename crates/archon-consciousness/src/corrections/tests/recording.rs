#[test]
fn record_correction_with_existing_rule() {
    let (graph, _) = make_tracker();
    let tracker = CorrectionTracker::new(&graph);

    // Create a rule first.
    let rules = RulesEngine::new(&graph);
    let rule = rules
        .add_rule("Always ask before modifying files", RuleSource::UserDefined)
        .expect("add_rule");
    let original_score = rule.score; // 50.0

    let correction = tracker
        .record_correction(
            CorrectionType::ActedWithoutPermission,
            "Modified config.toml without asking",
            "editing session",
            Some(&rule.id),
        )
        .expect("record_correction");

    assert_eq!(
        correction.correction_type,
        CorrectionType::ActedWithoutPermission
    );
    assert!((correction.severity - 5.0).abs() < f64::EPSILON);
    assert_eq!(correction.rule_id.as_deref(), Some(rule.id.as_str()));

    // Rule score should have been boosted by 5.0 * 5.0 = 25.0
    let updated = graph.get_memory(&rule.id).expect("get rule");
    let expected = original_score + 25.0;
    assert!(
        (updated.importance - expected).abs() < f64::EPSILON,
        "expected {expected}, got {}",
        updated.importance,
    );
    let related = graph
        .get_related_memories(&correction.id, 1)
        .expect("read correction relationship");
    assert!(related.iter().any(|memory| memory.id == rule.id));
}

#[test]
fn record_correction_auto_creates_rule() {
    let (graph, _) = make_tracker();
    let tracker = CorrectionTracker::new(&graph);

    let correction = tracker
        .record_correction(
            CorrectionType::FactualError,
            "Stated Rust 2024 edition does not exist",
            "research session",
            None,
        )
        .expect("record_correction");

    // A rule should have been auto-created.
    assert!(correction.rule_id.is_some());

    let rule_id = correction.rule_id.as_ref().expect("rule_id");
    let rule_mem = graph.get_memory(rule_id).expect("get auto-rule");
    assert_eq!(rule_mem.content, CORRECTION_DERIVED_RULE_TEXT);

    // Rule score should be boosted from 50.0 by 1.5 * 5.0 = 7.5
    let expected = 50.0 + 7.5;
    assert!(
        (rule_mem.importance - expected).abs() < f64::EPSILON,
        "expected {expected}, got {}",
        rule_mem.importance,
    );
    let related = graph
        .get_related_memories(&correction.id, 1)
        .expect("read correction relationship");
    assert!(related.iter().any(|memory| memory.id == *rule_id));
}

#[test]
fn automatic_corrections_share_versioned_generic_rule() {
    let (graph, _) = make_tracker();
    let tracker = CorrectionTracker::new(&graph);

    let first = tracker
        .record_correction(
            CorrectionType::FactualError,
            "Stated Rust 2024 edition does not exist",
            "research session",
            None,
        )
        .expect("record first correction");
    let second = tracker
        .record_correction(
            CorrectionType::ActedWithoutPermission,
            "Deleted a production database without approval",
            "deployment session",
            None,
        )
        .expect("record second correction");

    assert_eq!(
        first.rule_id.as_deref(),
        Some(CORRECTION_DERIVED_RULE_ID),
        "automatic corrections use the versioned generic rule"
    );
    assert_eq!(
        second.rule_id.as_deref(),
        Some(CORRECTION_DERIVED_RULE_ID),
        "dissimilar automatic corrections must not add duplicate generic prompts"
    );
    assert_eq!(
        RulesEngine::new(&graph)
            .get_rules_sorted()
            .expect("list rules")
            .iter()
            .filter(|rule| rule.source == RuleSource::CorrectionDerived)
            .count(),
        1
    );
}

#[test]
fn derived_rule_id_is_the_versioned_generic_rule() {
    assert_eq!(CORRECTION_DERIVED_RULE_ID, "rule:correction:generic-v1");
}

#[test]
fn correction_with_stable_id_retries_without_double_boost() {
    let (graph, _) = make_tracker();
    let tracker = CorrectionTracker::new(&graph);
    let rule = RulesEngine::new(&graph)
        .add_rule("Ask before modifying files", RuleSource::UserDefined)
        .expect("add rule");

    let first = tracker
        .record_correction_with_id(
            "correction:stable-retry",
            CorrectionType::ApproachCorrection,
            "Use the safe path",
            "test",
            Some(&rule.id),
        )
        .expect("first correction");
    let retry = tracker
        .record_correction_with_id(
            "correction:stable-retry",
            CorrectionType::ApproachCorrection,
            "Use the safe path",
            "test",
            Some(&rule.id),
        )
        .expect("lost-response retry");

    assert_eq!(first.id, retry.id);
    let stored_rule = graph.get_memory(&rule.id).expect("read rule");
    assert_eq!(stored_rule.importance, 60.0, "stable retry boosts once");
    assert_eq!(graph.memory_count().expect("count memories"), 2);
}
