//! Provenance CLI handler.

use std::path::PathBuf;

use anyhow::Result;
use cozo::DbInstance;

use crate::cli_args::ProvAction;

fn prov_db_path() -> PathBuf {
    crate::command::store_paths::evidence_db_path(&["ARCHON_PROV_DB_PATH", "ARCHON_KB_DB_PATH"])
}

fn open_db() -> Result<DbInstance> {
    let db_path = prov_db_path();
    let db = crate::command::store_paths::open_sqlite_db(&db_path, "provenance")?;
    archon_provenance::store::ensure_schema(&db)?;
    Ok(db)
}

pub async fn handle_prov_command(action: ProvAction) -> Result<()> {
    let db = open_db()?;
    match action {
        ProvAction::Trace { artifact_id } => trace(&db, &artifact_id),
        ProvAction::Export { artifact_id } => export(&db, &artifact_id),
        ProvAction::Verify { artifact_id } => verify(&db, &artifact_id),
    }
}

fn trace(db: &DbInstance, artifact_id: &str) -> Result<()> {
    let trace = archon_provenance::traverse::trace_artifact(db, artifact_id)?;
    println!("Trace: {}", trace.start_artifact_id);
    println!("Nodes: {}", trace.nodes.len());
    for node in &trace.nodes {
        println!(
            "  node {}  type={}  hash={}",
            node.artifact_id,
            node.artifact_type,
            node.content_hash.as_deref().unwrap_or("-")
        );
    }
    println!("Edges: {}", trace.edges.len());
    for edge in &trace.edges {
        println!(
            "  edge {}  {} -> {}  {}",
            edge.edge_id,
            edge.from_artifact_id,
            edge.to_artifact_id,
            edge.edge_type.as_str()
        );
    }
    println!("Reaches source: {}", trace.reaches_source());
    Ok(())
}

fn export(db: &DbInstance, artifact_id: &str) -> Result<()> {
    let trace = archon_provenance::traverse::trace_artifact(db, artifact_id)?;
    let json = archon_provenance::export_w3c::export_trace_jsonld(&trace);
    println!("{}", serde_json::to_string_pretty(&json)?);
    Ok(())
}

fn verify(db: &DbInstance, artifact_id: &str) -> Result<()> {
    let report = archon_provenance::verify::verify_artifact(db, artifact_id)?;
    println!("Artifact: {}", report.artifact_id);
    println!("Valid: {}", report.valid);
    println!("Reaches source: {}", report.reaches_source);
    if let Some(record_id) = &report.chain_record_id {
        println!("Chain record: {record_id}");
        println!("Chain valid: {}", report.chain_valid.unwrap_or(false));
    }
    println!("Nodes: {}", report.node_count);
    println!("Edges: {}", report.edge_count);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prov_db_path_prefers_explicit_override() {
        unsafe {
            std::env::set_var("ARCHON_PROV_DB_PATH", "/tmp/archon-prov-test.db");
        }
        assert_eq!(prov_db_path(), PathBuf::from("/tmp/archon-prov-test.db"));
        unsafe {
            std::env::remove_var("ARCHON_PROV_DB_PATH");
        }
    }
}
