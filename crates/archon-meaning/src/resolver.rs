use std::collections::BTreeMap;

use cozo::{DataValue, DbInstance, ScriptMutability, Vector};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{MeaningError, MeaningSample, Result, TripletRecord};

const FALLBACK_EMBEDDING_DIM: usize = 1536;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HydratedTriplet {
    pub triplet_id: String,
    pub workspace_id: String,
    pub anchor: Vec<f32>,
    pub positive: Vec<f32>,
    pub negative: Vec<f32>,
}

pub fn resolve_triplet_embeddings(
    db: &DbInstance,
    triplet: &TripletRecord,
) -> Result<Option<HydratedTriplet>> {
    crate::ensure_schema(db)?;
    let samples = crate::list_samples(db)?;
    let Some(anchor) = resolve_embedding(db, &samples, &triplet.anchor_artifact_id)? else {
        return Ok(None);
    };
    let Some(positive) = resolve_embedding(db, &samples, &triplet.positive_sample_id)? else {
        return Ok(None);
    };
    let Some(negative) = resolve_embedding(db, &samples, &triplet.negative_sample_id)? else {
        return Ok(None);
    };

    Ok(Some(HydratedTriplet {
        triplet_id: triplet.triplet_id.clone(),
        workspace_id: triplet.workspace_id.clone(),
        anchor,
        positive,
        negative,
    }))
}

pub fn list_hydrated_triplets(db: &DbInstance, limit: usize) -> Result<Vec<HydratedTriplet>> {
    let mut triplets = crate::list_triplets(db)?;
    triplets.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    if limit > 0 && triplets.len() > limit {
        let keep_from = triplets.len() - limit;
        triplets.drain(0..keep_from);
    }

    let mut hydrated = Vec::new();
    for triplet in triplets {
        if let Some(item) = resolve_triplet_embeddings(db, &triplet)? {
            hydrated.push(item);
        }
    }
    hydrated.sort_by(|left, right| left.triplet_id.cmp(&right.triplet_id));
    Ok(hydrated)
}

fn resolve_embedding(
    db: &DbInstance,
    samples: &[MeaningSample],
    id: &str,
) -> Result<Option<Vec<f32>>> {
    let Some(sample) = samples
        .iter()
        .find(|sample| sample.sample_id == id || sample.artifact_id == id)
    else {
        return Ok(None);
    };

    for candidate in [&sample.sample_id, &sample.artifact_id] {
        if let Some(embedding) = lookup_embedding(db, candidate)? {
            return Ok(Some(embedding));
        }
    }

    Ok(Some(text_embedding(&sample.text, FALLBACK_EMBEDDING_DIM)))
}

fn lookup_embedding(db: &DbInstance, chunk_id: &str) -> Result<Option<Vec<f32>>> {
    let mut params = BTreeMap::new();
    params.insert("cid".to_string(), DataValue::from(chunk_id));
    let result = db.run_script(
        "?[embedding] := *vec_text_chunks{chunk_id, embedding}, chunk_id = $cid :limit 1",
        params,
        ScriptMutability::Immutable,
    );
    let rows = match result {
        Ok(rows) => rows,
        Err(err) if relation_missing(&err.to_string()) => return Ok(None),
        Err(err) => {
            return Err(MeaningError::Store(format!(
                "lookup vec_text_chunks failed: {err}"
            )));
        }
    };
    Ok(rows
        .rows
        .first()
        .and_then(|row| row.first())
        .and_then(vector_value))
}

fn vector_value(value: &DataValue) -> Option<Vec<f32>> {
    match value {
        DataValue::Vec(Vector::F32(values)) => Some(values.to_vec()),
        DataValue::List(values) => Some(
            values
                .iter()
                .filter_map(|value| value.get_float().map(|value| value as f32))
                .collect(),
        ),
        _ => None,
    }
}

fn text_embedding(text: &str, dim: usize) -> Vec<f32> {
    let mut vector = vec![0.0; dim];
    for token in text
        .split(|ch: char| !ch.is_alphanumeric())
        .map(str::to_ascii_lowercase)
        .filter(|token| !token.is_empty())
    {
        let digest = Sha256::digest(token.as_bytes());
        let idx = ((digest[0] as usize) << 8 | digest[1] as usize) % dim;
        let sign = if digest[2] % 2 == 0 { 1.0 } else { -1.0 };
        vector[idx] += sign * (1.0 + (token.len() as f32).ln());
    }
    normalize(vector)
}

fn normalize(mut vector: Vec<f32>) -> Vec<f32> {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
}

fn relation_missing(message: &str) -> bool {
    message.contains("Cannot find requested stored relation")
        || message.contains("not found")
        || message.contains("does not exist")
}

#[cfg(test)]
mod tests {
    use cozo::Vector;
    use ndarray::Array1;

    use super::*;
    use crate::MeaningLabel;

    fn db() -> DbInstance {
        DbInstance::new("mem", "", Default::default()).unwrap()
    }

    fn sample(sample_id: &str, artifact_id: &str, label: MeaningLabel) -> MeaningSample {
        MeaningSample {
            sample_id: sample_id.into(),
            workspace_id: "workspace".into(),
            artifact_id: artifact_id.into(),
            label,
            source_event_id: format!("event-{sample_id}"),
            event_type: "UserAccepted".into(),
            text: format!("{sample_id} representative correction text"),
            metadata_json: serde_json::json!({}),
            created_at: "2026-05-07T00:00:00Z".into(),
        }
    }

