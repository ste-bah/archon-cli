use cozo::DbInstance;
use serde::{Deserialize, Serialize};

use crate::chain;
use crate::errors::{ProvenanceError, Result};
use crate::{store, traverse};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChainVerification {
    pub record_id: String,
    pub artifact_id: String,
    pub valid: bool,
    pub stored_chain_hash: String,
    pub expected_chain_hash: String,
    pub missing_parent_record_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactVerification {
    pub artifact_id: String,
    pub valid: bool,
    pub reaches_source: bool,
    pub chain_record_id: Option<String>,
    pub chain_valid: Option<bool>,
    pub edge_count: usize,
    pub node_count: usize,
}

pub fn verify_record_chain(db: &DbInstance, record_id: &str) -> Result<ChainVerification> {
    let record = store::get_record(db, record_id)?
        .ok_or_else(|| ProvenanceError::NotFound(format!("record {record_id}")))?;
    let mut parent_hashes = Vec::new();
    let mut missing = Vec::new();
    for parent_id in &record.parent_record_ids {
        if let Some(parent) = store::get_record(db, parent_id)? {
            parent_hashes.push(parent.chain_hash);
        } else {
            missing.push(parent_id.clone());
        }
    }
    let expected = chain::chain_hash(
        &parent_hashes,
        &record.operation,
        &record.input_hashes,
        &record.output_hash,
        record.tool_name.as_deref(),
        record.model.as_deref(),
        &record.parameters_json,
    );
    Ok(ChainVerification {
        record_id: record.record_id,
        artifact_id: record.artifact_id,
        valid: missing.is_empty() && expected == record.chain_hash,
        stored_chain_hash: record.chain_hash,
        expected_chain_hash: expected,
        missing_parent_record_ids: missing,
    })
}

pub fn verify_artifact(db: &DbInstance, artifact_id: &str) -> Result<ArtifactVerification> {
    let trace = traverse::trace_artifact(db, artifact_id)?;
    let chain = if let Some(record) = store::get_record_by_artifact(db, artifact_id)? {
        Some(verify_record_chain(db, &record.record_id)?)
    } else {
        None
    };
    let chain_valid = chain.as_ref().map(|report| report.valid);
    Ok(ArtifactVerification {
        artifact_id: artifact_id.to_string(),
        valid: !trace.edges.is_empty() && trace.reaches_source() && chain_valid.unwrap_or(true),
        reaches_source: trace.reaches_source(),
        chain_record_id: chain.as_ref().map(|report| report.record_id.clone()),
        chain_valid,
        edge_count: trace.edges.len(),
        node_count: trace.nodes.len(),
    })
}
