use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::models::{ArtifactRecord, ProcessingJob, ProvenanceEdge};

use super::common::{edge_type_str, parse_edge_type};

pub fn insert_artifact(db: &DbInstance, art: &ArtifactRecord) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("aid".into(), DataValue::from(art.artifact_id.as_str()));
    params.insert("did".into(), DataValue::from(art.document_id.as_str()));
    params.insert("atype".into(), DataValue::from(art.artifact_type.as_str()));
    params.insert("hash".into(), DataValue::from(art.content_hash.as_str()));
    params.insert("cat".into(), DataValue::from(art.created_at.as_str()));
    params.insert(
        "prov".into(),
        DataValue::from(art.provenance_record_id.as_str()),
    );

    crate::cozo_retry::run_script_guarded(
        db,
        "?[artifact_id, document_id, artifact_type, content_hash, created_at, provenance_record_id] \
         <- [[$aid, $did, $atype, $hash, $cat, $prov]] \
         :put doc_artifacts { artifact_id => document_id, artifact_type, content_hash, created_at, provenance_record_id }",
        params,
        ScriptMutability::Mutable,
        "insert doc_artifacts",
    )
    .map_err(|e| anyhow::anyhow!("insert doc_artifacts failed: {e}"))?;
    Ok(())
}

pub fn list_artifacts_for_doc(db: &DbInstance, document_id: &str) -> Result<Vec<ArtifactRecord>> {
    let mut params = BTreeMap::new();
    params.insert("did".into(), DataValue::from(document_id));
    let result = db
        .run_script(
            "?[artifact_id, document_id, artifact_type, content_hash, created_at, provenance_record_id] \
             := *doc_artifacts{artifact_id, document_id, artifact_type, content_hash, created_at, provenance_record_id}, \
             document_id = $did",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list artifacts failed: {e}"))?;
    Ok(result
        .rows
        .iter()
        .map(|row| ArtifactRecord {
            artifact_id: row[0].get_str().unwrap_or("").to_string(),
            document_id: row[1].get_str().unwrap_or("").to_string(),
            artifact_type: row[2].get_str().unwrap_or("").to_string(),
            content_hash: row[3].get_str().unwrap_or("").to_string(),
            created_at: row[4].get_str().unwrap_or("").to_string(),
            provenance_record_id: row[5].get_str().unwrap_or("").to_string(),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// ProvenanceEdge
// ---------------------------------------------------------------------------

pub fn insert_provenance_edge(db: &DbInstance, edge: &ProvenanceEdge) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("eid".into(), DataValue::from(edge.edge_id.as_str()));
    params.insert(
        "from".into(),
        DataValue::from(edge.from_artifact_id.as_str()),
    );
    params.insert("to".into(), DataValue::from(edge.to_artifact_id.as_str()));
    params.insert(
        "etype".into(),
        DataValue::from(edge_type_str(&edge.edge_type)),
    );
    params.insert("cat".into(), DataValue::from(edge.created_at.as_str()));

    crate::cozo_retry::run_script_guarded(
        db,
        "?[edge_id, from_artifact_id, to_artifact_id, edge_type, created_at] \
         <- [[$eid, $from, $to, $etype, $cat]] \
         :put doc_provenance_edges { edge_id => from_artifact_id, to_artifact_id, edge_type, created_at }",
        params,
        ScriptMutability::Mutable,
        "insert doc_provenance_edges",
    )
    .map_err(|e| anyhow::anyhow!("insert provenance edge failed: {e}"))?;
    Ok(())
}

pub fn list_provenance_from(
    db: &DbInstance,
    from_artifact_id: &str,
) -> Result<Vec<ProvenanceEdge>> {
    let mut params = BTreeMap::new();
    params.insert("faid".into(), DataValue::from(from_artifact_id));

    let result = db
        .run_script(
            "?[edge_id, from_artifact_id, to_artifact_id, edge_type, created_at] \
             := *doc_provenance_edges{edge_id, from_artifact_id, to_artifact_id, edge_type, created_at}, \
             from_artifact_id = $faid",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list provenance edges failed: {e}"))?;

    Ok(result
        .rows
        .iter()
        .map(|row| ProvenanceEdge {
            edge_id: row[0].get_str().unwrap_or("").to_string(),
            from_artifact_id: row[1].get_str().unwrap_or("").to_string(),
            to_artifact_id: row[2].get_str().unwrap_or("").to_string(),
            edge_type: parse_edge_type(row[3].get_str().unwrap_or("")),
            created_at: row[4].get_str().unwrap_or("").to_string(),
        })
        .collect())
}

pub fn list_provenance_to(db: &DbInstance, to_artifact_id: &str) -> Result<Vec<ProvenanceEdge>> {
    let mut params = BTreeMap::new();
    params.insert("taid".into(), DataValue::from(to_artifact_id));

    let result = db
        .run_script(
            "?[edge_id, from_artifact_id, to_artifact_id, edge_type, created_at] \
             := *doc_provenance_edges{edge_id, from_artifact_id, to_artifact_id, edge_type, created_at}, \
             to_artifact_id = $taid",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list provenance to failed: {e}"))?;

    Ok(result
        .rows
        .iter()
        .map(|row| ProvenanceEdge {
            edge_id: row[0].get_str().unwrap_or("").to_string(),
            from_artifact_id: row[1].get_str().unwrap_or("").to_string(),
            to_artifact_id: row[2].get_str().unwrap_or("").to_string(),
            edge_type: parse_edge_type(row[3].get_str().unwrap_or("")),
            created_at: row[4].get_str().unwrap_or("").to_string(),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// ProcessingJob
// ---------------------------------------------------------------------------

pub fn insert_processing_job(db: &DbInstance, job: &ProcessingJob) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("jid".into(), DataValue::from(job.job_id.as_str()));
    params.insert("did".into(), DataValue::from(job.document_id.as_str()));
    params.insert("jtype".into(), DataValue::from(job.job_type.as_str()));
    params.insert("status".into(), DataValue::from(job.status.as_str()));
    params.insert("sat".into(), DataValue::from(job.started_at.as_str()));
    params.insert(
        "cat".into(),
        DataValue::from(job.completed_at.as_deref().unwrap_or("")),
    );
    params.insert(
        "err".into(),
        DataValue::from(job.error_message.as_deref().unwrap_or("")),
    );

    crate::cozo_retry::run_script_guarded(
        db,
        "?[job_id, document_id, job_type, status, started_at, completed_at, error_message] \
         <- [[$jid, $did, $jtype, $status, $sat, $cat, $err]] \
         :put doc_processing_jobs { job_id => document_id, job_type, status, started_at, completed_at, error_message }",
        params,
        ScriptMutability::Mutable,
        "insert doc_processing_jobs",
    )
    .map_err(|e| anyhow::anyhow!("insert processing job failed: {e}"))?;
    Ok(())
}

/// Look up a single artifact by id.
pub fn get_artifact(db: &DbInstance, artifact_id: &str) -> Result<Option<ArtifactRecord>> {
    let mut params = BTreeMap::new();
    params.insert("aid".into(), DataValue::from(artifact_id));
    let result = crate::cozo_retry::run_script_guarded(
        db,
        "?[artifact_id, document_id, artifact_type, content_hash, created_at, provenance_record_id] \
         := *doc_artifacts{artifact_id, document_id, artifact_type, content_hash, created_at, provenance_record_id}, \
         artifact_id = $aid",
        params,
        ScriptMutability::Immutable,
        "get doc_artifacts",
    )
    .map_err(|e| anyhow::anyhow!("get artifact failed: {e}"))?;
    if result.rows.is_empty() {
        return Ok(None);
    }
    let row = &result.rows[0];
    Ok(Some(ArtifactRecord {
        artifact_id: row[0].get_str().unwrap_or("").to_string(),
        document_id: row[1].get_str().unwrap_or("").to_string(),
        artifact_type: row[2].get_str().unwrap_or("").to_string(),
        content_hash: row[3].get_str().unwrap_or("").to_string(),
        created_at: row[4].get_str().unwrap_or("").to_string(),
        provenance_record_id: row[5].get_str().unwrap_or("").to_string(),
    }))
}

/// Point an existing artifact's `provenance_record_id` at a provenance record (closes the
/// previously-empty-string gap). `:update` touches only that column for the existing key.
pub fn set_artifact_provenance_record(
    db: &DbInstance,
    artifact_id: &str,
    record_id: &str,
) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("aid".into(), DataValue::from(artifact_id));
    params.insert("rid".into(), DataValue::from(record_id));
    crate::cozo_retry::run_script_guarded(
        db,
        "?[artifact_id, provenance_record_id] <- [[$aid, $rid]] \
         :update doc_artifacts { artifact_id => provenance_record_id }",
        params,
        ScriptMutability::Mutable,
        "update doc_artifacts provenance_record_id",
    )
    .map_err(|e| anyhow::anyhow!("update doc_artifacts provenance_record_id failed: {e}"))?;
    Ok(())
}
