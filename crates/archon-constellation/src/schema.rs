use cozo::{DbInstance, ScriptMutability};

use crate::errors::{ConstellationError, Result};

pub const VECTOR_DIM: usize = 64;

pub fn ensure_schema(db: &DbInstance) -> Result<()> {
    for script in [
        r#":create constellation_centroids {
            centroid_id: String => target: String, version: Int,
            vector_json: String, sample_ids_json: String, sample_count: Int,
            source_relation: String, created_at: String
        }"#,
        &format!(
            ":create vec_constellations {{
                centroid_id: String => embedding: <F32; {VECTOR_DIM}>,
                provider: String
            }}"
        ),
        &format!(
            "::hnsw create vec_constellations:constellation_embedding_idx {{
                dim: {VECTOR_DIM},
                m: 32,
                dtype: F32,
                fields: [embedding],
                distance: Cosine,
                ef_construction: 128
            }}"
        ),
    ] {
        run_create(db, script)?;
    }
    Ok(())
}

fn run_create(db: &DbInstance, script: &str) -> Result<()> {
    match db.run_script(script, Default::default(), ScriptMutability::Mutable) {
        Ok(_) => Ok(()),
        Err(e) => {
            let msg = e.to_string();
            if is_already_exists(&msg) {
                Ok(())
            } else {
                Err(ConstellationError::Schema(format!(
                    "schema creation failed: {msg}"
                )))
            }
        }
    }
}

fn is_already_exists(msg: &str) -> bool {
    [
        "already exists",
        "conflicts with an existing",
        "name conflict",
        "index exists",
    ]
    .iter()
    .any(|phrase| msg.contains(phrase))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_schema_is_idempotent() {
        let db = DbInstance::new("mem", "", Default::default()).unwrap();
        ensure_schema(&db).unwrap();
        ensure_schema(&db).unwrap();
    }

    #[test]
    fn vector_dimension_matches_prd_relation() {
        assert_eq!(VECTOR_DIM, 64);
    }
}
