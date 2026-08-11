//! The compatibility predicate, tested as a predicate.
//!
//! Each condition gets a test that fails if that condition alone is removed.
//! A single "clusters similar things" test would pass against a plain group-by,
//! which is the implementation this module exists to avoid.

use chrono::{Duration, Utc};

use super::{
    DERIVED_TAG, Ineligible, compatible_clusters, ineligible_reason, provenance_compatible,
};
use crate::types::{Memory, MemoryType, RETIRED_TAG, SUPERSEDED_TAG};

fn memory(id: &str, age_days: i64) -> Memory {
    Memory {
        id: id.into(),
        content: format!("observation recorded as {id}"),
        title: "observation".into(),
        memory_type: MemoryType::Fact,
        importance: 0.5,
        tags: Vec::new(),
        source_type: "extraction".into(),
        project_path: "scope-a".into(),
        created_at: Utc::now() - Duration::days(age_days),
        updated_at: None,
        access_count: 0,
        last_accessed: None,
    }
}

fn week() -> Duration {
    Duration::days(7)
}

#[test]
fn two_ordinary_memories_from_one_scope_are_compatible() {
    assert!(provenance_compatible(
        &memory("a", 0),
        &memory("b", 1),
        week()
    ));
}

#[test]
fn a_memory_is_not_compatible_with_itself() {
    // A cluster built from one row repeated is a duplicate wearing a derived
    // label, not corroboration.
    let one = memory("a", 0);
    assert!(!provenance_compatible(&one, &one.clone(), week()));
}

#[test]
fn a_different_project_scope_is_incompatible() {
    // The failure this prevents: a preference stated for one project becoming a
    // claim about every project.
    let mut other = memory("b", 0);
    other.project_path = "scope-b".into();

    assert!(!provenance_compatible(&memory("a", 0), &other, week()));
}

#[test]
fn a_different_writer_is_incompatible() {
    // A fact the user stated and one a model guessed are not interchangeable
    // evidence, however alike they read.
    let mut other = memory("b", 0);
    other.source_type = "user".into();

    assert!(!provenance_compatible(&memory("a", 0), &other, week()));
}

#[test]
fn a_different_memory_type_is_incompatible() {
    let mut other = memory("b", 0);
    other.memory_type = MemoryType::Preference;

    assert!(!provenance_compatible(&memory("a", 0), &other, week()));
}

#[test]
fn memories_recorded_too_far_apart_are_incompatible() {
    // Two records of "the current target" a year apart are a record of a
    // change. Consolidating them asserts the older one is still true.
    assert!(!provenance_compatible(
        &memory("a", 0),
        &memory("b", 400),
        week()
    ));
}

#[test]
fn the_span_check_is_symmetric() {
    let recent = memory("a", 0);
    let old = memory("b", 400);

    assert_eq!(
        provenance_compatible(&recent, &old, week()),
        provenance_compatible(&old, &recent, week()),
        "which argument came first must not decide compatibility"
    );
}

#[test]
fn withheld_derived_bookkeeping_and_rule_rows_are_all_ineligible() {
    let mut superseded = memory("a", 0);
    superseded.tags.push(SUPERSEDED_TAG.into());
    assert_eq!(ineligible_reason(&superseded), Some(Ineligible::Withheld));

    let mut retired = memory("b", 0);
    retired.tags.push(RETIRED_TAG.into());
    assert_eq!(
        ineligible_reason(&retired),
        Some(Ineligible::Withheld),
        "a retired memory must not be resurrected by being consolidated"
    );

    let mut derived = memory("c", 0);
    derived.tags.push(DERIVED_TAG.into());
    assert_eq!(
        ineligible_reason(&derived),
        Some(Ineligible::AlreadyDerived),
        "consolidation output re-entering a cluster double-counts its sources"
    );

    let mut bookkeeping = memory("d", 0);
    bookkeeping.source_type = "garden".into();
    assert_eq!(
        ineligible_reason(&bookkeeping),
        Some(Ineligible::Bookkeeping)
    );

    let mut rule = memory("e", 0);
    rule.memory_type = MemoryType::Rule;
    assert_eq!(
        ineligible_reason(&rule),
        Some(Ineligible::PromptRule),
        "a semantic memory synthesised from rule text is a rule mutation in disguise"
    );

    assert_eq!(ineligible_reason(&memory("f", 0)), None);
}

