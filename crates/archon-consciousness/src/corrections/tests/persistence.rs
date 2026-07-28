#[test]
fn boost_clamps_at_100() {
    let (graph, _) = make_tracker();
    let tracker = CorrectionTracker::new(&graph);

    let rules = RulesEngine::new(&graph);
    let rule = rules
        .add_rule("fragile rule", RuleSource::SystemDefault)
        .expect("add");

    // Set score close to max.
    graph
        .apply_importance_delta(&rule.id, 48.0, "fixture:boost-clamp")
        .expect("set score");

    // DidForbiddenAction => 4.0 * 5.0 = 20.0 boost, should clamp.
    tracker
        .record_correction(
            CorrectionType::DidForbiddenAction,
            "created a file without permission",
            "test",
            Some(&rule.id),
        )
        .expect("record");

    let updated = graph.get_memory(&rule.id).expect("get");
    assert!(
        (updated.importance - 100.0).abs() < f64::EPSILON,
        "should clamp to 100.0, got {}",
        updated.importance,
    );
}

#[test]
fn correction_persists_in_graph() {
    let (graph, _) = make_tracker();
    let tracker = CorrectionTracker::new(&graph);

    let correction = tracker
        .record_correction(
            CorrectionType::ApproachCorrection,
            "Used unwrap in library code",
            "code review",
            None,
        )
        .expect("record");

    // Verify the memory is retrievable directly.
    let mem = graph.get_memory(&correction.id).expect("get");
    assert_eq!(mem.memory_type, MemoryType::Correction);
    assert!(mem.content.contains("unwrap"));
}
