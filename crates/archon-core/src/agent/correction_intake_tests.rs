use super::*;
use archon_consciousness::correction_classifier::{
    CORRECTION_CLASSIFIER_VERSION, RATIONALE_ABSTAIN_NO_SIGNAL, RATIONALE_PROVIDER_JUDGED,
};
use archon_memory::extraction::ExtractedMemory;
use archon_memory::graph::MemoryGraph;

fn graph() -> Arc<dyn MemoryTrait> {
    Arc::new(MemoryGraph::in_memory().expect("graph")) as Arc<dyn MemoryTrait>
}

fn extracted(content: &str, memory_type: MemoryType) -> ExtractedMemory {
    ExtractedMemory {
        content: content.to_string(),
        memory_type,
        tags: Vec::new(),
    }
}

#[test]
fn classifier_recognises_each_known_form() {
    assert_eq!(
        classify_correction("no, that is not the file"),
        Some(CorrectionType::FactualError)
    );
    assert_eq!(
        classify_correction("I said use the other branch"),
        Some(CorrectionType::RepeatedInstruction)
    );
    assert_eq!(
        classify_correction("don't push without asking me"),
        Some(CorrectionType::DidForbiddenAction)
    );
    assert_eq!(
        classify_correction("you did that without permission"),
        Some(CorrectionType::ActedWithoutPermission)
    );
    assert_eq!(
        classify_correction("you should have run the tests"),
        Some(CorrectionType::ApproachCorrection)
    );
}

/// The gap this design exists to cover.
///
/// A real correction phrased outside the keyword list returns `None` here. That
/// is not a bug to fix in the classifier -- there is always another phrasing --
/// it is why the semantic pass feeds the same writer.
#[test]
fn classifier_misses_unlisted_phrasings_which_is_the_gap_the_semantic_pass_covers() {
    assert_eq!(classify_correction("that's not what I meant"), None);
    assert_eq!(classify_correction("you've misread the requirement"), None);
}

#[test]
fn extracted_corrections_are_recorded_through_the_tracker() {
    let g = graph();
    let items = vec![
        extracted("Run the tests before pushing", MemoryType::Correction),
        extracted("Rust edition is 2024", MemoryType::Fact),
    ];

    let recorded = record_extracted_corrections(&g, &items, "turn:5 (semantic pass)");

    assert_eq!(recorded, 1, "only the correction is recorded here");
    let stored = g
        .search_memories(&archon_memory::types::SearchFilter {
            memory_type: Some(MemoryType::Correction),
            ..Default::default()
        })
        .expect("search");
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].content, "Run the tests before pushing");
}

/// Non-correction items must not be written by this path.
///
/// They belong to `store_extracted`, and writing them here would recreate the
/// two-writers problem in the opposite direction.
#[test]
fn non_corrections_are_left_for_the_other_writer() {
    let g = graph();
    let items = vec![
        extracted("Rust edition is 2024", MemoryType::Fact),
        extracted("prefer explicit error types", MemoryType::Rule),
    ];

    assert_eq!(record_extracted_corrections(&g, &items, "turn:5"), 0);
    assert_eq!(g.memory_count().expect("count"), 0);
}

/// An extractor-sourced correction is bounded like any other.
///
/// A different writer reaching the same relation is exactly how the unbounded
/// content got in the first time.
#[test]
fn extracted_corrections_are_bounded() {
    let g = graph();
    let limit = archon_memory::extraction::content_limit(MemoryType::Correction);
    let huge = "x".repeat(limit * 2);

    assert_eq!(
        record_extracted_corrections(&g, &[extracted(&huge, MemoryType::Correction)], "turn:5"),
        1
    );

    let stored = g
        .search_memories(&archon_memory::types::SearchFilter {
            memory_type: Some(MemoryType::Correction),
            ..Default::default()
        })
        .expect("search");
    assert!(
        stored[0].content.chars().count() <= limit,
        "an extractor-sourced correction must respect the same cap, got {}",
        stored[0].content.chars().count()
    );
}

// ── R3 shadow labels ─────────────────────────────────────────

fn cognitive_store(root: &std::path::Path) -> archon_cognitive::PersistentCognitiveStore {
    archon_cognitive::PersistentCognitiveStore::open(root.join(".archon").join("cognitive"))
        .expect("cognitive store")
}

/// Read the labels back through a SECOND handle on the same store.
///
/// A source-of-truth read rather than an assertion about the value we just
/// passed in: the roadmap's completion standard asks for exactly that, and it
/// is also the only way to catch an event the store rejected at validation.
fn shadow_rows(
    store: &archon_cognitive::PersistentCognitiveStore,
) -> Vec<archon_cognitive::CognitiveMetricEvent> {
    archon_cognitive::metrics::MetricEventStore::new(store.db(), store.root())
        .expect("metric event store")
        .events()
        .expect("read metric events")
}

