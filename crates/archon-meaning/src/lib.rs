//! Meaning compiler for governed-learning signals.

pub mod contrastive;
pub mod errors;
pub mod eval_dataset_builder;
pub mod export;
pub mod resolver;
pub mod samples;
pub mod triplets;

use std::collections::BTreeMap;

use cozo::{DataValue, DbInstance, ScriptMutability};
use sha2::{Digest, Sha256};

pub use errors::{MeaningError, Result};
pub use resolver::{
    FALLBACK_FEATURE_SPACE, HydratedTriplet, STORED_EMBEDDING_FEATURE_SPACE,
    list_hydrated_triplets, resolve_triplet_embeddings,
};
pub use samples::{MeaningLabel, MeaningSample};
pub use triplets::TripletRecord;

use contrastive::ContrastivePair;
use eval_dataset_builder::EvalDataset;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildReport {
    pub events_seen: usize,
    pub samples_created: usize,
    pub pairs_created: usize,
    pub triplets_created: usize,
    pub datasets_created: usize,
}

#[derive(Debug, Clone)]
struct LearningEventRow {
    event_id: String,
    workspace_id: String,
    event_type: String,
    source_artifact_id: String,
    outcome_artifact_id: Option<String>,
    signal: serde_json::Value,
    confidence: f64,
    provenance_record_id: String,
    created_at: String,
}

pub fn ensure_schema(db: &DbInstance) -> Result<()> {
    for script in [
        r#":create meaning_samples {
            sample_id: String => workspace_id: String, artifact_id: String,
            label: String, source_event_id: String, event_type: String,
            text: String, metadata_json: String, created_at: String
        }"#,
        r#":create meaning_contrastive_pairs {
            pair_id: String => workspace_id: String, positive_sample_id: String,
            negative_sample_id: String, anchor_artifact_id: String, created_at: String
        }"#,
        r#":create meaning_triplets {
            triplet_id: String => workspace_id: String, anchor_artifact_id: String,
            positive_sample_id: String, negative_sample_id: String, created_at: String
        }"#,
        r#":create meaning_eval_datasets {
            dataset_id: String => sample_count: Int, triplet_count: Int, created_at: String
        }"#,
    ] {
        run_create(db, script)?;
    }
    Ok(())
}

pub fn build_from_learning_events(db: &DbInstance) -> Result<BuildReport> {
    ensure_schema(db)?;
    let events = read_learning_events(db)?;
    let now = chrono::Utc::now().to_rfc3339();
    let samples = derive_samples(&events, &now);
    persist_build(db, events.len(), &samples, &now)
}

pub fn build_from_gametheory_runs(db: &DbInstance) -> Result<BuildReport> {
    ensure_schema(db)?;
    let now = chrono::Utc::now().to_rfc3339();
    let samples = read_gametheory_report_samples(db, &now)?;
    persist_build(db, samples.len(), &samples, &now)
}

fn persist_build(
    db: &DbInstance,
    events_seen: usize,
    samples: &[MeaningSample],
    now: &str,
) -> Result<BuildReport> {
    for sample in samples {
        insert_sample(db, sample)?;
    }
    let pairs = contrastive::build_pairs(samples, now);
    for pair in &pairs {
        insert_pair(db, pair)?;
    }
    let triplets = triplets::build_triplets(&pairs, now);
    for triplet in &triplets {
        insert_triplet(db, triplet)?;
    }
    let dataset = eval_dataset_builder::build_dataset(samples, &triplets, now);
    insert_dataset(db, &dataset)?;
    Ok(BuildReport {
        events_seen,
        samples_created: samples.len(),
        pairs_created: pairs.len(),
        triplets_created: triplets.len(),
        datasets_created: 1,
    })
}

