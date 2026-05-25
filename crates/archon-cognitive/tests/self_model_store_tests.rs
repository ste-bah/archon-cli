use archon_cognitive::self_model::{FactKind, SelfModelFact, SelfModelStore};
use cozo::DbInstance;

#[test]
fn cold_start_returns_neutral_domain_trust() {
    let db = DbInstance::new("mem", "", "").expect("mem db");
    let store = SelfModelStore::new(&db).expect("store");

    let trust = store.get_domain_trust("ci").expect("trust");

    assert_eq!(trust.domain, "ci");
    assert_eq!(trust.trust_score, 0.5);
    assert_eq!(trust.evidence_count, 0);
    assert!(trust.failure_cluster_ids.is_empty());
}

#[test]
fn domain_trust_uses_evidence_weighted_confidence() {
    let db = DbInstance::new("mem", "", "").expect("mem db");
    let store = SelfModelStore::new(&db).expect("store");
    store
        .write_facts(&[
            SelfModelFact::new("coding", FactKind::DomainTrust, "verified edit", 0.8, 4),
            SelfModelFact::new("coding", FactKind::DomainTrust, "missed test", 0.2, 1),
        ])
        .expect("write facts");

    let trust = store.get_domain_trust("coding").expect("trust");

    assert_eq!(trust.evidence_count, 5);
    assert!((trust.trust_score - 0.68).abs() < 0.001);
}

#[test]
fn failure_clusters_and_briefing_are_queryable() {
    let db = DbInstance::new("mem", "", "").expect("mem db");
    let store = SelfModelStore::new(&db).expect("store");
    store
        .write_facts(&[
            SelfModelFact::new(
                "research",
                FactKind::FailureCluster,
                "source verification gaps",
                0.9,
                3,
            ),
            SelfModelFact::new(
                "research",
                FactKind::CautionRule,
                "cite primary sources before conclusion",
                0.8,
                2,
            ),
        ])
        .expect("write facts");

    let clusters = store
        .query_failure_clusters("research", 10)
        .expect("clusters");
    let briefing = store.export_briefing().expect("briefing");
    let profile = store
        .read_profile(&["research".to_string()])
        .expect("profile");

    assert_eq!(clusters.len(), 1);
    assert_eq!(clusters[0].pattern_summary, "source verification gaps");
    assert_eq!(briefing.fact_count, 2);
    assert_eq!(briefing.active_failure_clusters, 1);
    assert_eq!(profile.caution_rules.len(), 1);
}
