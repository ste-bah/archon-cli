//! Provenance edge creation and chain hashing.
//!
//! Chain hash rule (TSPEC §4):
//!   chain_hash = SHA256(parent_hashes | operation | input_hashes | output_hash
//!                        | tool | model | parameters_json)

use sha2::{Digest, Sha256};

use crate::models::{ProvenanceEdge, ProvenanceEdgeType};

/// Create a provenance link between two artifacts.
pub fn make_edge(
    from_artifact_id: &str,
    to_artifact_id: &str,
    edge_type: ProvenanceEdgeType,
) -> ProvenanceEdge {
    ProvenanceEdge {
        edge_id: format!("edge-{}", uuid::Uuid::new_v4()),
        from_artifact_id: from_artifact_id.to_string(),
        to_artifact_id: to_artifact_id.to_string(),
        edge_type,
        created_at: chrono::Utc::now().to_rfc3339(),
    }
}

/// Compute a chain hash per the TSPEC rule.
///
/// ```text
/// chain_hash = SHA256(parent_hashes | operation | input_hashes | output_hash
///                      | tool | model | parameters_json)
/// ```
pub fn chain_hash(
    parent_hashes: &[String],
    operation: &str,
    input_hashes: &[String],
    output_hash: &str,
    tool: Option<&str>,
    model: Option<&str>,
    parameters_json: &str,
) -> String {
    let mut hasher = Sha256::new();

    for ph in parent_hashes {
        hasher.update(ph.as_bytes());
    }
    hasher.update(operation.as_bytes());
    for ih in input_hashes {
        hasher.update(ih.as_bytes());
    }
    hasher.update(output_hash.as_bytes());
    hasher.update(tool.unwrap_or(""));
    hasher.update(model.unwrap_or(""));
    hasher.update(parameters_json.as_bytes());

    hex::encode(hasher.finalize())
}

/// Build a provenance chain for document → page → chunk.
/// Returns edges linking chunk → page → document.
pub fn build_doc_lineage_edges(
    document_id: &str,
    artifact_id: &str,
    chunks: &[crate::models::ChunkArtifact],
    page_ids: &[String],
) -> Vec<ProvenanceEdge> {
    let mut edges = Vec::new();

    // chunk -> page edges: each chunk links to every page in its span
    for chunk in chunks {
        for page_num in chunk.page_start..=chunk.page_end {
            // Page IDs are 1-indexed: "page-{doc_id}-{n}"
            let target_page_id = format!("page-{}-{}", document_id, page_num);
            if page_ids.contains(&target_page_id) {
                edges.push(make_edge(
                    &chunk.chunk_id,
                    &target_page_id,
                    ProvenanceEdgeType::ExtractedFrom,
                ));
            }
        }
    }

    // artifact -> document edge
    edges.push(make_edge(
        artifact_id,
        document_id,
        ProvenanceEdgeType::DerivedFrom,
    ));

    edges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chain_hash_deterministic() {
        let a = chain_hash(
            &["parent1".into()],
            "ocr",
            &["input1".into()],
            "output1",
            Some("pdftotext"),
            None,
            "{}",
        );
        let b = chain_hash(
            &["parent1".into()],
            "ocr",
            &["input1".into()],
            "output1",
            Some("pdftotext"),
            None,
            "{}",
        );
        assert_eq!(a, b);
    }

    #[test]
    fn test_chain_hash_different_inputs() {
        let a = chain_hash(
            &["parent1".into()],
            "ocr",
            &["input1".into()],
            "output1",
            None,
            None,
            "{}",
        );
        let b = chain_hash(
            &["parent2".into()],
            "ocr",
            &["input1".into()],
            "output1",
            None,
            None,
            "{}",
        );
        assert_ne!(a, b);
    }

    #[test]
    fn test_make_edge_has_required_fields() {
        let edge = make_edge("from-1", "to-1", ProvenanceEdgeType::DerivedFrom);
        assert!(edge.edge_id.starts_with("edge-"));
        assert_eq!(edge.from_artifact_id, "from-1");
        assert_eq!(edge.to_artifact_id, "to-1");
        assert!(!edge.created_at.is_empty());
    }

    #[test]
    fn test_build_doc_lineage_edges() {
        use crate::models::ChunkArtifact;

        let chunks = vec![
            ChunkArtifact {
                chunk_id: "chunk-0".into(),
                document_id: "doc-1".into(),
                artifact_id: "art-1".into(),
                chunk_index: 0,
                page_start: 1,
                page_end: 1,
                content: String::new(),
                content_hash: String::new(),
                embedding_status: "pending".into(),
            },
            ChunkArtifact {
                chunk_id: "chunk-1".into(),
                document_id: "doc-1".into(),
                artifact_id: "art-1".into(),
                chunk_index: 1,
                page_start: 2,
                page_end: 2,
                content: String::new(),
                content_hash: String::new(),
                embedding_status: "pending".into(),
            },
        ];

        let edges = build_doc_lineage_edges(
            "doc-1",
            "art-1",
            &chunks,
            &["page-doc-1-1".into(), "page-doc-1-2".into()],
        );

        // 2 chunk->page edges + 1 artifact->document edge = 3
        assert_eq!(edges.len(), 3);
        // First edge: chunk-0 -> page-doc-1-1
        assert_eq!(edges[0].from_artifact_id, "chunk-0");
        assert_eq!(edges[0].to_artifact_id, "page-doc-1-1");
        // Last edge: artifact -> document
        let last = edges.last().unwrap();
        assert_eq!(last.from_artifact_id, "art-1");
        assert_eq!(last.to_artifact_id, "doc-1");
    }
}
