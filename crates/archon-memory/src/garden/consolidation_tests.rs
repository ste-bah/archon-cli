use chrono::{Duration, Utc};

use super::{derived_memory_tags, semantic_consolidation_candidates};
use crate::garden::provenance::DERIVED_TAG;
use crate::types::{Memory, MemoryType};

fn memory(id: &str, content: &str, importance: f64, age_days: i64) -> Memory {
    Memory {
        id: id.into(),
        content: content.into(),
        title: "deployment target".into(),
        memory_type: MemoryType::Fact,
        importance,
        tags: Vec::new(),
        source_type: "extraction".into(),
        project_path: "scope-a".into(),
        created_at: Utc::now() - Duration::days(age_days),
        updated_at: None,
        access_count: 0,
        last_accessed: None,
    }
}

/// Three wordings of one instruction, as a real store accumulates them.
fn restatements() -> Vec<Memory> {
    vec![
        memory("a", "always run the formatter before committing", 0.5, 2),
        memory("b", "run the formatter before committing, always", 0.7, 1),
        memory("c", "before committing always run the formatter", 0.4, 0),
    ]
}

#[test]
fn a_corroborated_claim_becomes_one_candidate() {
    let candidates = semantic_consolidation_candidates(&restatements(), 2, Duration::days(7), 0.5);

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].corroboration_count(), 3);
}

#[test]
fn the_proposed_content_is_verbatim_a_source() {
    // The whole safety argument for this phase. If the proposed text is ever
    // something no source said, consolidation has invented a memory.
    let memories = restatements();

    let candidates = semantic_consolidation_candidates(&memories, 2, Duration::days(7), 0.5);

    let proposed = &candidates[0].proposed_content;
    assert!(
        memories.iter().any(|m| &m.content == proposed),
        "proposed content {proposed:?} matches no source verbatim"
    );
    assert_eq!(
        candidates[0].representative_id, "b",
        "the highest-importance source should carry the claim"
    );
}

#[test]
fn provenance_compatible_but_unrelated_memories_are_not_consolidated() {
    // The trap this phase is most likely to fall into: same writer, same
    // project, same day, completely different subjects. Provenance says they
    // MAY be one claim; vocabulary says they are not.
    let memories = vec![
        memory("a", "always run the formatter before committing", 0.5, 0),
        memory("b", "the release window is the first Tuesday", 0.5, 0),
        memory("c", "prefer the smaller of two equivalent designs", 0.5, 0),
    ];

    let candidates = semantic_consolidation_candidates(&memories, 2, Duration::days(7), 0.5);

    assert!(
        candidates.is_empty(),
        "three unrelated statements were proposed as one claim: {candidates:#?}"
    );
}

#[test]
fn one_unrelated_member_rejects_the_whole_cluster() {
    // Checking each member against the representative only would let a cluster
    // fan out around an anchor, with members sharing nothing with each other.
    let mut memories = restatements();
    memories.push(memory(
        "d",
        "the release window is the first Tuesday",
        0.9,
        0,
    ));

    let candidates = semantic_consolidation_candidates(&memories, 2, Duration::days(7), 0.5);

    for candidate in &candidates {
        assert!(
            !candidate.sources.iter().any(|s| s.memory_id == "d"),
            "an unrelated memory entered a cluster: {candidate:#?}"
        );
    }
}

#[test]
fn a_derived_memory_is_never_reconsolidated() {
    // Consolidation output rejoining a cluster counts its sources twice, so
    // corroboration would grow out of the act of recording corroboration.
    let mut memories = restatements();
    memories[0].tags.push(DERIVED_TAG.into());

    let candidates = semantic_consolidation_candidates(&memories, 3, Duration::days(7), 0.5);

    assert!(
        candidates.is_empty(),
        "the derived row was counted toward the minimum cluster size"
    );
}

#[test]
fn importance_elevation_is_bounded() {
    // A large cluster must not mint a memory that outranks everything written
    // by hand.
    let mut memories = Vec::new();
    for index in 0..20 {
        memories.push(memory(
            &format!("m{index}"),
            "always run the formatter before committing",
            0.5,
            0,
        ));
    }

    let candidates = semantic_consolidation_candidates(&memories, 2, Duration::days(7), 0.5);

    assert_eq!(candidates.len(), 1);
    assert!(
        candidates[0].proposed_importance <= 0.7 + f64::EPSILON,
        "importance ran away with cluster size: {}",
        candidates[0].proposed_importance
    );
    assert!(
        candidates[0].proposed_importance > 0.5,
        "corroboration should count for something"
    );
}

#[test]
fn the_candidate_id_is_stable_for_the_same_cluster() {
    // A nightly pass re-deriving the same cluster must re-propose the same
    // candidate, not add a new row every night.
    let memories = restatements();

    let first = semantic_consolidation_candidates(&memories, 2, Duration::days(7), 0.5);
    let second = semantic_consolidation_candidates(&memories, 2, Duration::days(7), 0.5);

    assert_eq!(first[0].candidate_id, second[0].candidate_id);
}

#[test]
fn a_cluster_that_gains_a_member_is_a_different_candidate() {
    // The evidence changed, so the proposal is a different proposal. Reusing
    // the id would let a decision made about three sources silently apply to
    // four.
    let three = restatements();
    let mut four = three.clone();
    four.push(memory(
        "d",
        "always run the formatter before committing please",
        0.3,
        0,
    ));

    let from_three = semantic_consolidation_candidates(&three, 2, Duration::days(7), 0.5);
    let from_four = semantic_consolidation_candidates(&four, 2, Duration::days(7), 0.5);

    assert_ne!(from_three[0].candidate_id, from_four[0].candidate_id);
}

#[test]
fn candidates_carry_every_source_for_review() {
    let candidates = semantic_consolidation_candidates(&restatements(), 2, Duration::days(7), 0.5);

    let sources = &candidates[0].sources;
    assert_eq!(sources.len(), 3);
    assert!(
        sources.iter().all(|source| !source.excerpt.is_empty()),
        "a reviewer needs to see what each source actually said"
    );
}

#[test]
fn derived_tags_mark_the_run_and_exclude_future_clustering() {
    let tags = derived_memory_tags("run-1");

    assert!(tags.iter().any(|tag| tag == DERIVED_TAG));
    assert!(tags.iter().any(|tag| tag.contains("run-1")));
}