fn label(
    classification: CorrectionClassification,
    heuristic: Option<CorrectionType>,
    turn_number: u64,
) -> ShadowCorrectionLabel {
    ShadowCorrectionLabel {
        session_id: "shadow-test".into(),
        turn_number,
        task_class: "conversation".into(),
        model_id: "test-model".into(),
        classification,
        heuristic,
        correction_id: None,
        user_input_hash: user_input_hash("a user turn"),
        observed_at: chrono::Utc::now(),
    }
}

fn identity<'a>(event: &'a archon_cognitive::CognitiveMetricEvent, key: &str) -> &'a str {
    event.identity(key).unwrap_or_default()
}

/// Agreement: both the classifier and the live heuristic call it a correction.
#[test]
fn shadow_label_records_agreement_with_the_heuristic() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = cognitive_store(temp.path());
    let input = "no, that is not the file";

    record_shadow_correction_label(
        &store,
        &label(shadow_classify(input), classify_correction(input), 1),
    )
    .expect("write shadow label");

    let rows = shadow_rows(&store);
    assert_eq!(rows.len(), 1);
    assert_eq!(identity(&rows[0], "predicted_label"), "correction");
    assert_eq!(identity(&rows[0], "ground_truth_label"), "correction");
    assert_eq!(identity(&rows[0], "abstained"), "false");
    assert_eq!(identity(&rows[0], "agreement"), "true");
    assert_eq!(
        identity(&rows[0], "predicted_correction_type"),
        "factual_error"
    );
    assert_eq!(
        identity(&rows[0], "heuristic_correction_type"),
        "factual_error"
    );
    assert_eq!(
        identity(&rows[0], "classifier_version"),
        CORRECTION_CLASSIFIER_VERSION
    );
    // The whole point of shadow mode: the row itself says the classifier did
    // not mutate anything.
    assert_eq!(identity(&rows[0], "mutation_source"), "heuristic");
    assert_eq!(rows[0].label_source, SHADOW_LABEL_SOURCE);
    assert_eq!(
        rows[0].event_kind,
        archon_cognitive::MetricEventKind::CorrectionClassified
    );
}

/// Disagreement: the classifier calls it a correction, the heuristic does not.
///
/// Only reachable through the provider arm today -- both arms share one phrase
/// table -- so the judgement is supplied directly. The row must still record
/// the disagreement rather than collapsing it into the heuristic's answer.
#[test]
fn shadow_label_records_disagreement_with_the_heuristic() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = cognitive_store(temp.path());
    let provider_judged = CorrectionClassification {
        is_correction: true,
        correction_type: Some(CorrectionType::ApproachCorrection),
        confidence: 0.82,
        rationale_code: RATIONALE_PROVIDER_JUDGED.to_string(),
    };

    record_shadow_correction_label(&store, &label(provider_judged, None, 2))
        .expect("write shadow label");

    let rows = shadow_rows(&store);
    assert_eq!(rows.len(), 1);
    assert_eq!(identity(&rows[0], "predicted_label"), "correction");
    assert_eq!(identity(&rows[0], "ground_truth_label"), "not_correction");
    assert_eq!(identity(&rows[0], "abstained"), "false");
    assert_eq!(identity(&rows[0], "agreement"), "false");
    assert_eq!(rows[0].value, Some(0.82_f32 as f64));
}

/// An abstention is measured, but it is not counted as a disagreement: the
/// classifier declined to answer, so there is nothing to disagree with.
#[test]
fn abstention_is_labelled_as_abstention_not_as_a_negative_answer() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = cognitive_store(temp.path());
    let input = "that's not what I meant";

    let classification = shadow_classify(input);
    assert_eq!(classification.rationale_code, RATIONALE_ABSTAIN_NO_SIGNAL);
    record_shadow_correction_label(
        &store,
        &label(classification, classify_correction(input), 3),
    )
    .expect("write shadow label");

    let rows = shadow_rows(&store);
    assert_eq!(identity(&rows[0], "predicted_label"), "abstain");
    assert_eq!(identity(&rows[0], "abstained"), "true");
    assert_eq!(identity(&rows[0], "agreement"), "undefined");
    assert_eq!(identity(&rows[0], "predicted_correction_type"), "none");
}

/// A retried write is a replay, not a second observation.
#[test]
fn a_repeated_shadow_write_for_one_turn_is_not_counted_twice() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = cognitive_store(temp.path());
    let row = label(
        shadow_classify("no, wrong file"),
        classify_correction("no, wrong file"),
        4,
    );

    assert_eq!(
        record_shadow_correction_label(&store, &row).expect("first write"),
        archon_cognitive::MetricWriteOutcome::Written
    );
    assert_eq!(
        record_shadow_correction_label(&store, &row).expect("replayed write"),
        archon_cognitive::MetricWriteOutcome::DuplicateIgnored
    );
    assert_eq!(shadow_rows(&store).len(), 1);
}

