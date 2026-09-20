//! Tests for the accounting reconciliation checks.
//!
//! The walk and the identity rule live in `v2::review_findings` now; these
//! pin the two properties the accounting check depends on, through that owner.

#[cfg(test)]
mod fanout_counting_tests {
    use crate::v2::review_findings::collect_findings;

    /// A fan-out record exposes its branches twice: `items` holds the raw item
    /// results and `outcomes` wraps them with attribution. Counting both made
    /// one finding look like two, so the accounting was always one short and
    /// the run was refused for dropping a finding nobody dropped.
    #[test]
    fn a_finding_mirrored_in_items_and_outcomes_counts_once() {
        let record = serde_json::json!({
            "items": [{ "data": { "findings": [{ "id": "F7" }] } }],
            "outcomes": [{ "result": { "data": { "findings": [{ "id": "F7" }] } } }],
        });
        assert_eq!(collect_findings(&record).len(), 1);
    }

    /// Two branches each raising the same finding is genuinely two.
    #[test]
    fn the_same_finding_from_two_branches_counts_twice() {
        let record = serde_json::json!({
            "outcomes": [
                { "result": { "data": { "findings": [{ "id": "F7" }] } } },
                { "result": { "data": { "findings": [{ "id": "F7" }] } } },
            ],
        });
        assert_eq!(collect_findings(&record).len(), 2);
    }

    /// A record with only `items` still contributes.
    #[test]
    fn items_alone_is_still_collected() {
        let record = serde_json::json!({
            "items": [{ "data": { "adversarial_findings": [{ "id": "F1" }] } }],
        });
        assert_eq!(collect_findings(&record).len(), 1);
    }
}

#[cfg(test)]
mod finding_identity_tests {
    use crate::v2::review_findings::multiset;

    /// Attribution stamped onto a finding, and the cross-cutting marker, do not
    /// change which finding it is. Keying on exact JSON reported those as
    /// missing, so a run was refused for dropping a finding it had enriched.
    #[test]
    fn enrichment_does_not_change_identity() {
        let raw = serde_json::json!({ "id": "F4", "severity": "medium" });
        let stamped = serde_json::json!({
            "id": "F4", "severity": "medium",
            "canonical_task_ids": ["TASK-X-010"],
            "finding_scope": "cross_cutting",
        });
        assert_eq!(multiset(&[raw]), multiset(&[stamped]));
    }

    /// Different findings stay different.
    #[test]
    fn distinct_findings_stay_distinct() {
        let a = serde_json::json!({ "id": "F4" });
        let b = serde_json::json!({ "id": "F7" });
        assert_ne!(multiset(&[a]), multiset(&[b]));
    }

    /// A finding with no identity field is still compared exactly.
    #[test]
    fn unidentifiable_findings_compare_exactly() {
        let a = serde_json::json!({ "severity": "low" });
        let b = serde_json::json!({ "severity": "high" });
        assert_ne!(multiset(&[a]), multiset(&[b]));
    }
}
