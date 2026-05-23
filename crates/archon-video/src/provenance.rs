use anyhow::Result;
use archon_docs::models::ProvenanceEdgeType;
use cozo::DbInstance;

pub fn insert_edge(
    db: &DbInstance,
    from_artifact_id: &str,
    to_artifact_id: &str,
    edge_type: ProvenanceEdgeType,
) -> Result<()> {
    let edge = archon_docs::provenance::make_edge(from_artifact_id, to_artifact_id, edge_type);
    archon_docs::store::insert_provenance_edge(db, &edge)?;
    Ok(())
}