#[test]
fn an_ineligible_row_cannot_enter_through_a_matching_partner() {
    let mut derived = memory("b", 0);
    derived.tags.push(DERIVED_TAG.into());

    assert!(
        !provenance_compatible(&memory("a", 0), &derived, week()),
        "unary eligibility must be checked before provenance identity, or a \
         perfect provenance match would readmit an excluded row"
    );
}

#[test]
fn a_cluster_requires_every_pair_to_be_compatible_not_just_neighbours() {
    // THE test for this module. Three memories five days apart in sequence:
    // a-b and b-c are inside a seven-day span, a-c is ten days and is not.
    // A group-by, or a chain-based clustering, would admit all three and
    // produce a cluster spanning ten days from a seven-day rule.
    let memories = vec![memory("a", 10), memory("b", 5), memory("c", 0)];

    let clusters = compatible_clusters(&memories, 2, week());

    for cluster in &clusters {
        for (position, &left) in cluster.iter().enumerate() {
            for &right in &cluster[position + 1..] {
                assert!(
                    provenance_compatible(&memories[left], &memories[right], week()),
                    "cluster {cluster:?} contains an incompatible pair; the span \
                     bound has become a chain-length bound"
                );
            }
        }
    }
    assert!(
        clusters.iter().all(|cluster| cluster.len() < 3),
        "all three were admitted, so pairwise checking is not happening"
    );
}

#[test]
fn a_compatible_group_clusters_together() {
    // The predicate must not be so strict that it never groups anything; a
    // clusterer that admits nothing also never invents a memory.
    let memories = vec![memory("a", 0), memory("b", 1), memory("c", 2)];

    let clusters = compatible_clusters(&memories, 2, week());

    assert_eq!(clusters.len(), 1);
    assert_eq!(clusters[0].len(), 3);
}

#[test]
fn clusters_below_the_minimum_size_are_not_returned() {
    let memories = vec![memory("a", 0), memory("b", 1)];

    assert!(compatible_clusters(&memories, 3, week()).is_empty());
}

#[test]
fn a_minimum_size_below_two_produces_nothing() {
    // A cluster of one is a memory. Promoting it would create a second row
    // asserting the same thing with a derived label.
    let memories = vec![memory("a", 0), memory("b", 1)];

    assert!(compatible_clusters(&memories, 1, week()).is_empty());
    assert!(compatible_clusters(&memories, 0, week()).is_empty());
}

#[test]
fn a_memory_belongs_to_at_most_one_cluster() {
    // Two clusters sharing a member would each claim it as corroboration, so
    // one observation would support two derived memories.
    let memories = vec![
        memory("a", 0),
        memory("b", 1),
        memory("c", 2),
        memory("d", 3),
    ];

    let clusters = compatible_clusters(&memories, 2, week());

    let mut seen = std::collections::HashSet::new();
    for cluster in &clusters {
        for &member in cluster {
            assert!(seen.insert(member), "memory {member} is in two clusters");
        }
    }
}

#[test]
fn clustering_is_deterministic_for_a_fixed_input() {
    // An unattended job that proposes a different consolidation each night
    // gives a reviewer nothing stable to decide about.
    let memories = vec![
        memory("a", 0),
        memory("b", 1),
        memory("c", 2),
        memory("d", 30),
        memory("e", 31),
    ];

    let first = compatible_clusters(&memories, 2, week());
    let second = compatible_clusters(&memories, 2, week());

    assert_eq!(first, second);
    assert_eq!(
        first.len(),
        2,
        "the two time-separated groups stay separate"
    );
}
