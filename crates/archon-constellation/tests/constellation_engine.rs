use std::collections::BTreeMap;

use cozo::{DataValue, DbInstance, ScriptMutability};

fn db() -> DbInstance {
    DbInstance::new("mem", "", Default::default()).unwrap()
}

fn seed_sample(
    db: &DbInstance,
    id: &str,
    workspace: &str,
    label: &str,
    event_type: &str,
    text: &str,
) {
    archon_meaning::ensure_schema(db).unwrap();
    let mut params = BTreeMap::new();
    params.insert("id".into(), DataValue::from(id));
    params.insert("wid".into(), DataValue::from(workspace));
    params.insert("aid".into(), DataValue::from(format!("artifact-{id}").as_str()));
    params.insert("label".into(), DataValue::from(label));
    params.insert("eid".into(), DataValue::from(format!("event-{id}").as_str()));
    params.insert("et".into(), DataValue::from(event_type));
    params.insert("txt".into(), DataValue::from(text));
    params.insert("meta".into(), DataValue::from("{}"));
    params.insert("ts".into(), DataValue::from("2026-01-01T00:00:00Z"));
    db.run_script(
        "?[sample_id, workspace_id, artifact_id, label, source_event_id, event_type, text, metadata_json, created_at] <- \
         [[$id, $wid, $aid, $label, $eid, $et, $txt, $meta, $ts]] \
         :put meaning_samples { sample_id => workspace_id, artifact_id, label, source_event_id, event_type, text, metadata_json, created_at }",
        params,
        ScriptMutability::Mutable,
    )
    .unwrap();
}

#[test]
fn build_creates_centroid_from_positive_samples() {
    let db = db();
    seed_sample(&db, "p1", "workspace", "positive", "UserAccepted", "secure tested patch");
    seed_sample(&db, "p2", "workspace", "positive", "CompletionClaimVerified", "secure permission boundary");
    let report = archon_constellation::build_constellation(&db, "project").unwrap();
    assert_eq!(report.sample_count, 2);
    assert_eq!(report.vector_rows, 1);
    assert!(report.centroid_id.is_some());
}

#[test]
fn build_ignores_negative_samples() {
    let db = db();
    seed_sample(&db, "p1", "workspace", "positive", "UserAccepted", "stable retry policy");
    seed_sample(&db, "n1", "workspace", "negative", "UserCorrected", "broken unsafe shortcut");
    let report = archon_constellation::build_constellation(&db, "project").unwrap();
    let centroids = archon_constellation::list_centroids(&db).unwrap();
    assert_eq!(report.sample_count, 1);
    assert_eq!(centroids[0].sample_ids, vec!["p1".to_string()]);
}

#[test]
fn empty_store_produces_no_centroid() {
    let db = db();
    let report = archon_constellation::build_constellation(&db, "project").unwrap();
    assert_eq!(report.sample_count, 0);
    assert!(report.centroid_id.is_none());
}

#[test]
fn score_returns_high_similarity_for_matching_text() {
    let db = db();
    seed_sample(&db, "p1", "workspace", "positive", "UserAccepted", "permission boundary regression tests");
    archon_constellation::build_constellation(&db, "project").unwrap();
    let score = archon_constellation::score_text(&db, "project", "permission boundary regression tests").unwrap();
    assert!(score.similarity > 0.7);
}

#[test]
fn drift_flags_far_text_below_threshold() {
    let db = db();
    seed_sample(&db, "p1", "workspace", "positive", "UserAccepted", "permission boundary regression tests");
    archon_constellation::build_constellation(&db, "project").unwrap();
    let report = archon_constellation::detect_drift(&db, "project", "banana invoice watercolor", 0.8).unwrap();
    assert!(report.drifted);
}

#[test]
fn list_centroids_reads_persisted_rows() {
    let db = db();
    seed_sample(&db, "p1", "gametheory-runs", "positive", "GameTheoryFinalReport", "auction incentive mechanism");
    archon_constellation::build_constellation(&db, "strategic-workflow").unwrap();
    let centroids = archon_constellation::list_centroids(&db).unwrap();
    assert_eq!(centroids.len(), 1);
    assert_eq!(centroids[0].target, "strategic-workflow");
}

#[test]
fn build_versions_repeat_centroids() {
    let db = db();
    seed_sample(&db, "p1", "workspace", "positive", "UserAccepted", "secure tested output");
    let first = archon_constellation::build_constellation(&db, "project").unwrap();
    let second = archon_constellation::build_constellation(&db, "project").unwrap();
    assert_eq!(first.version, Some(1));
    assert_eq!(second.version, Some(2));
}

#[test]
fn missing_centroid_is_reported_for_score() {
    let db = db();
    archon_constellation::ensure_schema(&db).unwrap();
    let err = archon_constellation::score_text(&db, "project", "some answer").unwrap_err();
    assert!(err.to_string().contains("no constellation centroid"));
}

#[test]
fn typo_target_does_not_build_broad_centroid() {
    let db = db();
    seed_sample(&db, "p1", "workspace", "positive", "UserAccepted", "secure tested output");
    let err = archon_constellation::build_constellation(&db, "projct").unwrap_err();
    assert!(err.to_string().contains("invalid constellation target"));
    archon_constellation::ensure_schema(&db).unwrap();
    assert!(archon_constellation::list_centroids(&db).unwrap().is_empty());
}
