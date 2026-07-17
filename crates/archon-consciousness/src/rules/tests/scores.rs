#[test]
fn import_scores_skips_ambiguous_text_fallback_when_rule_id_is_missing() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);
    let first = engine
        .add_rule("Review before changing files", RuleSource::UserDefined)
        .expect("add first rule");
    let second = engine
        .add_rule("Review before changing files", RuleSource::CorrectionDerived)
        .expect("add second rule");

    let imported = engine
        .import_scores(&[crate::persistence::RuleScoreEntry {
            rule_id: "missing:legacy-rule".to_string(),
            rule_text: "Review before changing files".to_string(),
            score: 80.0,
        }])
        .expect("ambiguous legacy score should skip");

    assert_eq!(imported, 0);
    assert_eq!(
        graph.get_memory(&first.id).expect("read first rule").importance,
        50.0
    );
    assert_eq!(
        graph.get_memory(&second.id).expect("read second rule").importance,
        50.0
    );
}

#[test]
fn score_mutations_persist_exactly_one_trend_tag() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);
    let rule = engine
        .add_rule("persisted trend", RuleSource::UserDefined)
        .expect("add rule");

    engine.reinforce_rule(&rule.id).expect("reinforce");
    engine.decay_scores(5.0).expect("decay");

    let stored = graph.get_memory(&rule.id).expect("reload rule memory");
    let trend_tags: Vec<_> = stored
        .tags
        .iter()
        .filter(|tag| tag.starts_with("trend:"))
        .collect();
    assert_eq!(trend_tags, ["trend:declining"]);
}

#[test]
fn format_for_prompt_output() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);

    let r1 = engine
        .add_rule("Ask before modifying", RuleSource::UserDefined)
        .expect("add");
    let r2 = engine
        .add_rule("Explain reasoning", RuleSource::SystemDefault)
        .expect("add");

    graph
        .apply_importance_delta(&r1.id, 35.0, "fixture:prompt-high")
        .expect("set");
    graph
        .apply_importance_delta(&r2.id, -5.0, "fixture:prompt-low")
        .expect("set");

    let output = engine.format_for_prompt().expect("format");
    assert!(output.starts_with("<behavioral_rules>"));
    assert!(output.ends_with("</behavioral_rules>"));
    assert!(output.contains("[score: 85.0 up]"));
    assert!(output.contains("[score: 45.0 down]"));
    // Higher score should come first.
    let pos_85 = output.find("85.0").expect("contains 85");
    let pos_45 = output.find("45.0").expect("contains 45");
    assert!(pos_85 < pos_45);
}

#[test]
fn format_empty_returns_empty_string() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);
    let output = engine.format_for_prompt().expect("format");
    assert!(output.is_empty());
}

#[test]
fn import_scores_rejects_out_of_range_scores_without_mutating_rule() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);
    let rule = engine
        .add_rule("ask before editing", RuleSource::UserDefined)
        .expect("add rule");

    let result = engine.import_scores(&[crate::persistence::RuleScoreEntry {
        rule_id: rule.id.clone(),
        rule_text: rule.text.clone(),
        score: 101.0,
    }]);

    assert!(result.is_err(), "out-of-range imported score must fail");
    assert_eq!(
        graph.get_memory(&rule.id).expect("get rule").importance,
        50.0
    );
}

#[test]
fn get_rules_sorted_skips_stored_non_finite_scores() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);
    let valid = engine
        .add_rule("valid rule", RuleSource::UserDefined)
        .expect("add valid rule");
    let invalid_id = graph
        .store_memory(
            "invalid rule",
            "",
            MemoryType::Rule,
            f64::NAN,
            &[RuleSource::UserDefined.as_tag(), Trend::Stable.as_tag()],
            "test",
            "",
        )
        .expect("store invalid rule");

    let rules = engine.get_rules_sorted().expect("list rules");

    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].id, valid.id);
    assert_ne!(rules[0].id, invalid_id);
}
