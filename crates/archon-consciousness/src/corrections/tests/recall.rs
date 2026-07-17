#[test]
fn derived_rule_collision_with_wrong_content_rejects_before_correction_mutation() {
    let (graph, _) = make_tracker();
    let content = "Use Edit before modifying config files";
    let rule_id = CORRECTION_DERIVED_RULE_ID;
    graph
        .store_memory_with_id(
            &rule_id,
            "wrong generic rule text",
            "",
            MemoryType::Rule,
            50.0,
            &[
                "source:correction_derived".to_string(),
                "trend:stable".to_string(),
            ],
            "rules_engine",
            "",
        )
        .expect("seed wrong-content rule");
    let tracker = CorrectionTracker::new(&graph);

    let result = tracker.record_correction_with_id(
        "correction:derived-content-collision",
        CorrectionType::ApproachCorrection,
        content,
        "test",
        None,
    );

    assert!(result.is_err(), "wrong-content generic rule must reject");
    assert_eq!(graph.memory_count().expect("count memories"), 1);
}

#[test]
fn derived_rule_collision_with_user_source_rejects_before_correction_mutation() {
    let (graph, _) = make_tracker();
    let content = "Use Edit before modifying config files";
    let rule_id = CORRECTION_DERIVED_RULE_ID;
    let rule_text = CORRECTION_DERIVED_RULE_TEXT;
    graph
        .store_memory_with_id(
            &rule_id,
            &rule_text,
            "",
            MemoryType::Rule,
            50.0,
            &[
                "source:user_defined".to_string(),
                "trend:stable".to_string(),
            ],
            "rules_engine",
            "",
        )
        .expect("seed colliding user rule");
    let tracker = CorrectionTracker::new(&graph);

    let result = tracker.record_correction_with_id(
        "correction:derived-source-collision",
        CorrectionType::ApproachCorrection,
        content,
        "test",
        None,
    );

    assert!(
        result.is_err(),
        "wrong-source deterministic rule must reject"
    );
    let stored_rule = graph.get_memory(&rule_id).expect("read colliding rule");
    assert_eq!(
        stored_rule.importance, 50.0,
        "collision must not boost rule"
    );
    assert_eq!(
        graph.memory_count().expect("count memories"),
        1,
        "collision must not store a correction"
    );
    assert!(
        graph
            .get_related_memories(&rule_id, 1)
            .expect("read relationships")
            .is_empty(),
        "collision must not create a relationship"
    );
}

#[test]
fn equivalent_corrections_reuse_one_derived_rule() {
    let (graph, _) = make_tracker();
    let tracker = CorrectionTracker::new(&graph);

    let first = tracker
        .record_correction(
            CorrectionType::ApproachCorrection,
            "Use Edit before modifying config files",
            "first session",
            None,
        )
        .expect("record first correction");
    let second = tracker
        .record_correction(
            CorrectionType::ApproachCorrection,
            "  use   edit BEFORE modifying config files  ",
            "second session",
            None,
        )
        .expect("record second correction");

    let rule_id = first
        .rule_id
        .as_ref()
        .expect("first derived rule")
        .to_string();
    assert_eq!(second.rule_id.as_deref(), Some(rule_id.as_str()));
    let rule = graph.get_memory(&rule_id).expect("get reused rule");
    assert!((rule.importance - 70.0).abs() < f64::EPSILON);

    let rules = RulesEngine::new(&graph)
        .get_rules_sorted()
        .expect("list rules");
    assert_eq!(
        rules
            .iter()
            .filter(|candidate| candidate.source == RuleSource::CorrectionDerived)
            .count(),
        1
    );
    for correction in [&first, &second] {
        let stored = graph
            .get_memory(&correction.id)
            .expect("reload correction row");
        assert_eq!(stored.memory_type, MemoryType::Correction);
        let related = graph
            .get_related_memories(&correction.id, 1)
            .expect("read correction relationship");
        assert!(related.iter().any(|memory| memory.id == rule_id));
    }
}

