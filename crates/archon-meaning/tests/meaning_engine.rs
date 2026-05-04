use cozo::{DataValue, DbInstance, ScriptMutability};

fn test_db() -> DbInstance {
    DbInstance::new("mem", "", "").expect("mem db")
}

fn ensure_learning_schema(db: &DbInstance) {
    db.run_script(
        r#":create learning_events {
            event_id: String => workspace_id: String, event_type: String,
            source_artifact_id: String, outcome_artifact_id: String, signal: String,
            confidence: Float, provenance_record_id: String, created_at: String
        }"#,
        Default::default(),
        ScriptMutability::Mutable,
    )
    .expect("learning schema");
}

fn insert_event(db: &DbInstance, id: &str, workspace: &str, event_type: &str, text: &str) {
    let mut params = std::collections::BTreeMap::new();
    params.insert("id".into(), DataValue::from(id));
    params.insert("ws".into(), DataValue::from(workspace));
    params.insert("et".into(), DataValue::from(event_type));
    params.insert(
        "src".into(),
        DataValue::from(format!("source-{id}").as_str()),
    );
    params.insert(
        "out".into(),
        DataValue::from(format!("artifact-{id}").as_str()),
    );
    params.insert(
        "sig".into(),
        DataValue::from(serde_json::json!({"text": text}).to_string().as_str()),
    );
    params.insert("conf".into(), DataValue::from(0.9));
    params.insert(
        "prov".into(),
        DataValue::from(format!("prov-{id}").as_str()),
    );
    params.insert("ts".into(), DataValue::from("2026-01-01T00:00:00Z"));
    db.run_script(
        "?[event_id, workspace_id, event_type, source_artifact_id, outcome_artifact_id, signal, confidence, provenance_record_id, created_at] <- \
         [[$id, $ws, $et, $src, $out, $sig, $conf, $prov, $ts]] \
         :put learning_events { event_id => workspace_id, event_type, source_artifact_id, outcome_artifact_id, signal, confidence, provenance_record_id, created_at }",
        params,
        ScriptMutability::Mutable,
    )
    .expect("insert event");
}

fn seed_positive_negative(db: &DbInstance) {
    ensure_learning_schema(db);
    insert_event(db, "pos", "ws-1", "UserAccepted", "accepted answer");
    insert_event(db, "neg", "ws-1", "UserCorrected", "bad answer");
}

fn seed_gametheory_report(db: &DbInstance) {
    db.run_script(
        r#":create gt_final_reports {
            run_id: String => report_md: String, created_at: String,
            total_cost_usd: String, total_duration_ms: String
        }"#,
        Default::default(),
        ScriptMutability::Mutable,
    )
    .expect("gt schema");
    db.run_script(
        "?[run_id, report_md, created_at, total_cost_usd, total_duration_ms] <- \
         [['gt-1', '# Strategic report\nUse incentives carefully.', 'now', '0', '0']] \
         :put gt_final_reports { run_id => report_md, created_at, total_cost_usd, total_duration_ms }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .expect("gt report");
}

#[test]
fn build_creates_samples_pairs_triplets_and_dataset() {
    let db = test_db();
    seed_positive_negative(&db);
    let report = archon_meaning::build_from_learning_events(&db).unwrap();
    assert_eq!(report.events_seen, 2);
    assert_eq!(report.samples_created, 2);
    assert_eq!(report.pairs_created, 1);
    assert_eq!(report.triplets_created, 1);
    assert_eq!(report.datasets_created, 1);
}

#[test]
fn positive_and_negative_labels_are_persisted() {
    let db = test_db();
    seed_positive_negative(&db);
    archon_meaning::build_from_learning_events(&db).unwrap();
    let samples = archon_meaning::list_samples(&db).unwrap();
    assert!(
        samples
            .iter()
            .any(|s| s.label == archon_meaning::MeaningLabel::Positive)
    );
    assert!(
        samples
            .iter()
            .any(|s| s.label == archon_meaning::MeaningLabel::Negative)
    );
}

#[test]
fn unrelated_learning_events_do_not_create_samples() {
    let db = test_db();
    ensure_learning_schema(&db);
    insert_event(&db, "skip", "ws-1", "RetrievalUsed", "neutral");
    let report = archon_meaning::build_from_learning_events(&db).unwrap();
    assert_eq!(report.samples_created, 0);
    assert_eq!(archon_meaning::list_samples(&db).unwrap().len(), 0);
}

#[test]
fn pairs_do_not_cross_workspaces() {
    let db = test_db();
    ensure_learning_schema(&db);
    insert_event(&db, "pos", "ws-1", "UserAccepted", "accepted");
    insert_event(&db, "neg", "ws-2", "UserCorrected", "corrected");
    archon_meaning::build_from_learning_events(&db).unwrap();
    assert!(archon_meaning::list_pairs(&db).unwrap().is_empty());
}

#[test]
fn triplets_reference_distinct_positive_and_negative_samples() {
    let db = test_db();
    seed_positive_negative(&db);
    archon_meaning::build_from_learning_events(&db).unwrap();
    let triplets = archon_meaning::list_triplets(&db).unwrap();
    assert_eq!(triplets.len(), 1);
    assert_ne!(
        triplets[0].positive_sample_id,
        triplets[0].negative_sample_id
    );
}

#[test]
fn samples_export_as_jsonl() {
    let db = test_db();
    seed_positive_negative(&db);
    archon_meaning::build_from_learning_events(&db).unwrap();
    let jsonl =
        archon_meaning::export::samples_jsonl(&archon_meaning::list_samples(&db).unwrap()).unwrap();
    assert!(jsonl.contains("\"row_type\":\"sample\""));
    assert!(jsonl.contains("accepted answer"));
}

#[test]
fn triplets_export_as_jsonl() {
    let db = test_db();
    seed_positive_negative(&db);
    archon_meaning::build_from_learning_events(&db).unwrap();
    let jsonl =
        archon_meaning::export::triplets_jsonl(&archon_meaning::list_triplets(&db).unwrap())
            .unwrap();
    assert!(jsonl.contains("\"row_type\":\"triplet\""));
    assert!(jsonl.contains("positive_sample_id"));
}

#[test]
fn build_is_safe_when_learning_events_relation_is_missing() {
    let db = test_db();
    let report = archon_meaning::build_from_learning_events(&db).unwrap();
    assert_eq!(report.events_seen, 0);
    assert_eq!(report.samples_created, 0);
}

#[test]
fn gametheory_runs_source_builds_positive_report_samples() {
    let db = test_db();
    seed_gametheory_report(&db);
    let report = archon_meaning::build_from_gametheory_runs(&db).unwrap();
    assert_eq!(report.events_seen, 1);
    assert_eq!(report.samples_created, 1);
    let samples = archon_meaning::list_samples(&db).unwrap();
    assert_eq!(samples[0].workspace_id, "gametheory-runs");
    assert_eq!(samples[0].label, archon_meaning::MeaningLabel::Positive);
    assert!(samples[0].text.contains("Strategic report"));
}

#[test]
fn stable_ids_are_deterministic() {
    assert_eq!(
        archon_meaning::stable_id("sample", &["a", "b"]),
        archon_meaning::stable_id("sample", &["a", "b"])
    );
}

#[test]
fn sample_metadata_preserves_provenance_record_id() {
    let db = test_db();
    seed_positive_negative(&db);
    archon_meaning::build_from_learning_events(&db).unwrap();
    let samples = archon_meaning::list_samples(&db).unwrap();
    assert!(
        samples
            .iter()
            .any(|sample| sample.metadata_json["provenance_record_id"] == "prov-pos")
    );
}
