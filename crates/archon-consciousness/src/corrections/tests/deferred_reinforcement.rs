/// The split, stated as its defining property: recording a correction moves no
/// score.
///
/// Under the joined path this same call raised the rule to 75.0. If that
/// happens again, an unattributable correction is voting on rule scores, which
/// is the failure the R2 slice's "reinforce only after attribution" exists to
/// stop.
#[test]
fn recording_without_reinforcement_leaves_the_rule_score_alone() {
    let (graph, _) = make_tracker();
    let tracker = CorrectionTracker::new(&graph);
    let rules = RulesEngine::new(&graph);
    let rule = rules
        .add_rule("Always ask before modifying files", RuleSource::UserDefined)
        .expect("add_rule");

    let correction = tracker
        .record_correction_unreinforced(
            CorrectionType::ActedWithoutPermission,
            "Modified a file without asking",
            "editing session",
            Some(&rule.id),
        )
        .expect("record without reinforcement");

    let stored = graph.get_memory(&rule.id).expect("get rule");
    assert!(
        (stored.importance - 50.0).abs() < f64::EPSILON,
        "recording must not move the score, got {}",
        stored.importance
    );
    // The record and the link are still written -- what is withheld is only the
    // score change.
    assert_eq!(correction.rule_id.as_deref(), Some(rule.id.as_str()));
    let related = graph
        .get_related_memories(&correction.id, 1)
        .expect("read correction relationship");
    assert!(related.iter().any(|memory| memory.id == rule.id));
}

/// Reinforcing afterwards produces exactly the joined path's result.
#[test]
fn deferred_reinforcement_reaches_the_same_score_as_the_joined_path() {
    let (graph, _) = make_tracker();
    let tracker = CorrectionTracker::new(&graph);
    let rules = RulesEngine::new(&graph);
    let rule = rules
        .add_rule("Always ask before modifying files", RuleSource::UserDefined)
        .expect("add_rule");

    let correction = tracker
        .record_correction_unreinforced(
            CorrectionType::ActedWithoutPermission,
            "Modified a file without asking",
            "editing session",
            Some(&rule.id),
        )
        .expect("record without reinforcement");
    assert!(
        tracker
            .reinforce_from_correction(&correction)
            .expect("reinforce"),
        "a correction linked to a rule reports that it reinforced one"
    );

    let stored = graph.get_memory(&rule.id).expect("get rule");
    assert!(
        (stored.importance - 75.0).abs() < f64::EPSILON,
        "expected 50 + 5.0*5.0, got {}",
        stored.importance
    );
}

/// The deferral is only safe if it stays exactly-once. The idempotency key is
/// the correction id, and it does not expire when the recording call returns.
#[test]
fn reinforcing_one_correction_twice_raises_the_score_once() {
    let (graph, _) = make_tracker();
    let tracker = CorrectionTracker::new(&graph);

    let correction = tracker
        .record_correction_unreinforced(
            CorrectionType::FactualError,
            "That is not the right file",
            "turn:3",
            None,
        )
        .expect("record without reinforcement");

    tracker
        .reinforce_from_correction(&correction)
        .expect("first reinforcement");
    tracker
        .reinforce_from_correction(&correction)
        .expect("replayed reinforcement");

    let rule_id = correction.rule_id.as_deref().expect("derived rule");
    let stored = graph.get_memory(rule_id).expect("get rule");
    assert!(
        (stored.importance - 57.5).abs() < f64::EPSILON,
        "expected 50 + 1.5*5.0 applied once, got {}",
        stored.importance
    );
}

/// The derived rule is still created by the recording half, at its base score.
///
/// Withholding reinforcement is not the same as withholding the rule: the
/// taxonomy rule has to exist for the correction's edge to point at something.
#[test]
fn the_derived_rule_exists_at_its_base_score_before_any_reinforcement() {
    let (graph, _) = make_tracker();
    let tracker = CorrectionTracker::new(&graph);

    let correction = tracker
        .record_correction_unreinforced(
            CorrectionType::DidForbiddenAction,
            "Do not push without asking",
            "turn:4",
            None,
        )
        .expect("record without reinforcement");

    let rule_id = correction.rule_id.as_deref().expect("derived rule");
    assert_eq!(rule_id, "rule:correction:did-forbidden-action:v2");
    let stored = graph.get_memory(rule_id).expect("get rule");
    assert!(
        (stored.importance - 50.0).abs() < f64::EPSILON,
        "expected the untouched base score, got {}",
        stored.importance
    );
}

/// Two unreinforced corrections against one rule leave it exactly where it was.
///
/// The rich-get-richer failure finding 40 closed was about which rule got the
/// boost. This is the adjacent one: how many boosts a detector may hand out
/// without justifying any of them.
#[test]
fn many_unattributed_corrections_still_move_no_score() {
    let (graph, _) = make_tracker();
    let tracker = CorrectionTracker::new(&graph);

    for index in 0..5 {
        tracker
            .record_correction_unreinforced_with_id(
                &format!("corr-{index}"),
                CorrectionType::ApproachCorrection,
                "Use the other approach",
                &format!("turn:{index}"),
                None,
            )
            .expect("record without reinforcement");
    }

    let stored = graph
        .get_memory("rule:correction:approach-correction:v2")
        .expect("get rule");
    assert!(
        (stored.importance - 50.0).abs() < f64::EPSILON,
        "five unjustified corrections moved the score to {}",
        stored.importance
    );
}