/// The user's text never enters the measurement log; its hash does.
#[test]
fn shadow_label_carries_a_hash_rather_than_the_user_turn() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = cognitive_store(temp.path());
    let secret = "no, the token is hunter2";
    let mut row = label(shadow_classify(secret), classify_correction(secret), 5);
    row.user_input_hash = user_input_hash(secret);

    record_shadow_correction_label(&store, &row).expect("write shadow label");

    let rows = shadow_rows(&store);
    let serialized = serde_json::to_string(&rows[0]).expect("serialize row");
    assert!(
        !serialized.contains("hunter2"),
        "raw user text leaked into the metric row"
    );
    assert_eq!(
        identity(&rows[0], "user_input_hash"),
        user_input_hash(secret)
    );
}

/// The provider arm must be off on the live path.
///
/// `abstain.no_signal` is only reachable with the arm disabled; an enabled arm
/// produces `provider.*` or `abstain.provider_unavailable` instead.
#[test]
fn live_path_classifier_runs_with_the_provider_arm_off() {
    assert_eq!(
        shadow_classify("that isn't quite it").rationale_code,
        RATIONALE_ABSTAIN_NO_SIGNAL
    );
}

/// The deterministic arm and the live mutating heuristic must never disagree,
/// because they are now one table. This is what keeps explicit-case recall at
/// 1.0 measurable rather than aspirational.
#[test]
fn shadow_classifier_and_live_heuristic_share_one_phrase_table() {
    for input in [
        "no, that is not the file",
        "I said use the other branch",
        "don't push without asking me",
        "you did that without permission",
        "you should have run the tests",
        "what does this function do?",
    ] {
        let classification = shadow_classify(input);
        assert_eq!(
            classification.correction_type,
            classify_correction(input),
            "taxonomy diverged for {input:?}"
        );
        assert_eq!(
            classification.is_correction,
            classify_correction(input).is_some(),
            "verdict diverged for {input:?}"
        );
    }
}

// ── live wiring ──────────────────────────────────────────────

async fn wait_for_shadow_rows(
    store: &archon_cognitive::PersistentCognitiveStore,
    expected: usize,
) -> Vec<archon_cognitive::CognitiveMetricEvent> {
    // The write is dispatched to the blocking pool so it cannot add latency to
    // the turn, which means the test has to wait for it rather than assume it.
    for _ in 0..200 {
        let rows = shadow_rows(store);
        if rows.len() >= expected {
            return rows;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("shadow label never reached the metric store");
}

/// Abstention creates nothing.
///
/// The turn the heuristic declines must leave the memory graph exactly as it
/// found it -- no correction, no derived rule -- while still being measured.
#[tokio::test]
async fn abstained_turn_records_a_label_and_mutates_nothing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = super::super::tests::test_agent();
    agent.config.session_id = "abstain-session".into();
    agent.set_cognitive_store(cognitive_store(temp.path()));
    let g = graph();

    agent
        .detect_and_record_correction("that's not what I meant", &g)
        .await;

    assert_eq!(
        g.memory_count().expect("count"),
        0,
        "an abstained turn must not create a correction or a derived rule"
    );

    let store = cognitive_store(temp.path());
    let rows = wait_for_shadow_rows(&store, 1).await;
    assert_eq!(identity(&rows[0], "abstained"), "true");
    assert_eq!(identity(&rows[0], "ground_truth_label"), "not_correction");
    assert_eq!(rows[0].session_id, "abstain-session");
}

/// The live mutating path still belongs to the heuristic, and the label written
/// beside it points at the correction the heuristic actually stored.
#[tokio::test]
async fn heuristic_correction_turn_records_a_label_linked_to_the_stored_correction() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = super::super::tests::test_agent();
    agent.config.session_id = "correction-session".into();
    agent.set_cognitive_store(cognitive_store(temp.path()));
    let g = graph();

    agent
        .detect_and_record_correction("no, that is not the file", &g)
        .await;

    let stored = g
        .search_memories(&archon_memory::types::SearchFilter {
            memory_type: Some(MemoryType::Correction),
            ..Default::default()
        })
        .expect("search");
    assert_eq!(stored.len(), 1, "the heuristic still owns the mutation");

    let store = cognitive_store(temp.path());
    let rows = wait_for_shadow_rows(&store, 1).await;
    assert_eq!(identity(&rows[0], "correction_id"), stored[0].id);
    assert_eq!(identity(&rows[0], "agreement"), "true");
    assert_eq!(identity(&rows[0], "mutation_source"), "heuristic");
}
