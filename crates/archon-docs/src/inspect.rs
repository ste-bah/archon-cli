//! Inspection output formatting for document state.
//!
//! Provides human-readable views of stored documents, chunks,
//! provenance edges, and processing jobs without manual Cozo queries.

use std::collections::HashSet;

use anyhow::Result;
use cozo::DbInstance;

use crate::models::SourceDocument;
use crate::store;

/// Full inspection output for a document.
#[derive(Clone, Debug)]
pub struct DocInspectOutput {
    pub document: SourceDocument,
    pub chunks: Vec<crate::models::ChunkArtifact>,
    pub pages: Vec<crate::models::PageArtifact>,
    pub ocr_runs: Vec<crate::models::OcrRun>,
    pub provenance_edges: Vec<crate::models::ProvenanceEdge>,
}

/// Inspect a single document by ID.
pub fn inspect_document(db: &DbInstance, document_id: &str) -> Result<DocInspectOutput> {
    let document = store::get_doc_source(db, document_id)?
        .ok_or_else(|| anyhow::anyhow!("Document not found: {}", document_id))?;

    let chunks = store::list_chunks_for_doc(db, document_id)?;
    let pages = store::list_pages_for_doc(db, document_id)?;
    let ocr_runs = store::list_ocr_runs_for_doc(db, document_id)?;

    // Collect all edges connected to this document's sub-artifacts:
    // edges from document_id, from any chunk_id, from any page_id,
    // from any ocr-result artifact, or TO document_id.
    let mut seen = HashSet::new();
    let mut provenance_edges = Vec::new();
    let mut push_unique = |edge: crate::models::ProvenanceEdge| {
        if seen.insert(edge.edge_id.clone()) {
            provenance_edges.push(edge);
        }
    };
    for edge in store::list_provenance_from(db, document_id)? {
        push_unique(edge);
    }
    for edge in store::list_provenance_to(db, document_id)? {
        push_unique(edge);
    }
    for chunk in &chunks {
        for edge in store::list_provenance_from(db, &chunk.chunk_id)? {
            push_unique(edge);
        }
    }
    for page in &pages {
        for edge in store::list_provenance_from(db, &page.page_id)? {
            push_unique(edge);
        }
    }
    for run in &ocr_runs {
        let artifact_id = format!("ocr-result-{}", run.ocr_run_id);
        for edge in store::list_provenance_from(db, &artifact_id)? {
            push_unique(edge);
        }
    }

    Ok(DocInspectOutput {
        document,
        chunks,
        pages,
        ocr_runs,
        provenance_edges,
    })
}

/// Format a DocInspectOutput for CLI display.
pub fn format_inspect_output(output: &DocInspectOutput) -> String {
    let mut lines = Vec::new();

    lines.push("=== Document ===".to_string());
    lines.push(format!("  ID:           {}", output.document.document_id));
    lines.push(format!("  Path:         {}", output.document.source_path));
    lines.push(format!("  Media Type:   {}", output.document.media_type));
    lines.push(format!("  Content Hash: {}", output.document.content_hash));
    lines.push(format!(
        "  Status:       {:?}",
        output.document.status
    ));
    lines.push(format!(
        "  Discovered:   {}",
        output.document.discovered_at
    ));

    if !output.pages.is_empty() {
        lines.push(String::new());
        lines.push(format!("=== Pages ({} total) ===", output.pages.len()));
        for page in &output.pages {
            lines.push(format!(
                "  Page {}  id={}  text_hash={}",
                page.page_number,
                page.page_id,
                page.text_hash.as_deref().unwrap_or("none")
            ));
        }
    }

    if !output.chunks.is_empty() {
        lines.push(String::new());
        lines.push(format!("=== Chunks ({} total) ===", output.chunks.len()));
        for chunk in &output.chunks {
            lines.push(format!(
                "  [{}] pages {}-{}  hash={}  embed={}",
                chunk.chunk_id,
                chunk.page_start,
                chunk.page_end,
                &chunk.content_hash[..16.min(chunk.content_hash.len())],
                chunk.embedding_status
            ));
        }
    }

    if !output.ocr_runs.is_empty() {
        lines.push(String::new());
        lines.push(format!(
            "=== OCR Runs ({} total) ===",
            output.ocr_runs.len()
        ));
        for run in &output.ocr_runs {
            lines.push(format!(
                "  {}  provider={}  status={:?}  duration={:?}",
                run.ocr_run_id, run.provider, run.status, run.duration_ms
            ));
        }
    }

    if !output.provenance_edges.is_empty() {
        lines.push(String::new());
        lines.push(format!(
            "=== Provenance Edges ({} total) ===",
            output.provenance_edges.len()
        ));
        for edge in &output.provenance_edges {
            lines.push(format!(
                "  {} -> {}  type={:?}",
                edge.from_artifact_id, edge.to_artifact_id, edge.edge_type
            ));
        }
    }

    lines.join("\n")
}

/// List all documents with summary information.
pub fn format_list_output(documents: &[SourceDocument]) -> String {
    if documents.is_empty() {
        return "No documents ingested.".to_string();
    }

    let mut lines = vec![format!(
        "{} document(s):",
        documents.len()
    )];

    for doc in documents {
        lines.push(format!(
            "  {}  {:?}  {}  [{}]",
            doc.document_id,
            doc.status,
            doc.source_path,
            &doc.content_hash[..12.min(doc.content_hash.len())]
        ));
    }

    lines.join("\n")
}