#[test]
fn similar_corrections_reuse_one_derived_rule() {
    let (graph, _) = make_tracker();
    let tracker = CorrectionTracker::new(&graph);

    let first = tracker
        .record_correction(
            CorrectionType::ApproachCorrection,
            "Use Edit before modifying config files",
            "first session",
            None,
        )
        .expect("record first correction");
    let second = tracker
        .record_correction(
            CorrectionType::ApproachCorrection,
            "Use Edit before modifying configuration files",
            "second session",
            None,
        )
        .expect("record similar correction");

    assert_eq!(first.rule_id, second.rule_id);
    let rules = RulesEngine::new(&graph)
        .get_rules_sorted()
        .expect("list rules");
    assert_eq!(
        rules
            .iter()
            .filter(|rule| rule.source == RuleSource::CorrectionDerived)
            .count(),
        1
    );
}

#[test]
fn automatic_similarity_reuse_ignores_explicit_user_rule_target() {
    let (graph, _) = make_tracker();
    let user_rule = RulesEngine::new(&graph)
        .add_rule("User-authored edit guidance", RuleSource::UserDefined)
        .expect("add user rule");
    let tracker = CorrectionTracker::new(&graph);

    tracker
        .record_correction(
            CorrectionType::ApproachCorrection,
            "Use Edit before modifying config files",
            "explicit session",
            Some(&user_rule.id),
        )
        .expect("record explicit correction");
    let automatic = tracker
        .record_correction(
            CorrectionType::ApproachCorrection,
            "Use Edit before modifying configuration files",
            "automatic session",
            None,
        )
        .expect("record automatic correction");

    let automatic_rule_id = automatic.rule_id.as_deref().expect("derived rule");
    assert_ne!(automatic_rule_id, user_rule.id);
    let automatic_rule = graph
        .get_memory(automatic_rule_id)
        .expect("read automatic rule");
    assert_eq!(automatic_rule.memory_type, MemoryType::Rule);
    assert_eq!(automatic_rule.content, CORRECTION_DERIVED_RULE_TEXT);
    assert!(automatic_rule
        .tags
        .iter()
        .any(|tag| tag == "source:correction_derived"));
    assert_eq!(
        automatic_rule
            .tags
            .iter()
            .filter(|tag| tag.starts_with("trend:"))
            .count(),
        1
    );
    assert_eq!(automatic_rule.source_type, "rules_engine");
    assert_eq!(
        graph.get_memory(&user_rule.id).expect("read user rule").importance,
        60.0,
        "only the explicit correction boosts the user rule"
    );
}

#[test]
fn correction_derived_rule_does_not_embed_raw_user_text() {
    let (graph, _) = make_tracker();
    let tracker = CorrectionTracker::new(&graph);
    let raw = "No, use Edit on /private/customer-alpha/secret-config.toml";

    let correction = tracker
        .record_correction(
            CorrectionType::ApproachCorrection,
            raw,
            "coding session",
            None,
        )
        .expect("record correction");

    let rule = graph
        .get_memory(correction.rule_id.as_deref().expect("derived rule"))
        .expect("load derived rule");
    assert_eq!(rule.content, "Review the approach that triggered this correction before repeating it.");
    assert!(!rule.content.contains("customer-alpha"));
    assert_eq!(graph.get_memory(&correction.id).unwrap().content, raw);
}

#[test]
fn recall_corrections_finds_stored() {
    let (graph, _) = make_tracker();
    let tracker = CorrectionTracker::new(&graph);

    tracker
        .record_correction(
            CorrectionType::RepeatedInstruction,
            "User already said not to create README files",
            "doc session",
            None,
        )
        .expect("record");

    tracker
        .record_correction(
            CorrectionType::ApproachCorrection,
            "Should have used edit instead of write",
            "coding session",
            None,
        )
        .expect("record");

    let results = tracker.recall_corrections("README", 10).expect("recall");

    assert!(
        !results.is_empty(),
        "should find at least one correction matching 'README'",
    );
    assert!(results[0].content.contains("README"));
}