fn derive_samples(events: &[LearningEventRow], now: &str) -> Vec<MeaningSample> {
    events
        .iter()
        .filter_map(|event| {
            let label = samples::classify_event(&event.event_type)?;
            let artifact_id = event
                .outcome_artifact_id
                .clone()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| event.source_artifact_id.clone());
            Some(MeaningSample {
                sample_id: stable_id("sample", &[&event.event_id, label.as_str()]),
                workspace_id: event.workspace_id.clone(),
                artifact_id,
                label,
                source_event_id: event.event_id.clone(),
                event_type: event.event_type.clone(),
                text: samples::sample_text(&event.signal, &event.source_artifact_id),
                metadata_json: serde_json::json!({
                    "confidence": event.confidence,
                    "provenance_record_id": event.provenance_record_id,
                    "event_created_at": event.created_at
                }),
                created_at: now.to_string(),
            })
        })
        .collect()
}

fn read_gametheory_report_samples(db: &DbInstance, now: &str) -> Result<Vec<MeaningSample>> {
    let result = db.run_script(
        "?[run_id, report_md, created_at] := *gt_final_reports{run_id, report_md, created_at}",
        Default::default(),
        ScriptMutability::Immutable,
    );
    let rows = match result {
        Ok(rows) => rows,
        Err(e) if relation_missing(&e.to_string()) => return Ok(Vec::new()),
        Err(e) => {
            return Err(MeaningError::Store(format!(
                "read gt_final_reports failed: {e}"
            )));
        }
    };
    Ok(rows
        .rows
        .iter()
        .map(|row| {
            let run_id = str_col(row, 0);
            let report_md = str_col(row, 1);
            MeaningSample {
                sample_id: stable_id("sample", &[&run_id, "gametheory-final-report"]),
                workspace_id: "gametheory-runs".into(),
                artifact_id: format!("final-report:{run_id}"),
                label: MeaningLabel::Positive,
                source_event_id: run_id,
                event_type: "GameTheoryFinalReport".into(),
                text: report_md,
                metadata_json: serde_json::json!({
                    "source": "gt_final_reports",
                    "report_created_at": str_col(row, 2)
                }),
                created_at: now.to_string(),
            }
        })
        .collect())
}

