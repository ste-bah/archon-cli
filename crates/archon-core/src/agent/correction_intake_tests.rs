use super::*;
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