#[test]
fn malformed_stored_severity_uses_correction_type_default() {
    let timestamp = Utc::now();
    let memory = Memory {
        id: "malformed-severity".to_string(),
        content: "correction".to_string(),
        title: String::new(),
        memory_type: MemoryType::Correction,
        importance: 0.0,
        tags: vec![
            CorrectionType::DidForbiddenAction.as_tag(),
            "severity:NaN".to_string(),
        ],
        source_type: "test".to_string(),
        project_path: String::new(),
        created_at: timestamp,
        updated_at: None,
        access_count: 0,
        last_accessed: None,
    };

    let correction = memory_to_correction(memory).expect("parse correction");

    assert_eq!(
        correction.severity,
        CorrectionType::DidForbiddenAction.severity_multiplier()
    );
}

#[test]
fn correction_sorting_breaks_timestamp_ties_by_stable_id() {
    let timestamp = Utc::now();
    let mut corrections = vec![
        Correction {
            id: "b".into(),
            correction_type: CorrectionType::FactualError,
            content: "same".into(),
            context: "test".into(),
            severity: 1.5,
            rule_id: None,
            timestamp,
        },
        Correction {
            id: "a".into(),
            correction_type: CorrectionType::FactualError,
            content: "same".into(),
            context: "test".into(),
            severity: 1.5,
            rule_id: None,
            timestamp,
        },
    ];

    sort_corrections(&mut corrections);

    assert_eq!(
        corrections
            .iter()
            .map(|correction| correction.id.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
}
#[test]
fn correction_sorting_orders_non_finite_severity_deterministically() {
    let timestamp = Utc::now();
    let mut corrections = vec![
        Correction {
            id: "finite".into(),
            correction_type: CorrectionType::FactualError,
            content: "same".into(),
            context: "test".into(),
            severity: 1.5,
            rule_id: None,
            timestamp,
        },
        Correction {
            id: "non-finite".into(),
            correction_type: CorrectionType::FactualError,
            content: "same".into(),
            context: "test".into(),
            severity: f64::NAN,
            rule_id: None,
            timestamp,
        },
    ];

    sort_corrections(&mut corrections);

    assert_eq!(
        corrections
            .iter()
            .map(|correction| correction.id.as_str())
            .collect::<Vec<_>>(),
        ["non-finite", "finite"]
    );
}

#[test]
fn recall_corrections_orders_by_severity_before_truncating() {
    let (graph, _) = make_tracker();
    let tracker = CorrectionTracker::new(&graph);

    let factual = tracker
        .record_correction(
            CorrectionType::FactualError,
            "shared correction detail",
            "first",
            None,
        )
        .expect("record factual correction");
    let critical = tracker
        .record_correction(
            CorrectionType::ActedWithoutPermission,
            "shared correction detail",
            "second",
            None,
        )
        .expect("record critical correction");

    let results = tracker
        .recall_corrections("shared correction detail", 10)
        .expect("recall corrections");
    let ids: Vec<_> = results
        .iter()
        .map(|correction| correction.id.as_str())
        .collect();
    assert_eq!(ids, [critical.id.as_str(), factual.id.as_str()]);
    assert_eq!(
        tracker
            .recall_corrections("shared correction detail", 1)
            .expect("recall one correction")[0]
            .id,
        critical.id
    );
}

#[test]
fn recall_with_limit_truncates() {
    let (graph, _) = make_tracker();
    let tracker = CorrectionTracker::new(&graph);

    for i in 0..5 {
        tracker
            .record_correction(
                CorrectionType::FactualError,
                &format!("error number {i}"),
                "bulk",
                None,
            )
            .expect("record");
    }

    let results = tracker.recall_corrections("error", 2).expect("recall");
    assert!(results.len() <= 2);
}

#[test]
fn correction_type_tag_roundtrip() {
    let types = [
        CorrectionType::FactualError,
        CorrectionType::ApproachCorrection,
        CorrectionType::RepeatedInstruction,
        CorrectionType::DidForbiddenAction,
        CorrectionType::ActedWithoutPermission,
    ];
    for ct in &types {
        let tag = ct.as_tag();
        let parsed = CorrectionType::from_tag(&tag);
        assert_eq!(parsed, Some(*ct), "roundtrip failed for {tag}");
    }
}