pub fn list_samples(db: &DbInstance) -> Result<Vec<MeaningSample>> {
    ensure_schema(db)?;
    let result = db
        .run_script(
            "?[sample_id, workspace_id, artifact_id, label, source_event_id, event_type, text, metadata_json, created_at] := \
             *meaning_samples{sample_id, workspace_id, artifact_id, label, source_event_id, event_type, text, metadata_json, created_at}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| MeaningError::Store(format!("list meaning_samples failed: {e}")))?;
    result.rows.iter().map(|row| row_to_sample(row)).collect()
}

pub fn list_pairs(db: &DbInstance) -> Result<Vec<ContrastivePair>> {
    ensure_schema(db)?;
    let result = db
        .run_script(
            "?[pair_id, workspace_id, positive_sample_id, negative_sample_id, anchor_artifact_id, created_at] := \
             *meaning_contrastive_pairs{pair_id, workspace_id, positive_sample_id, negative_sample_id, anchor_artifact_id, created_at}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| MeaningError::Store(format!("list meaning pairs failed: {e}")))?;
    Ok(result
        .rows
        .iter()
        .map(|row| ContrastivePair {
            pair_id: str_col(row, 0),
            workspace_id: str_col(row, 1),
            positive_sample_id: str_col(row, 2),
            negative_sample_id: str_col(row, 3),
            anchor_artifact_id: str_col(row, 4),
            created_at: str_col(row, 5),
        })
        .collect())
}

pub fn list_triplets(db: &DbInstance) -> Result<Vec<TripletRecord>> {
    ensure_schema(db)?;
    let result = db
        .run_script(
            "?[triplet_id, workspace_id, anchor_artifact_id, positive_sample_id, negative_sample_id, created_at] := \
             *meaning_triplets{triplet_id, workspace_id, anchor_artifact_id, positive_sample_id, negative_sample_id, created_at}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| MeaningError::Store(format!("list meaning_triplets failed: {e}")))?;
    Ok(result
        .rows
        .iter()
        .map(|row| TripletRecord {
            triplet_id: str_col(row, 0),
            workspace_id: str_col(row, 1),
            anchor_artifact_id: str_col(row, 2),
            positive_sample_id: str_col(row, 3),
            negative_sample_id: str_col(row, 4),
            created_at: str_col(row, 5),
        })
        .collect())
}

fn read_learning_events(db: &DbInstance) -> Result<Vec<LearningEventRow>> {
    let result = db.run_script(
        "?[event_id, workspace_id, event_type, source_artifact_id, outcome_artifact_id, signal, confidence, provenance_record_id, created_at] := \
         *learning_events{event_id, workspace_id, event_type, source_artifact_id, outcome_artifact_id, signal, confidence, provenance_record_id, created_at}",
        Default::default(),
        ScriptMutability::Immutable,
    );
    let rows = match result {
        Ok(rows) => rows,
        Err(e) if relation_missing(&e.to_string()) => return Ok(Vec::new()),
        Err(e) => {
            return Err(MeaningError::Store(format!(
                "read learning_events failed: {e}"
            )));
        }
    };
    rows.rows
        .iter()
        .map(|row| row_to_learning_event(row))
        .collect()
}

fn insert_sample(db: &DbInstance, sample: &MeaningSample) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("id".into(), DataValue::from(sample.sample_id.as_str()));
    params.insert("wid".into(), DataValue::from(sample.workspace_id.as_str()));
    params.insert("aid".into(), DataValue::from(sample.artifact_id.as_str()));
    params.insert("label".into(), DataValue::from(sample.label.as_str()));
    params.insert(
        "eid".into(),
        DataValue::from(sample.source_event_id.as_str()),
    );
    params.insert("et".into(), DataValue::from(sample.event_type.as_str()));
    params.insert("txt".into(), DataValue::from(sample.text.as_str()));
    params.insert(
        "meta".into(),
        DataValue::from(sample.metadata_json.to_string().as_str()),
    );
    params.insert("ts".into(), DataValue::from(sample.created_at.as_str()));
    db.run_script(
        "?[sample_id, workspace_id, artifact_id, label, source_event_id, event_type, text, metadata_json, created_at] <- \
         [[$id, $wid, $aid, $label, $eid, $et, $txt, $meta, $ts]] \
         :put meaning_samples { sample_id => workspace_id, artifact_id, label, source_event_id, event_type, text, metadata_json, created_at }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| MeaningError::Store(format!("insert meaning sample failed: {e}")))?;
    Ok(())
}

fn insert_pair(db: &DbInstance, pair: &ContrastivePair) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("id".into(), DataValue::from(pair.pair_id.as_str()));
    params.insert("wid".into(), DataValue::from(pair.workspace_id.as_str()));
    params.insert(
        "pos".into(),
        DataValue::from(pair.positive_sample_id.as_str()),
    );
    params.insert(
        "neg".into(),
        DataValue::from(pair.negative_sample_id.as_str()),
    );
    params.insert(
        "anchor".into(),
        DataValue::from(pair.anchor_artifact_id.as_str()),
    );
    params.insert("ts".into(), DataValue::from(pair.created_at.as_str()));
    db.run_script(
        "?[pair_id, workspace_id, positive_sample_id, negative_sample_id, anchor_artifact_id, created_at] <- \
         [[$id, $wid, $pos, $neg, $anchor, $ts]] \
         :put meaning_contrastive_pairs { pair_id => workspace_id, positive_sample_id, negative_sample_id, anchor_artifact_id, created_at }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| MeaningError::Store(format!("insert meaning pair failed: {e}")))?;
    Ok(())
}

fn insert_triplet(db: &DbInstance, triplet: &TripletRecord) -> Result<()> {
    triplet.validate()?;
    let mut params = BTreeMap::new();
    params.insert("id".into(), DataValue::from(triplet.triplet_id.as_str()));
    params.insert("wid".into(), DataValue::from(triplet.workspace_id.as_str()));
    params.insert(
        "anchor".into(),
        DataValue::from(triplet.anchor_artifact_id.as_str()),
    );
    params.insert(
        "pos".into(),
        DataValue::from(triplet.positive_sample_id.as_str()),
    );
    params.insert(
        "neg".into(),
        DataValue::from(triplet.negative_sample_id.as_str()),
    );
    params.insert("ts".into(), DataValue::from(triplet.created_at.as_str()));
    db.run_script(
        "?[triplet_id, workspace_id, anchor_artifact_id, positive_sample_id, negative_sample_id, created_at] <- \
         [[$id, $wid, $anchor, $pos, $neg, $ts]] \
         :put meaning_triplets { triplet_id => workspace_id, anchor_artifact_id, positive_sample_id, negative_sample_id, created_at }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| MeaningError::Store(format!("insert meaning triplet failed: {e}")))?;
    Ok(())
}

fn insert_dataset(db: &DbInstance, dataset: &EvalDataset) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("id".into(), DataValue::from(dataset.dataset_id.as_str()));
    params.insert(
        "samples".into(),
        DataValue::from(dataset.sample_count as i64),
    );
    params.insert(
        "triplets".into(),
        DataValue::from(dataset.triplet_count as i64),
    );
    params.insert("ts".into(), DataValue::from(dataset.created_at.as_str()));
    db.run_script(
        "?[dataset_id, sample_count, triplet_count, created_at] <- [[$id, $samples, $triplets, $ts]] \
         :put meaning_eval_datasets { dataset_id => sample_count, triplet_count, created_at }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| MeaningError::Store(format!("insert eval dataset failed: {e}")))?;
    Ok(())
}

fn row_to_learning_event(row: &[DataValue]) -> Result<LearningEventRow> {
    Ok(LearningEventRow {
        event_id: str_col(row, 0),
        workspace_id: str_col(row, 1),
        event_type: str_col(row, 2),
        source_artifact_id: str_col(row, 3),
        outcome_artifact_id: non_empty(row, 4),
        signal: serde_json::from_str(&str_col(row, 5))?,
        confidence: row.get(6).and_then(DataValue::get_float).unwrap_or(0.0),
        provenance_record_id: str_col(row, 7),
        created_at: str_col(row, 8),
    })
}

fn row_to_sample(row: &[DataValue]) -> Result<MeaningSample> {
    Ok(MeaningSample {
        sample_id: str_col(row, 0),
        workspace_id: str_col(row, 1),
        artifact_id: str_col(row, 2),
        label: MeaningLabel::parse(&str_col(row, 3)).unwrap_or(MeaningLabel::Negative),
        source_event_id: str_col(row, 4),
        event_type: str_col(row, 5),
        text: str_col(row, 6),
        metadata_json: serde_json::from_str(&str_col(row, 7))?,
        created_at: str_col(row, 8),
    })
}

pub fn stable_id(prefix: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    format!("{prefix}-{}", &hex::encode(hasher.finalize())[..24])
}

fn run_create(db: &DbInstance, script: &str) -> Result<()> {
    match db.run_script(script, Default::default(), ScriptMutability::Mutable) {
        Ok(_) => Ok(()),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("already exists") || msg.contains("conflicts with an existing") {
                Ok(())
            } else {
                Err(MeaningError::Schema(msg))
            }
        }
    }
}

fn str_col(row: &[DataValue], idx: usize) -> String {
    row.get(idx)
        .and_then(DataValue::get_str)
        .unwrap_or("")
        .to_string()
}

fn non_empty(row: &[DataValue], idx: usize) -> Option<String> {
    let value = str_col(row, idx);
    (!value.is_empty()).then_some(value)
}

fn relation_missing(message: &str) -> bool {
    message.contains("Cannot find requested stored relation")
        || message.contains("not found")
        || message.contains("does not exist")
}
