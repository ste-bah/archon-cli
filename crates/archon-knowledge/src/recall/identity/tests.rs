use super::*;
use crate::recall::RecallHit;

fn hit(source: RecallSource, id: &str, content: &str, rank: usize, refs: &[&str]) -> RecallHit {
    RecallHit::at_rank(source, id, content, rank)
        .with_provenance(refs.iter().map(|r| (*r).to_string()))
}

#[test]
fn identity_ignores_case_spacing_and_trailing_punctuation() {
    assert_eq!(
        content_identity("The plugin is safe."),
        content_identity("the  plugin\n is safe")
    );
}

/// Internal negation must never normalise away — the whole conflict machinery
/// depends on these two staying distinct.
#[test]
fn identity_separates_a_claim_from_its_negation() {
    assert_ne!(
        content_identity("The plugin is safe."),
        content_identity("The plugin is not safe.")
    );
}

#[test]
fn same_content_from_two_stores_becomes_one_hit() {
    let merged = merge(vec![
        hit(
            RecallSource::Docs,
            "chunk-1",
            "Archon uses CozoDB.",
            0,
            &["chunk:chunk-1", "doc:doc-1"],
        ),
        hit(
            RecallSource::Knowledge,
            "chunk-1",
            "archon uses cozodb",
            1,
            &["chunk:chunk-1"],
        ),
    ]);

    assert_eq!(merged.hits.len(), 1, "duplicate content was not folded");
    let survivor = &merged.hits[0];
    assert_eq!(survivor.source, RecallSource::Docs, "best rank should win");
    assert_eq!(survivor.duplicates.len(), 1);
    assert_eq!(survivor.duplicates[0].source, RecallSource::Knowledge);
    // The union is what proves the two stores' references name one artifact.
    assert_eq!(survivor.provenance_refs, vec!["chunk:chunk-1", "doc:doc-1"]);
    assert!(merged.conflicts.is_empty());
}

#[test]
fn one_provenance_ref_with_two_contents_is_a_conflict_and_both_survive() {
    let merged = merge(vec![
        hit(
            RecallSource::Docs,
            "chunk-1",
            "Retention is thirty days.",
            0,
            &["chunk:chunk-1"],
        ),
        hit(
            RecallSource::Knowledge,
            "chunk-1-v2",
            "Retention is ninety days.",
            0,
            &["chunk:chunk-1"],
        ),
    ]);

    assert_eq!(merged.hits.len(), 2, "a conflict must not be deduped away");
    assert_eq!(merged.conflicts.len(), 1);
    let conflict = &merged.conflicts[0];
    assert_eq!(conflict.kind, ConflictKind::DivergentContentForProvenance);
    assert_eq!(conflict.identity, "chunk:chunk-1");
    assert_eq!(conflict.members.len(), 2);
    assert!(merged.hits.iter().all(|hit| hit.conflicts == vec![0]));
}

#[test]
fn opposite_polarity_across_two_stores_is_surfaced() {
    let merged = merge(vec![
        hit(
            RecallSource::Memory,
            "mem-1",
            "The plugin is safe.",
            0,
            &["memory:mem-1"],
        ),
        hit(
            RecallSource::Docs,
            "chunk-9",
            "The plugin is not safe.",
            0,
            &["chunk:chunk-9"],
        ),
    ]);

    assert_eq!(merged.hits.len(), 2);
    let polarity: Vec<&RecallConflict> = merged
        .conflicts
        .iter()
        .filter(|conflict| conflict.kind == ConflictKind::OppositePolarity)
        .collect();
    assert_eq!(polarity.len(), 1, "{:?}", merged.conflicts);
    assert_eq!(polarity[0].identity, "plugin / safe");
    assert!(polarity[0].explanation.contains("memory"));
    // Each hit points at the conflict it is party to.
    assert!(merged.hits.iter().all(|hit| !hit.conflicts.is_empty()));
}

#[test]
fn agreeing_hits_from_different_artifacts_are_not_a_conflict() {
    let merged = merge(vec![
        hit(
            RecallSource::Memory,
            "mem-1",
            "The plugin is safe.",
            0,
            &["memory:mem-1"],
        ),
        hit(
            RecallSource::Docs,
            "chunk-9",
            "The runtime is fast.",
            0,
            &["chunk:chunk-9"],
        ),
    ]);
    assert_eq!(merged.hits.len(), 2);
    assert!(merged.conflicts.is_empty());
}

/// The merge runs on results from four threads, so its order must not depend on
/// arrival order. Same input in any permutation, same output.
#[test]
fn merge_order_is_independent_of_input_order() {
    let build = || {
        vec![
            hit(RecallSource::Code, "a.rs:1", "alpha", 1, &["file:a.rs"]),
            hit(RecallSource::Memory, "m1", "beta", 0, &["memory:m1"]),
            hit(RecallSource::Docs, "c1", "gamma", 0, &["chunk:c1"]),
        ]
    };
    let mut forward = build();
    let mut backward = build();
    backward.reverse();

    let a = merge(std::mem::take(&mut forward));
    let b = merge(std::mem::take(&mut backward));
    let ids = |m: &Merged| -> Vec<String> {
        m.hits
            .iter()
            .map(|hit| format!("{}/{}", hit.source, hit.source_id))
            .collect()
    };
    assert_eq!(ids(&a), ids(&b));
    // Rank 0 beats rank 1; between two rank-0 hits the source enum order wins.
    assert_eq!(ids(&a), vec!["memory/m1", "docs/c1", "code/a.rs:1"]);
}

#[test]
fn the_same_record_returned_twice_does_not_become_its_own_duplicate() {
    let merged = merge(vec![
        hit(RecallSource::Docs, "c1", "same text", 0, &["chunk:c1"]),
        hit(RecallSource::Docs, "c1", "same text", 0, &["chunk:c1"]),
    ]);
    assert_eq!(merged.hits.len(), 1);
    assert!(merged.hits[0].duplicates.is_empty());
}
