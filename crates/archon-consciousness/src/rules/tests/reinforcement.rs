#[test]
fn reinforce_increases_score() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);

    let rule = engine
        .add_rule("be polite", RuleSource::CorrectionDerived)
        .expect("add");
    let reinforced = engine.reinforce_rule(&rule.id).expect("reinforce");

    assert!((reinforced.score - 55.0).abs() < f64::EPSILON);
    assert!(reinforced.last_triggered.is_some());
}

#[test]
fn reinforce_clamps_at_100() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);

    let rule = engine
        .add_rule("max rule", RuleSource::UserDefined)
        .expect("add");

    // Set score close to max.
    graph
        .apply_importance_delta(&rule.id, 48.0, "fixture:reinforce-max")
        .expect("set score");

    let reinforced = engine.reinforce_rule(&rule.id).expect("reinforce");
    assert!((reinforced.score - 100.0).abs() < f64::EPSILON);
}

#[test]
fn decay_reduces_scores() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);

    engine
        .add_rule("rule a", RuleSource::SystemDefault)
        .expect("add");
    engine
        .add_rule("rule b", RuleSource::SystemDefault)
        .expect("add");

    engine.decay_scores(10.0).expect("decay");

    let rules = engine.get_rules_sorted().expect("list");
    for r in &rules {
        assert!((r.score - 40.0).abs() < f64::EPSILON);
    }
}

#[test]
fn decay_clamps_at_zero() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);

    let rule = engine
        .add_rule("low", RuleSource::SystemDefault)
        .expect("add");
    graph
        .apply_importance_delta(&rule.id, -47.0, "fixture:decay-low")
        .expect("set");

    engine.decay_scores(10.0).expect("decay");

    let rules = engine.get_rules_sorted().expect("list");
    assert!((rules[0].score).abs() < f64::EPSILON);
}

#[test]
fn sorting_by_score_descending() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);

    let r1 = engine
        .add_rule("low priority", RuleSource::SystemDefault)
        .expect("add");
    let r2 = engine
        .add_rule("high priority", RuleSource::UserDefined)
        .expect("add");

    graph
        .apply_importance_delta(&r1.id, -30.0, "fixture:sort-low")
        .expect("set");
    graph
        .apply_importance_delta(&r2.id, 30.0, "fixture:sort-high")
        .expect("set");

    let rules = engine.get_rules_sorted().expect("list");
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].id, r2.id);
    assert_eq!(rules[1].id, r1.id);
}

#[test]
fn format_for_prompt_uses_stored_trend() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);
    let r1 = engine
        .add_rule("Ask before modifying", RuleSource::UserDefined)
        .expect("add");
    let _r2 = engine
        .add_rule("Explain reasoning", RuleSource::SystemDefault)
        .expect("add");

    engine.reinforce_rule(&r1.id).expect("reinforce");
    engine.decay_scores(5.0).expect("decay");

    let output = engine.format_for_prompt().expect("format");
    assert!(output.contains("[score: 50.0 down]"));
    assert!(output.contains("[score: 45.0 down]"));
}

#[test]
fn reinforce_persists_rising_trend_for_reload_and_prompt() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);
    let rule = engine
        .add_rule("ask before editing", RuleSource::UserDefined)
        .expect("add rule");

    engine.reinforce_rule(&rule.id).expect("reinforce");

    let stored = graph.get_memory(&rule.id).expect("reload rule memory");
    assert!(stored.tags.iter().any(|tag| tag == "trend:rising"));
    assert!(!stored.tags.iter().any(|tag| tag == "trend:stable"));
    let reloaded = engine
        .get_rules_sorted()
        .expect("reload rules")
        .into_iter()
        .find(|candidate| candidate.id == rule.id)
        .expect("find rule");
    assert_eq!(reloaded.trend, Trend::Rising);
    assert!(
        engine
            .format_for_prompt()
            .expect("format prompt")
            .contains("[score: 55.0 up]")
    );
}

#[test]
fn decay_persists_declining_trend_for_reload() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);
    let rule = engine
        .add_rule("avoid destructive commands", RuleSource::UserDefined)
        .expect("add rule");

    engine.decay_scores(5.0).expect("decay");

    let stored = graph.get_memory(&rule.id).expect("reload rule memory");
    assert!(stored.tags.iter().any(|tag| tag == "trend:declining"));
    let reloaded = engine
        .get_rules_sorted()
        .expect("reload rules")
        .into_iter()
        .find(|candidate| candidate.id == rule.id)
        .expect("find rule");
    assert_eq!(reloaded.trend, Trend::Declining);
}

#[test]
fn format_for_prompt_keeps_only_the_top_ten_rules() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);
    for index in 0..11 {
        let rule = engine
            .add_rule(&format!("rule {index}"), RuleSource::UserDefined)
            .expect("add rule");
        engine
            .import_scores(&[crate::persistence::RuleScoreEntry {
                rule_id: rule.id,
                rule_text: format!("rule {index}"),
                score: index as f64,
            }])
            .expect("persist priority");
    }

    let output = engine.format_for_prompt().expect("format prompt");
    assert!(output.contains("rule 10"));
    assert!(!output.contains("rule 0\n"));
    assert_eq!(output.matches("[score:").count(), MAX_PROMPT_RULES);
}
