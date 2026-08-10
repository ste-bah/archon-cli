//! Issue #80(b): the startup briefing reports what was measured, and says so
//! when nothing was.

use archon_cognitive::SituationKind;
use archon_cognitive::self_model::briefing::{build, known_domains, trust_fact_id};
use cozo::{DbInstance, ScriptMutability};

fn db() -> DbInstance {
    let db = DbInstance::new("mem", "", "").unwrap();
    archon_cognitive::ensure_cognitive_schema(&db).unwrap();
    db
}

fn write_fact(db: &DbInstance, fact_id: &str, domain: &str, kind: &str, confidence: f64, n: i64) {
    let script = format!(
        "?[fact_id, domain, fact_kind, statement, confidence, evidence_count, last_seen_at, expires_at, created_at] <- \
         [['{fact_id}', '{domain}', '{kind}', 'statement for {fact_id}', {confidence}, {n}, '2026-01-01T00:00:00Z', '', '2026-01-01T00:00:00Z']]
         :put self_model_facts {{ fact_id => domain, fact_kind, statement, confidence, evidence_count, last_seen_at, expires_at, created_at }}"
    );
    db.run_script(&script, Default::default(), ScriptMutability::Mutable)
        .unwrap();
}

/// A self-model that has measured nothing has nothing to brief. Rendering a
/// block of neutral-looking lines would make an unmeasured model read as a
/// reported one.
#[test]
fn an_unmeasured_self_model_produces_no_briefing() {
    let briefing = build(&db(), Vec::new()).unwrap();

    assert!(briefing.is_empty());
    assert!(briefing.render().is_none());
    assert_eq!(briefing.unmeasured_domains, known_domains());
}

/// The rule the briefing exists to respect: a domain with no fact is named as
/// unmeasured, never given a default confidence.
#[test]
fn an_absent_domain_is_reported_absent_rather_than_neutral() {
    let db = db();
    write_fact(
        &db,
        &trust_fact_id("coding"),
        "coding",
        "domain_trust",
        0.62,
        14,
    );

    let briefing = build(&db, Vec::new()).unwrap();
    let text = briefing
        .render()
        .expect("a measured domain is worth briefing");

    assert_eq!(briefing.measured.len(), 1);
    assert_eq!(briefing.measured[0].domain, "coding");
    assert_eq!(briefing.measured[0].evidence_count, 14);
    assert!(
        text.contains("coding: confidence 0.62 over 14 verified outcomes"),
        "{text}"
    );
    assert!(text.contains("No measured evidence yet for:"), "{text}");
    assert!(!briefing.unmeasured_domains.contains(&"coding".to_string()));
    assert!(briefing.unmeasured_domains.contains(&"git".to_string()));
    // No fabricated number for a domain nobody measured.
    assert!(!text.contains("git: confidence"), "{text}");
}

/// Caution rules are operator policy, not learned evidence, so they are carried
/// through as text and never counted as measured domains.
#[test]
fn caution_rules_and_failure_clusters_are_reported_separately_from_trust() {
    let db = db();
    write_fact(&db, "caution:1", "git", "caution_rule", 1.0, 0);
    write_fact(&db, "cluster:1", "git", "failure_cluster", 0.9, 3);

    let briefing = build(&db, Vec::new()).unwrap();
    let text = briefing.render().expect("caution rules are worth briefing");

    assert!(briefing.measured.is_empty());
    assert_eq!(briefing.active_failure_clusters, 1);
    assert_eq!(briefing.caution_rules, vec!["statement for caution:1"]);
    assert!(text.contains("Caution: statement for caution:1"), "{text}");
    assert!(
        text.contains("Active failure clusters on record: 1"),
        "{text}"
    );
    // A caution rule for `git` does not make `git` a measured domain.
    assert!(briefing.unmeasured_domains.contains(&"git".to_string()));
}

/// A `NaN` confidence is not a measurement, and rendering it would put the word
/// `NaN` in the prompt where a reader would take it for an observation.
#[test]
fn a_non_finite_confidence_is_not_briefed_as_a_measurement() {
    let db = db();
    let mut params = std::collections::BTreeMap::new();
    params.insert("confidence".to_string(), cozo::DataValue::from(f64::NAN));
    db.run_script(
        "?[fact_id, domain, fact_kind, statement, confidence, evidence_count, last_seen_at, expires_at, created_at] <- \
         [['domain_trust:coding', 'coding', 'domain_trust', 'broken', $confidence, 4, '2026-01-01T00:00:00Z', '', '2026-01-01T00:00:00Z']]
         :put self_model_facts { fact_id => domain, fact_kind, statement, confidence, evidence_count, last_seen_at, expires_at, created_at }",
        params,
        ScriptMutability::Mutable,
    )
    .unwrap();

    let briefing = build(&db, Vec::new()).unwrap();

    assert!(briefing.measured.is_empty());
    assert!(briefing.unmeasured_domains.contains(&"coding".to_string()));
}

/// The domain enumeration has to follow the situation classifier. A kind added
/// later without a domain mapping would silently drop out of the briefing.
#[test]
fn every_situation_kind_maps_into_the_briefed_domain_set() {
    let domains = known_domains();

    assert_eq!(SituationKind::ALL.len(), 10);
    assert!(!domains.is_empty());
    for domain in [
        "ci",
        "coding",
        "git",
        "pipeline",
        "research",
        "world_model",
        "safety",
        "general",
    ] {
        assert!(domains.contains(&domain.to_string()), "missing {domain}");
    }
}
