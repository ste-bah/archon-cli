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
fn reconcile_snapshot_trends_compares_persisted_scores() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);
    let rising = engine
        .add_rule("rising", RuleSource::UserDefined)
        .expect("add rising rule");
    let stable = engine
        .add_rule("stable", RuleSource::UserDefined)
        .expect("add stable rule");
    let declining = engine
        .add_rule("declining", RuleSource::UserDefined)
        .expect("add declining rule");
    let previous = engine.export_scores().expect("export baseline");

    engine
        .boost_rule_by(&rising.id, 5.0, "fixture:snapshot-rising")
        .expect("raise score");
    engine
        .apply_score_delta(&declining.id, -5.0, "fixture:snapshot-declining")
        .expect("lower score");

    engine
        .reconcile_trends(&previous)
        .expect("reconcile snapshot trends");

    let rules = engine.get_rules_sorted().expect("reload rules");
    assert_eq!(
        rules
            .iter()
            .find(|rule| rule.id == rising.id)
            .unwrap()
            .trend,
        Trend::Rising
    );
    assert_eq!(
        rules
            .iter()
            .find(|rule| rule.id == stable.id)
            .unwrap()
            .trend,
        Trend::Stable
    );
    assert_eq!(
        rules
            .iter()
            .find(|rule| rule.id == declining.id)
            .unwrap()
            .trend,
        Trend::Declining
    );
}

#[test]
fn legacy_text_reconciliation_skips_ambiguous_current_rules() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);
    let first = engine
        .add_rule("duplicate text", RuleSource::UserDefined)
        .expect("add first rule");
    let second = engine
        .add_rule("duplicate text", RuleSource::SystemDefault)
        .expect("add second rule");
    let legacy = [crate::persistence::RuleScoreEntry {
        rule_id: "missing-legacy-id".to_string(),
        rule_text: "duplicate text".to_string(),
        score: 40.0,
    }];

    engine
        .reconcile_trends(&legacy)
        .expect("reconcile legacy scores");

    for id in [first.id, second.id] {
        let rule = engine
            .get_rules_sorted()
            .expect("reload rules")
            .into_iter()
            .find(|rule| rule.id == id)
            .expect("find rule");
        assert_eq!(rule.trend, Trend::Stable);
    }
}

#[test]
fn zero_delta_import_preserves_reconciled_trend() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);
    let rule = engine
        .add_rule("preserve trend", RuleSource::UserDefined)
        .expect("add rule");
    let previous = engine.export_scores().expect("export baseline");
    engine
        .boost_rule_by(&rule.id, 5.0, "fixture:import-rising")
        .expect("raise score");
    engine.reconcile_trends(&previous).expect("reconcile trend");
    let current = engine.export_scores().expect("export current score");

    engine.import_scores(&current).expect("import same score");

    let reloaded = engine
        .get_rules_sorted()
        .expect("reload rules")
        .into_iter()
        .find(|candidate| candidate.id == rule.id)
        .expect("find rule");
    assert_eq!(reloaded.trend, Trend::Rising);
}

#[test]
fn format_for_prompt_includes_score_floor_and_excludes_score_below_it() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);
    let below = engine
        .add_rule("below prompt floor", RuleSource::CorrectionDerived)
        .expect("add below-floor rule");
    let at_floor = engine
        .add_rule("at prompt floor", RuleSource::CorrectionDerived)
        .expect("add floor rule");
    graph
        .apply_importance_delta(&below.id, -45.1, "fixture:score-4.9")
        .expect("set score to 4.9");
    graph
        .apply_importance_delta(&at_floor.id, -45.0, "fixture:score-5.0")
        .expect("set score to 5.0");

    let prompt = engine.format_for_prompt().expect("format prompt");

    assert!(!prompt.contains("below prompt floor"));
    assert!(prompt.contains("at prompt floor"));
}

#[test]
fn format_for_prompt_omits_rules_below_score_floor() {
    let (graph, _) = make_engine();
    let engine = RulesEngine::new(&graph);
    let rule = engine
        .add_rule("retired rule", RuleSource::CorrectionDerived)
        .expect("add rule");
    graph
        .apply_importance_delta(&rule.id, -46.0, "fixture:below-prompt-floor")
        .expect("lower score");

    assert_eq!(engine.format_for_prompt().expect("format prompt"), "");
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
                score: (index + 5) as f64,
            }])
            .expect("persist priority");
    }

    let output = engine.format_for_prompt().expect("format prompt");
    assert!(output.contains("rule 10"));
    assert!(!output.contains("rule 0\n"));
    assert_eq!(output.matches("[score:").count(), MAX_PROMPT_RULES);
}
