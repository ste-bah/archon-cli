use super::*;

#[test]
fn source_labels_round_trip() {
    for source in RecallSource::ALL {
        assert_eq!(RecallSource::parse(source.as_str()), Some(source));
    }
    assert_eq!(RecallSource::parse("nowhere"), None);
}

/// The enum order is the final tie-break in the merge, so it is a contract.
#[test]
fn source_order_is_pinned() {
    assert!(RecallSource::Memory < RecallSource::Docs);
    assert!(RecallSource::Docs < RecallSource::Knowledge);
    assert!(RecallSource::Knowledge < RecallSource::Code);
}

#[test]
fn even_share_never_lets_one_source_take_the_whole_limit() {
    let policy = SourcePolicy::even_share(&RecallSource::ALL, 10, Duration::from_secs(1));
    for budget in policy.budgets() {
        assert_eq!(
            budget.quota, 3,
            "{} took more than its share",
            budget.source
        );
    }
}

/// A limit smaller than the source count must still let every source speak;
/// a quota of zero would be silent omission wearing a rounding error.
#[test]
fn even_share_floors_quota_at_one() {
    let policy = SourcePolicy::even_share(&RecallSource::ALL, 1, Duration::from_secs(1));
    assert!(policy.budgets().iter().all(|budget| budget.quota == 1));
}

#[test]
fn from_budgets_keeps_the_last_entry_for_a_source() {
    let policy = SourcePolicy::from_budgets(vec![
        SourceBudget {
            source: RecallSource::Docs,
            quota: 2,
            latency_budget: Duration::from_secs(1),
        },
        SourceBudget {
            source: RecallSource::Docs,
            quota: 9,
            latency_budget: Duration::from_secs(2),
        },
    ]);
    assert_eq!(policy.budgets().len(), 1);
    assert_eq!(policy.budget_for(RecallSource::Docs).unwrap().quota, 9);
}

#[test]
fn policy_excludes_sources_it_does_not_name() {
    let policy = SourcePolicy::even_share(&[RecallSource::Docs], 4, Duration::from_secs(1));
    assert!(policy.allows(RecallSource::Docs));
    assert!(!policy.allows(RecallSource::Code));
    let query = RecallQuery {
        text: "x".into(),
        limit: 4,
        source_policy: policy,
    };
    assert_eq!(query.quota_for(RecallSource::Code), 0);
}

/// The only hit constructor stamps the placeholder calibration, so there is no
/// path by which a hit acquires a score without also acquiring the warning.
#[test]
fn every_constructed_hit_is_uncalibrated() {
    let hit = RecallHit::at_rank(RecallSource::Docs, "chunk-1", "text", 0);
    assert_eq!(hit.calibration, ScoreCalibration::UncalibratedRankOrder);
    assert!(!hit.calibration.is_measured());
    assert_eq!(hit.normalized_score, 1.0);
    assert!(hit.source_score.is_none());
}

#[test]
fn provenance_refs_are_sorted_and_deduped() {
    let hit = RecallHit::at_rank(RecallSource::Docs, "c", "t", 0).with_provenance([
        "doc:b".to_string(),
        "chunk:a".to_string(),
        "doc:b".into(),
    ]);
    assert_eq!(hit.provenance_refs, vec!["chunk:a", "doc:b"]);
}
