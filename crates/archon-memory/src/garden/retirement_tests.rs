use chrono::Utc;

use super::{PrunePolicy, RetirementCandidate, RetirementReason};
use crate::types::{Memory, MemoryType};

fn memory(content: &str) -> Memory {
    Memory {
        id: "mem-1".into(),
        content: content.into(),
        title: "a title".into(),
        memory_type: MemoryType::Fact,
        importance: 0.2,
        tags: vec!["x".into()],
        source_type: "test".into(),
        project_path: String::new(),
        created_at: Utc::now(),
        updated_at: None,
        access_count: 3,
        last_accessed: None,
    }
}

#[test]
fn only_the_delete_policy_permits_destruction() {
    assert!(PrunePolicy::Delete.may_delete());
    assert!(
        !PrunePolicy::Propose.may_delete(),
        "the policy an unattended pass runs under must never authorise a delete"
    );
}

#[test]
fn a_candidate_carries_enough_to_review_without_the_store() {
    let reason = RetirementReason::Stale {
        days_since_access: 91,
        staleness_days: 30,
        importance_floor: 0.3,
    };

    let candidate =
        RetirementCandidate::from_memory(&memory("something worth recognising"), reason);

    assert_eq!(candidate.memory_id, "mem-1");
    assert_eq!(candidate.title, "a title");
    assert_eq!(candidate.excerpt, "something worth recognising");
    assert_eq!(candidate.memory_type, MemoryType::Fact);
    assert_eq!(candidate.access_count, 3);
    assert_eq!(candidate.reason.kind(), "stale");
}

#[test]
fn an_excerpt_is_flattened_and_clipped_on_a_character_boundary() {
    // Content is arbitrary user text. A byte-index truncation would panic on a
    // multi-byte character, and a panic inside a background consolidation is a
    // pass that silently stops happening.
    let long: String = "é".repeat(1000);
    let candidate = RetirementCandidate::from_memory(
        &memory(&format!("line one\n\tline  two {long}")),
        RetirementReason::Overflow {
            max_memories: 10,
            total_memories: 11,
        },
    );

    assert_eq!(candidate.excerpt.chars().count(), 300);
    assert!(
        !candidate.excerpt.contains('\n'),
        "newlines must be flattened so a candidate renders on one line"
    );
    assert_eq!(candidate.reason.kind(), "overflow");
}

#[test]
fn a_candidate_round_trips_through_serde() {
    // Candidates cross a crate boundary into the governed store, so the shape
    // has to survive a serialize/deserialize cycle intact.
    let candidate = RetirementCandidate::from_memory(
        &memory("round trip"),
        RetirementReason::Stale {
            days_since_access: 40,
            staleness_days: 30,
            importance_floor: 0.3,
        },
    );

    let json = serde_json::to_string(&candidate).expect("serialize");
    let back: RetirementCandidate = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(back, candidate);
}
