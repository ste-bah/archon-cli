use std::collections::BTreeMap;

use cozo::{DataValue, DbInstance, ScriptMutability, Vector};
use ndarray::Array1;
use serde::{Deserialize, Serialize};

use crate::errors::{ConstellationError, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConstellationCentroid {
    pub centroid_id: String,
    pub target: String,
    pub version: u32,
    pub vector: Vec<f32>,
    pub sample_ids: Vec<String>,
    pub sample_count: usize,
    pub source_relation: String,
    pub created_at: String,
}

pub fn insert_centroid(db: &DbInstance, centroid: &ConstellationCentroid) -> Result<()> {
    let vector_json = serde_json::to_string(&centroid.vector)?;
    let sample_ids_json = serde_json::to_string(&centroid.sample_ids)?;
    let mut params = BTreeMap::new();
    params.insert("id".into(), DataValue::from(centroid.centroid_id.as_str()));
    params.insert("target".into(), DataValue::from(centroid.target.as_str()));
    params.insert("version".into(), DataValue::from(centroid.version as i64));
    params.insert("vector".into(), DataValue::from(vector_json.as_str()));
    params.insert(
        "sample_ids".into(),
        DataValue::from(sample_ids_json.as_str()),
    );
    params.insert(
        "count".into(),
        DataValue::from(centroid.sample_count as i64),
    );
    params.insert(
        "source".into(),
        DataValue::from(centroid.source_relation.as_str()),
    );
    params.insert("ts".into(), DataValue::from(centroid.created_at.as_str()));
    db.run_script(
        "?[centroid_id, target, version, vector_json, sample_ids_json, sample_count, source_relation, created_at] <- \
         [[$id, $target, $version, $vector, $sample_ids, $count, $source, $ts]] \
         :put constellation_centroids { centroid_id => target, version, vector_json, sample_ids_json, sample_count, source_relation, created_at }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| ConstellationError::Store(format!("insert centroid failed: {e}")))?;
    Ok(())
}

pub fn insert_vector(db: &DbInstance, centroid_id: &str, vector: &[f32]) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("id".into(), DataValue::from(centroid_id));
    params.insert(
        "embedding".into(),
        DataValue::Vec(Vector::F32(Array1::from_vec(vector.to_vec()))),
    );
    params.insert(
        "provider".into(),
        DataValue::from("archon-hash-centroid-v1"),
    );
    db.run_script(
        "?[centroid_id, embedding, provider] <- [[$id, $embedding, $provider]] \
         :put vec_constellations { centroid_id => embedding, provider }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| ConstellationError::Store(format!("insert centroid vector failed: {e}")))?;
    Ok(())
}

pub fn list_centroids(db: &DbInstance) -> Result<Vec<ConstellationCentroid>> {
    let result = db
        .run_script(
            "?[centroid_id, target, version, vector_json, sample_ids_json, sample_count, source_relation, created_at] := \
             *constellation_centroids{centroid_id, target, version, vector_json, sample_ids_json, sample_count, source_relation, created_at}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| ConstellationError::Store(format!("list centroids failed: {e}")))?;
    result.rows.iter().map(|row| row_to_centroid(row)).collect()
}

pub fn latest_centroid(db: &DbInstance, target: &str) -> Result<Option<ConstellationCentroid>> {
    let mut centroids = list_centroids(db)?;
    centroids.retain(|centroid| centroid.target == target);
    centroids.sort_by_key(|centroid| centroid.version);
    Ok(centroids.pop())
}

pub fn next_version(db: &DbInstance, target: &str) -> Result<u32> {
    let latest = latest_centroid(db, target)?;
    Ok(latest.map(|centroid| centroid.version + 1).unwrap_or(1))
}

pub fn count_vectors(db: &DbInstance) -> Result<usize> {
    let result = db
        .run_script(
            "?[count(centroid_id)] := *vec_constellations{centroid_id}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| ConstellationError::Store(format!("count vec_constellations failed: {e}")))?;
    Ok(result
        .rows
        .first()
        .and_then(|row| row.first())
        .and_then(DataValue::get_int)
        .unwrap_or(0) as usize)
}

fn row_to_centroid(row: &[DataValue]) -> Result<ConstellationCentroid> {
    Ok(ConstellationCentroid {
        centroid_id: str_col(row, 0),
        target: str_col(row, 1),
        version: int_col(row, 2) as u32,
        vector: serde_json::from_str(&str_col(row, 3))?,
        sample_ids: serde_json::from_str(&str_col(row, 4))?,
        sample_count: int_col(row, 5) as usize,
        source_relation: str_col(row, 6),
        created_at: str_col(row, 7),
    })
}

fn str_col(row: &[DataValue], idx: usize) -> String {
    row.get(idx)
        .and_then(DataValue::get_str)
        .unwrap_or_default()
        .to_string()
}

fn int_col(row: &[DataValue], idx: usize) -> i64 {
    row.get(idx).and_then(DataValue::get_int).unwrap_or(0)
}
