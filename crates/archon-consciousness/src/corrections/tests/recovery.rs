#[test]
fn provenance_status_read_error_preserves_correction_and_relationship() {
    let (graph, _) = make_tracker();
    let rule = RulesEngine::new(&graph)
        .add_rule("Ask before modifying files", RuleSource::UserDefined)
        .expect("add rule");
    let failing_graph = FailingMemory {
        inner: &graph,
        failure_point: FailurePoint::StatusReadError,
    };
    let tracker = CorrectionTracker::new(&failing_graph);

    let error = tracker
        .record_correction_with_id(
            "correction:status-read-error",
            CorrectionType::ApproachCorrection,
            "Use the safe path",
            "test",
            Some(&rule.id),
        )
        .expect_err("uncertain boost outcome must surface");

    assert!(matches!(error, CorrectionError::BoostOutcomeUnknown(_)));
    assert!(
        graph.get_memory("correction:status-read-error").is_ok(),
        "uncertainty must retain correction evidence"
    );
    assert!(
        graph
            .get_related_memories("correction:status-read-error", 1)
            .expect("read correction relationship")
            .iter()
            .any(|memory| memory.id == rule.id),
        "uncertainty must retain relationship evidence"
    );
}

#[test]
fn two_lost_score_responses_preserve_committed_correction_and_relationship() {
    let (graph, _) = make_tracker();
    let rule = RulesEngine::new(&graph)
        .add_rule("Ask before modifying files", RuleSource::UserDefined)
        .expect("add rule");
    let failing_graph = FailingMemory {
        inner: &graph,
        failure_point: FailurePoint::ScoreUpdateAfterCommit,
    };
    let tracker = CorrectionTracker::new(&failing_graph);

    let correction = tracker
        .record_correction_with_id(
            "correction:two-lost-score-responses",
            CorrectionType::ApproachCorrection,
            "Use the safe path",
            "test",
            Some(&rule.id),
        )
        .expect("committed delta status recovers both lost responses");

    assert_eq!(
        graph.get_memory(&rule.id).expect("read rule").importance,
        60.0,
        "the idempotent provenance applies one boost"
    );
    assert!(
        graph.get_memory(&correction.id).is_ok(),
        "correction remains"
    );
    assert!(
        graph
            .get_related_memories(&correction.id, 1)
            .expect("read correction relationship")
            .iter()
            .any(|memory| memory.id == rule.id),
        "correction relationship remains"
    );
}

#[test]
fn lost_score_response_retries_without_deleting_committed_correction_or_double_boosting() {
    let (graph, _) = make_tracker();
    let rule = RulesEngine::new(&graph)
        .add_rule("Ask before modifying files", RuleSource::UserDefined)
        .expect("add rule");
    let failing_graph = FailingMemory {
        inner: &graph,
        failure_point: FailurePoint::ScoreUpdateAfterCommitOnce(
            std::sync::atomic::AtomicBool::new(false),
        ),
    };
    let tracker = CorrectionTracker::new(&failing_graph);

    let correction = tracker
        .record_correction_with_id(
            "correction:lost-score-response",
            CorrectionType::ApproachCorrection,
            "Use the safe path",
            "test",
            Some(&rule.id),
        )
        .expect("retry after a lost delta response succeeds");

    let stored_rule = graph.get_memory(&rule.id).expect("read rule");
    assert_eq!(stored_rule.importance, 60.0, "idempotent retry boosts once");
    assert!(
        graph.get_memory(&correction.id).is_ok(),
        "correction remains stored"
    );
    assert!(
        graph
            .get_related_memories(&correction.id, 1)
            .expect("read correction relationship")
            .iter()
            .any(|memory| memory.id == rule.id),
        "correction relationship remains after recovered response"
    );
}