    fn triplet(id: &str, anchor: &str, positive: &str, negative: &str) -> TripletRecord {
        TripletRecord {
            triplet_id: id.into(),
            workspace_id: "workspace".into(),
            anchor_artifact_id: anchor.into(),
            positive_sample_id: positive.into(),
            negative_sample_id: negative.into(),
            created_at: "2026-05-07T00:00:00Z".into(),
        }
    }

    fn seed_sample(db: &DbInstance, sample: &MeaningSample, vector: &[f32]) {
        crate::ensure_schema(db).unwrap();
        crate::insert_sample(db, sample).unwrap();
        ensure_vec_text_chunks(db, vector.len());
        insert_embedding(db, &sample.sample_id, vector);
        insert_embedding(db, &sample.artifact_id, vector);
    }

    fn seed_triplet(db: &DbInstance, triplet: &TripletRecord) {
        crate::insert_triplet(db, triplet).unwrap();
    }

    fn ensure_vec_text_chunks(db: &DbInstance, dim: usize) {
        let result = db.run_script(
            &format!(
                ":create vec_text_chunks {{ chunk_id: String => embedding: <F32; {dim}>, provider: String }}"
            ),
            Default::default(),
            ScriptMutability::Mutable,
        );
        if let Err(err) = result {
            let msg = err.to_string();
            if !msg.contains("already exists") && !msg.contains("conflicts") {
                panic!("create vec_text_chunks failed: {msg}");
            }
        }
    }

    fn insert_embedding(db: &DbInstance, chunk_id: &str, vector: &[f32]) {
        let mut params = BTreeMap::new();
        params.insert("cid".into(), DataValue::from(chunk_id));
        params.insert(
            "embedding".into(),
            DataValue::Vec(Vector::F32(Array1::from_vec(vector.to_vec()))),
        );
        params.insert("provider".into(), DataValue::from("test"));
        db.run_script(
            "?[chunk_id, embedding, provider] <- [[$cid, $embedding, $provider]] :put vec_text_chunks { chunk_id => embedding, provider }",
            params,
            ScriptMutability::Mutable,
        )
        .unwrap();
    }

    #[test]
    fn resolve_triplet_embeddings_returns_hydrated_triplet() {
        let db = db();
        seed_sample(
            &db,
            &sample("pos", "anchor-artifact", MeaningLabel::Positive),
            &[0.0, 0.0, 0.0],
        );
        seed_sample(
            &db,
            &sample("good", "good-artifact", MeaningLabel::Positive),
            &[0.1, 0.0, 0.0],
        );
        seed_sample(
            &db,
            &sample("bad", "bad-artifact", MeaningLabel::Negative),
            &[0.9, 0.0, 0.0],
        );

        let hydrated =
            resolve_triplet_embeddings(&db, &triplet("t1", "anchor-artifact", "good", "bad"))
                .unwrap()
                .unwrap();

        assert_eq!(hydrated.triplet_id, "t1");
        assert_eq!(hydrated.anchor.len(), 3);
        assert_eq!(hydrated.positive, vec![0.1, 0.0, 0.0]);
        assert_eq!(hydrated.negative, vec![0.9, 0.0, 0.0]);
    }

    #[test]
    fn resolve_triplet_embeddings_returns_none_for_missing_artifact() {
        let db = db();
        seed_sample(
            &db,
            &sample("good", "good-artifact", MeaningLabel::Positive),
            &[0.1, 0.0, 0.0],
        );
        seed_sample(
            &db,
            &sample("bad", "bad-artifact", MeaningLabel::Negative),
            &[0.9, 0.0, 0.0],
        );

        let hydrated =
            resolve_triplet_embeddings(&db, &triplet("t1", "missing-artifact", "good", "bad"))
                .unwrap();

        assert!(hydrated.is_none());
    }

    #[test]
    fn list_hydrated_triplets_skips_unresolvable() {
        let db = db();
        seed_sample(
            &db,
            &sample("anchor", "anchor-artifact", MeaningLabel::Positive),
            &[0.0, 0.0, 0.0],
        );
        seed_sample(
            &db,
            &sample("good", "good-artifact", MeaningLabel::Positive),
            &[0.1, 0.0, 0.0],
        );
        seed_sample(
            &db,
            &sample("bad", "bad-artifact", MeaningLabel::Negative),
            &[0.9, 0.0, 0.0],
        );
        seed_triplet(&db, &triplet("t1", "anchor-artifact", "good", "bad"));
        seed_triplet(&db, &triplet("t2", "anchor", "good", "bad"));
        seed_triplet(&db, &triplet("t3", "missing-artifact", "good", "bad"));

        let hydrated = list_hydrated_triplets(&db, 10).unwrap();

        assert_eq!(hydrated.len(), 2);
        assert_eq!(
            hydrated
                .iter()
                .map(|triplet| triplet.triplet_id.as_str())
                .collect::<Vec<_>>(),
            vec!["t1", "t2"]
        );
    }
}
