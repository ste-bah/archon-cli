#[test]
fn add_and_get_rule() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);

    let rule = engine
        .add_rule(
            "Do not modify files without asking",
            RuleSource::UserDefined,
        )
        .expect("add_rule should succeed");

    assert_eq!(rule.text, "Do not modify files without asking");
    assert!((rule.score - 50.0).abs() < f64::EPSILON);
    assert_eq!(rule.source, RuleSource::UserDefined);
    assert_eq!(rule.trend, Trend::Stable);

    let all = engine.get_rules_sorted().expect("get_rules_sorted");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, rule.id);
}

#[test]
fn remove_rule() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);

    let rule = engine
        .add_rule("temp rule", RuleSource::SystemDefault)
        .expect("add");
    engine.remove_rule(&rule.id).expect("remove");

    let all = engine.get_rules_sorted().expect("list");
    assert!(all.is_empty());
}

#[test]
fn remove_nonexistent_fails() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);
    let err = engine.remove_rule("no-such-id");
    assert!(err.is_err());
}
