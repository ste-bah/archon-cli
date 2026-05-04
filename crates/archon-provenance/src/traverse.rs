use std::collections::{BTreeSet, VecDeque};

use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

use crate::errors::{ProvenanceError, Result};
use crate::record::{ProvenanceEdge, ProvenanceEdgeType};
use crate::{chain, store};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceNode {
    pub artifact_id: String,
    pub artifact_type: String,
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceTrace {
    pub start_artifact_id: String,
    pub nodes: Vec<TraceNode>,
    pub edges: Vec<ProvenanceEdge>,
}

impl ProvenanceTrace {
    pub fn reaches_source(&self) -> bool {
        self.nodes
            .iter()
            .any(|node| node.artifact_type == "source_document")
    }
}

pub fn trace_artifact(db: &DbInstance, artifact_id: &str) -> Result<ProvenanceTrace> {
    trace_artifact_with_limit(db, artifact_id, 16)
}

pub fn trace_artifact_with_limit(
    db: &DbInstance,
    artifact_id: &str,
    max_depth: usize,
) -> Result<ProvenanceTrace> {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut seen_nodes = BTreeSet::new();
    let mut seen_edges = BTreeSet::new();
    let mut queue = VecDeque::from([(artifact_id.to_string(), 0_usize)]);

    while let Some((current, depth)) = queue.pop_front() {
        if !seen_nodes.insert(current.clone()) {
            continue;
        }
        nodes.push(describe_node(db, &current)?);
        if depth >= max_depth {
            continue;
        }
        for edge in outgoing_edges(db, &current)? {
            let edge_key = format!(
                "{}\0{}\0{}",
                edge.from_artifact_id,
                edge.to_artifact_id,
                edge.edge_type.as_str()
            );
            if seen_edges.insert(edge_key) {
                queue.push_back((edge.to_artifact_id.clone(), depth + 1));
                edges.push(edge);
            }
        }
    }

    Ok(ProvenanceTrace {
        start_artifact_id: artifact_id.to_string(),
        nodes,
        edges,
    })
}

fn outgoing_edges(db: &DbInstance, artifact_id: &str) -> Result<Vec<ProvenanceEdge>> {
    let mut edges = store::list_edges_from(db, artifact_id)?;
    edges.extend(query_relation_edges(
        db,
        "doc_provenance_edges",
        artifact_id,
    )?);
    edges.extend(query_relation_edges(
        db,
        "gt_provenance_edges",
        artifact_id,
    )?);
    edges.extend(synthetic_doc_edges(db, artifact_id)?);
    Ok(edges)
}

fn query_relation_edges(
    db: &DbInstance,
    relation: &str,
    artifact_id: &str,
) -> Result<Vec<ProvenanceEdge>> {
    let mut params = std::collections::BTreeMap::new();
    params.insert("artifact".into(), DataValue::from(artifact_id));
    let script = format!(
        "?[edge_id, from_artifact_id, to_artifact_id, edge_type, created_at] := \
         *{relation}{{edge_id, from_artifact_id, to_artifact_id, edge_type, created_at}}, \
         from_artifact_id = $artifact"
    );
    match db.run_script(&script, params, ScriptMutability::Immutable) {
        Ok(result) => Ok(result
            .rows
            .iter()
            .map(|row| store::row_to_edge(row))
            .collect()),
        Err(e) if store::relation_missing(&e.to_string()) => Ok(Vec::new()),
        Err(e) => Err(ProvenanceError::Store(format!(
            "query {relation} provenance failed: {e}"
        ))),
    }
}

fn synthetic_doc_edges(db: &DbInstance, artifact_id: &str) -> Result<Vec<ProvenanceEdge>> {
    let mut edges = Vec::new();
    edges.extend(chunk_to_page_edges(db, artifact_id)?);
    edges.extend(page_to_document_edge(db, artifact_id)?);
    edges.extend(artifact_to_document_edge(db, artifact_id)?);
    Ok(edges)
}

fn chunk_to_page_edges(db: &DbInstance, chunk_id: &str) -> Result<Vec<ProvenanceEdge>> {
    let mut params = std::collections::BTreeMap::new();
    params.insert("cid".into(), DataValue::from(chunk_id));
    let result = db.run_script(
        "?[document_id, page_start, page_end] := *doc_chunks{chunk_id, document_id, page_start, page_end}, chunk_id = $cid",
        params,
        ScriptMutability::Immutable,
    );
    let rows = match result {
        Ok(rows) => rows,
        Err(e) if store::relation_missing(&e.to_string()) => return Ok(Vec::new()),
        Err(e) => {
            return Err(ProvenanceError::Store(format!(
                "query doc_chunks failed: {e}"
            )));
        }
    };
    let mut edges = Vec::new();
    for row in &rows.rows {
        let document_id = str_col(row, 0);
        let start = int_col(row, 1).max(1) as u32;
        let end = int_col(row, 2).max(start as i64) as u32;
        for page in start..=end {
            let page_id = format!("page-{document_id}-{page}");
            edges.push(synthetic_edge(
                chunk_id,
                &page_id,
                ProvenanceEdgeType::ExtractedFrom,
            ));
        }
    }
    Ok(edges)
}

fn page_to_document_edge(db: &DbInstance, page_id: &str) -> Result<Vec<ProvenanceEdge>> {
    let mut params = std::collections::BTreeMap::new();
    params.insert("pid".into(), DataValue::from(page_id));
    let result = db.run_script(
        "?[document_id] := *doc_pages{page_id, document_id}, page_id = $pid",
        params,
        ScriptMutability::Immutable,
    );
    match result {
        Ok(rows) => Ok(rows
            .rows
            .iter()
            .map(|row| synthetic_edge(page_id, &str_col(row, 0), ProvenanceEdgeType::ExtractedFrom))
            .collect()),
        Err(e) if store::relation_missing(&e.to_string()) => Ok(Vec::new()),
        Err(e) => Err(ProvenanceError::Store(format!(
            "query doc_pages failed: {e}"
        ))),
    }
}

fn artifact_to_document_edge(db: &DbInstance, artifact_id: &str) -> Result<Vec<ProvenanceEdge>> {
    let mut params = std::collections::BTreeMap::new();
    params.insert("aid".into(), DataValue::from(artifact_id));
    let result = db.run_script(
        "?[document_id] := *doc_artifacts{artifact_id, document_id}, artifact_id = $aid",
        params,
        ScriptMutability::Immutable,
    );
    match result {
        Ok(rows) => Ok(rows
            .rows
            .iter()
            .map(|row| {
                synthetic_edge(
                    artifact_id,
                    &str_col(row, 0),
                    ProvenanceEdgeType::DerivedFrom,
                )
            })
            .collect()),
        Err(e) if store::relation_missing(&e.to_string()) => Ok(Vec::new()),
        Err(e) => Err(ProvenanceError::Store(format!(
            "query doc_artifacts failed: {e}"
        ))),
    }
}

fn synthetic_edge(from: &str, to: &str, edge_type: ProvenanceEdgeType) -> ProvenanceEdge {
    let hash = chain::chain_hash_from_str(
        &[],
        "synthetic-edge",
        &[from.to_string(), to.to_string()],
        edge_type.as_str(),
        None,
        None,
        "{}",
    );
    ProvenanceEdge {
        edge_id: format!("edge-synth-{}", &hash[..16]),
        from_artifact_id: from.to_string(),
        to_artifact_id: to.to_string(),
        edge_type,
        created_at: "synthetic".into(),
    }
}

fn describe_node(db: &DbInstance, artifact_id: &str) -> Result<TraceNode> {
    if let Some((artifact_type, hash)) = lookup_doc_source(db, artifact_id)? {
        return Ok(TraceNode {
            artifact_id: artifact_id.to_string(),
            artifact_type,
            content_hash: hash,
        });
    }
    if let Some((artifact_type, hash)) = lookup_doc_artifact(db, artifact_id)? {
        return Ok(TraceNode {
            artifact_id: artifact_id.to_string(),
            artifact_type,
            content_hash: hash,
        });
    }
    if lookup_exists(db, "doc_pages", "page_id", artifact_id)? {
        return Ok(TraceNode {
            artifact_id: artifact_id.to_string(),
            artifact_type: "page".into(),
            content_hash: None,
        });
    }
    if let Some(hash) = lookup_chunk_hash(db, artifact_id)? {
        return Ok(TraceNode {
            artifact_id: artifact_id.to_string(),
            artifact_type: "chunk".into(),
            content_hash: Some(hash),
        });
    }
    Ok(TraceNode {
        artifact_id: artifact_id.to_string(),
        artifact_type: "artifact".into(),
        content_hash: None,
    })
}

fn lookup_doc_source(
    db: &DbInstance,
    document_id: &str,
) -> Result<Option<(String, Option<String>)>> {
    let rows = query_two_cols(
        db,
        "?[media_type, content_hash] := *doc_sources{document_id, media_type, content_hash}, document_id = $id",
        document_id,
    )?;
    Ok(rows.first().map(|row| {
        (
            "source_document".to_string(),
            row.get(1).and_then(DataValue::get_str).map(str::to_string),
        )
    }))
}

fn lookup_doc_artifact(
    db: &DbInstance,
    artifact_id: &str,
) -> Result<Option<(String, Option<String>)>> {
    let rows = query_two_cols(
        db,
        "?[artifact_type, content_hash] := *doc_artifacts{artifact_id, artifact_type, content_hash}, artifact_id = $id",
        artifact_id,
    )?;
    Ok(rows.first().map(|row| {
        (
            str_col(row, 0),
            row.get(1).and_then(DataValue::get_str).map(str::to_string),
        )
    }))
}

fn lookup_chunk_hash(db: &DbInstance, chunk_id: &str) -> Result<Option<String>> {
    let rows = query_two_cols(
        db,
        "?[content_hash, document_id] := *doc_chunks{chunk_id, content_hash, document_id}, chunk_id = $id",
        chunk_id,
    )?;
    Ok(rows
        .first()
        .and_then(|row| row.first().and_then(DataValue::get_str).map(str::to_string)))
}

fn lookup_exists(db: &DbInstance, relation: &str, key: &str, value: &str) -> Result<bool> {
    let mut params = std::collections::BTreeMap::new();
    params.insert("id".into(), DataValue::from(value));
    let script = format!("?[{key}] := *{relation}{{{key}}}, {key} = $id");
    match db.run_script(&script, params, ScriptMutability::Immutable) {
        Ok(rows) => Ok(!rows.rows.is_empty()),
        Err(e) if store::relation_missing(&e.to_string()) => Ok(false),
        Err(e) => Err(ProvenanceError::Store(format!(
            "lookup {relation}.{key} failed: {e}"
        ))),
    }
}

fn query_two_cols(db: &DbInstance, script: &str, value: &str) -> Result<Vec<Vec<DataValue>>> {
    let mut params = std::collections::BTreeMap::new();
    params.insert("id".into(), DataValue::from(value));
    match db.run_script(script, params, ScriptMutability::Immutable) {
        Ok(rows) => Ok(rows.rows),
        Err(e) if store::relation_missing(&e.to_string()) => Ok(Vec::new()),
        Err(e) => Err(ProvenanceError::Store(format!("node lookup failed: {e}"))),
    }
}

fn str_col(row: &[DataValue], idx: usize) -> String {
    row.get(idx)
        .and_then(DataValue::get_str)
        .unwrap_or("")
        .to_string()
}

fn int_col(row: &[DataValue], idx: usize) -> i64 {
    row.get(idx).and_then(DataValue::get_int).unwrap_or(0)
}
