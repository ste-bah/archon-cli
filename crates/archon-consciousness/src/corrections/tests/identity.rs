#[test]
fn stable_correction_id_with_changed_content_creates_no_derived_rule_or_rows() {
    let (graph, _) = make_tracker();
    let tracker = CorrectionTracker::new(&graph);
    let correction_id = "correction:changed-content";

    tracker
        .record_correction_with_id(
            correction_id,
            CorrectionType::ApproachCorrection,
            "Use the safe path",
            "initial context",
            None,
        )
        .expect("record initial correction");
    let before = memory_rows(&graph);

    assert!(
        tracker
            .record_correction_with_id(
                correction_id,
                CorrectionType::ApproachCorrection,
                "Use a different path",
                "initial context",
                None,
            )
            .is_err(),
        "changed stable correction content must fail"
    );

    assert_eq!(
        memory_rows(&graph),
        before,
        "a semantic collision must not create a derived rule or mutate rows"
    );
}

#[test]
fn stable_correction_id_with_changed_explicit_target_leaves_access_counts_unchanged() {
    let (graph, _) = make_tracker();
    let tracker = CorrectionTracker::new(&graph);
    let rules = RulesEngine::new(&graph);
    let first_rule = rules
        .add_rule("Ask before modifying files", RuleSource::UserDefined)
        .expect("add first rule");
    let second_rule = rules
        .add_rule("Use targeted edits", RuleSource::UserDefined)
        .expect("add second rule");
    let correction_id = "correction:changed-explicit-target";

    tracker
        .record_correction_with_id(
            correction_id,
            CorrectionType::ApproachCorrection,
            "Use the safe path",
            "initial context",
            Some(&first_rule.id),
        )
        .expect("record initial correction");
    let before = memory_rows(&graph);

    assert!(
        tracker
            .record_correction_with_id(
                correction_id,
                CorrectionType::ApproachCorrection,
                "Use the safe path",
                "initial context",
                Some(&second_rule.id),
            )
            .is_err(),
        "changed stable correction target must fail"
    );

    let after = memory_rows(&graph);
    assert_eq!(after, before, "a semantic collision must not mutate rows");
    let before_rows: Vec<Memory> = serde_json::from_value(before).expect("decode before rows");
    let after_rows: Vec<Memory> = serde_json::from_value(after).expect("decode after rows");
    for rule_id in [&first_rule.id, &second_rule.id] {
        let before_rule = before_rows
            .iter()
            .find(|memory| memory.id == *rule_id)
            .expect("target rule before collision");
        let after_rule = after_rows
            .iter()
            .find(|memory| memory.id == *rule_id)
            .expect("target rule after collision");
        assert_eq!(
            after_rule.access_count, before_rule.access_count,
            "target rule access count must not change for {rule_id}"
        );
    }
}

#[test]
fn stable_correction_id_rejects_changed_type_context_or_target_rule_without_mutation() {
    let (graph, _) = make_tracker();
    let tracker = CorrectionTracker::new(&graph);
    let rules = RulesEngine::new(&graph);
    let first_rule = rules
        .add_rule("Ask before modifying files", RuleSource::UserDefined)
        .expect("add first rule");
    let second_rule = rules
        .add_rule("Use targeted edits", RuleSource::UserDefined)
        .expect("add second rule");
    let correction_id = "correction:semantic-identity";

    tracker
        .record_correction_with_id(
            correction_id,
            CorrectionType::ApproachCorrection,
            "Use the safe path",
            "initial context",
            Some(&first_rule.id),
        )
        .expect("record correction");

    for (correction_type, context, rule_id) in [
        (
            CorrectionType::FactualError,
            "initial context",
            first_rule.id.as_str(),
        ),
        (
            CorrectionType::ApproachCorrection,
            "changed context",
            first_rule.id.as_str(),
        ),
        (
            CorrectionType::ApproachCorrection,
            "initial context",
            second_rule.id.as_str(),
        ),
    ] {
        assert!(
            tracker
                .record_correction_with_id(
                    correction_id,
                    correction_type,
                    "Use the safe path",
                    context,
                    Some(rule_id),
                )
                .is_err(),
            "a stable correction ID must reject changed semantics"
        );
    }

    let stored_first = graph.get_memory(&first_rule.id).expect("read first rule");
    let stored_second = graph.get_memory(&second_rule.id).expect("read second rule");
    assert_eq!(
        stored_first.importance, 60.0,
        "only the original boost applies"
    );
    assert_eq!(
        stored_second.importance, 50.0,
        "changed target cannot boost"
    );
    assert_eq!(graph.memory_count().expect("count memories"), 3);
}
