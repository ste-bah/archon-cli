//! Token-aware (Marker-block) chunk persistence — the S-1 wire-in.
//!
//! Routes PDF ingest through `archon_ingest_ext::chunk::chunk_blocks` instead of the
//! legacy `chunk_with_page_anchors`, mapping `ChunkOut → ChunkArtifact` (keeping the
//! `chunk-{document_id}-{i}` id format that the satellite relations key on) and capturing
//! per-chunk bboxes into `doc_chunk_spatial`. Blocks come either from Marker (real bboxes,
//! `coord_space = "marker"`) or are synthesized from flat extracted text
//! (`coord_space = "none"`, no bboxes) so the token-aware chunker runs on the
//! poppler/pdftotext path too while preserving page lineage.

use cozo::DbInstance;

use archon_ingest_ext::chunk::{chunk_blocks_default, Block, BlockType, ChunkOut, PageBoxes};
use archon_ingest_ext::layout;

use crate::chunking::page_for_offset;
use crate::errors::DocsError;
use crate::hash::sha256_str;
use crate::models::{
    ArtifactRecord, ChunkArtifact, ChunkSpatial, Locator, LocatorKind, PageOffset,
    ProvenanceEdgeType,
};
use crate::provenance::make_edge;
use crate::store;

/// Coordinate-space marker meaning "no spatial info" (flat-text fallback path).
pub const COORD_NONE: &str = "none";
/// Coordinate space for Marker-derived bboxes.
pub const COORD_MARKER: &str = "marker";

/// Synthesize a `Block` stream from flat extracted text + page offsets (no Marker).
/// Splits on blank lines (paragraphs), assigns each paragraph the page that owns its
/// start offset (faithful to `chunk_with_page_anchors`'s running-cursor offset logic),
/// and uses a sentinel bbox. Lets the token-aware chunker run on the pdftotext path
/// while preserving `page_start/page_end` lineage.
pub fn blocks_from_text(text: &str, page_offsets: &[PageOffset]) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut byte_pos = 0usize;
    let segments: Vec<&str> = text.split("\n\n").collect();
    let seg_count = segments.len();
    for (i, raw) in segments.iter().enumerate() {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let local = raw.find(trimmed).unwrap_or(0);
            let page = page_for_offset(byte_pos + local, page_offsets);
            blocks.push(Block {
                block_type: BlockType::Text,
                text: trimmed.to_string(),
                bbox: [0.0, 0.0, 0.0, 0.0],
                page,
            });
        }
        byte_pos += raw.len();
        if i + 1 < seg_count {
            byte_pos += 2; // the "\n\n" separator
        }
    }
    blocks
}

/// Map one `ChunkOut` (chunker output) to a persistable `ChunkArtifact`, preserving the
/// `chunk-{document_id}-{i}` id format the satellite relations join on.
fn chunkout_to_artifact(
    document_id: &str,
    artifact_id: &str,
    i: usize,
    out: &ChunkOut,
) -> ChunkArtifact {
    ChunkArtifact {
        chunk_id: format!("chunk-{}-{}", document_id, i),
        document_id: document_id.to_string(),
        artifact_id: artifact_id.to_string(),
        chunk_index: i as u32,
        page_start: out.page_start,
        page_end: out.page_end,
        content: out.text.clone(),
        content_hash: sha256_str(&out.text),
        embedding_status: "pending".to_string(),
    }
}

fn bbox_json(b: &[f32; 4]) -> serde_json::Value {
    serde_json::json!([b[0], b[1], b[2], b[3]])
}

fn bbox_json_string(b: &[f32; 4]) -> String {
    serde_json::to_string(&bbox_json(b)).unwrap_or_else(|_| "[]".to_string())
}

fn map_locator_kind(k: layout::LocatorKind) -> LocatorKind {
    match k {
        layout::LocatorKind::Bekker => LocatorKind::Bekker,
        layout::LocatorKind::PageNumber => LocatorKind::PageNumber,
    }
}

/// Build the per-chunk spatial row, or `None` on the no-spatial (flat-text) path or when
/// the chunk carries no boxes. `blocks` holds the FULL per-page structure (lossless,
/// page-sorted): `[{"page_num", "super_box", "blocks":[[x0,y0,x1,y1],…]}, …]`. `page_num`
/// and `super_box` are the primary (page_start) page's, for indexing/overlay convenience.
/// `spatial_hash = sha256(coord_space ∥ canonical(blocks_json))` — covered by the chain.
pub(crate) fn chunkout_spatial(
    chunk_id: &str,
    out: &ChunkOut,
    coord_space: &str,
) -> Option<ChunkSpatial> {
    if coord_space == COORD_NONE || out.bboxes.is_empty() {
        return None;
    }
    let mut pages: Vec<&PageBoxes> = out.bboxes.iter().collect();
    pages.sort_by_key(|p| p.page_num);
    let full = serde_json::Value::Array(
        pages
            .iter()
            .map(|p| {
                serde_json::json!({
                    "page_num": p.page_num,
                    "super_box": bbox_json(&p.super_box),
                    "blocks": serde_json::Value::Array(p.blocks.iter().map(bbox_json).collect()),
                })
            })
            .collect(),
    );
    let blocks_json = serde_json::to_string(&full).unwrap_or_else(|_| "[]".to_string());
    let primary = pages
        .iter()
        .find(|p| p.page_num == out.page_start)
        .or_else(|| pages.first());
    let super_box = primary.map(|p| p.super_box).unwrap_or([0.0; 4]);
    let super_json = serde_json::to_string(&bbox_json(&super_box)).unwrap_or_else(|_| "[]".to_string());
    let spatial_hash = sha256_str(&format!("{}\u{0}{}", coord_space, blocks_json));
    Some(ChunkSpatial {
        chunk_id: chunk_id.to_string(),
        page_num: out.page_start,
        super_box: super_json,
        blocks: blocks_json,
        coord_space: coord_space.to_string(),
        spatial_hash,
    })
}

/// Persist a token-aware chunking of `blocks`: inserts the OCR text artifact, the chunks
/// (token-budgeted, page-lineage preserved), and — when `coord_space != "none"` — each
/// chunk's `doc_chunk_spatial` row. Returns the chunks and the spatial rows written.
///
/// `full_text` is hashed for the artifact's `content_hash` (parity with the legacy path);
/// `blocks` is what actually gets chunked.
pub(crate) fn persist_block_chunks(
    db: &DbInstance,
    document_id: &str,
    artifact_id: &str,
    artifact_type: &str,
    blocks: &[Block],
    coord_space: &str,
) -> Result<(Vec<ChunkArtifact>, Vec<ChunkSpatial>), DocsError> {
    // S-3: strip + capture standalone Bekker / page-number blocks as citation locators
    // before chunking, so they leave the embed text but become resolvable anchors.
    let (clean_blocks, locator_hits) = layout::extract_locators(blocks);
    // Guard: if EVERY block was a standalone number, don't strip the document down to
    // nothing (which would yield zero chunks) — keep the content, capture no locators.
    let (clean_blocks, locator_hits) = if clean_blocks.is_empty() && !blocks.is_empty() {
        (blocks.to_vec(), Vec::new())
    } else {
        (clean_blocks, locator_hits)
    };

    // Artifact content_hash reflects the ACTUAL ingested content (post strip / table-render),
    // not the pdftotext layer — faithful on the Marker path where the two differ.
    let artifact_content = clean_blocks
        .iter()
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let artifact = ArtifactRecord {
        artifact_id: artifact_id.to_string(),
        document_id: document_id.to_string(),
        artifact_type: artifact_type.to_string(),
        content_hash: sha256_str(&artifact_content),
        created_at: chrono::Utc::now().to_rfc3339(),
        provenance_record_id: String::new(),
    };
    store::insert_artifact(db, &artifact).map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;

    for (i, h) in locator_hits.iter().enumerate() {
        store::insert_locator(
            db,
            &Locator {
                locator_id: format!("loc-{}-{}", document_id, i),
                document_id: document_id.to_string(),
                page_num: h.page,
                kind: map_locator_kind(h.kind),
                value: h.value.clone(),
                bbox: bbox_json_string(&h.bbox),
            },
        )
        .map_err(|e| DocsError::Storage {
            message: e.to_string(),
        })?;
    }

    let chunkouts = chunk_blocks_default(&clean_blocks);
    let mut chunks = Vec::with_capacity(chunkouts.len());
    let mut spatials = Vec::new();
    for (i, out) in chunkouts.iter().enumerate() {
        let chunk = chunkout_to_artifact(document_id, artifact_id, i, out);
        store::insert_chunk(db, &chunk).map_err(|e| DocsError::Storage {
            message: e.to_string(),
        })?;
        if let Some(spatial) = chunkout_spatial(&chunk.chunk_id, out, coord_space) {
            store::insert_chunk_spatial(db, &spatial).map_err(|e| DocsError::Storage {
                message: e.to_string(),
            })?;
            spatials.push(spatial);
        }
        chunks.push(chunk);
    }

    store::insert_provenance_edge(
        db,
        &make_edge(artifact_id, document_id, ProvenanceEdgeType::DerivedFrom),
    )
    .map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;

    Ok((chunks, spatials))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-blockchunk-{}.db", uuid::Uuid::new_v4());
        DbInstance::new("sqlite", &path, "").unwrap()
    }

    #[test]
    fn blocks_from_text_assigns_pages() {
        // page 1 owns [0, 20), page 2 owns [20, end).
        let text = "First para.\n\nSecond para on page two.";
        let offsets = vec![
            PageOffset { page: 1, char_start: 0, char_end: 13 },
            PageOffset { page: 2, char_start: 13, char_end: text.len() },
        ];
        let blocks = blocks_from_text(text, &offsets);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].text, "First para.");
        assert_eq!(blocks[0].page, 1);
        assert_eq!(blocks[1].page, 2);
        assert_eq!(blocks[0].bbox, [0.0, 0.0, 0.0, 0.0], "flat-text path has sentinel bbox");
    }

    #[test]
    fn spatial_is_none_for_flat_text_path() {
        let out = ChunkOut {
            text: "x".into(),
            page_start: 1,
            page_end: 1,
            bboxes: vec![PageBoxes { page_num: 1, super_box: [1.0, 2.0, 3.0, 4.0], blocks: vec![[1.0, 2.0, 3.0, 4.0]] }],
        };
        assert!(chunkout_spatial("chunk-d-0", &out, COORD_NONE).is_none());
        let s = chunkout_spatial("chunk-d-0", &out, COORD_MARKER).expect("marker → spatial");
        assert_eq!(s.page_num, 1);
        assert_eq!(s.coord_space, "marker");
        assert_eq!(s.super_box, "[1.0,2.0,3.0,4.0]");
        assert!(s.blocks.contains("\"page_num\":1"));
        assert_eq!(s.spatial_hash.len(), 64, "sha256 hex");
    }

    #[test]
    fn persist_token_aware_marker_path_writes_chunks_and_spatial() {
        let db = test_db();
        crate::schema::ensure_doc_schema(&db).unwrap();
        // Two big single-page blocks → two chunks (each exceeds max alone), with real bboxes.
        let big = "word ".repeat(1200); // ~6000 chars → ~1500 tok > max
        let blocks = vec![
            Block { block_type: BlockType::Text, text: big.clone(), bbox: [10.0, 20.0, 100.0, 40.0], page: 1 },
            Block { block_type: BlockType::Text, text: big, bbox: [10.0, 50.0, 100.0, 80.0], page: 2 },
        ];
        let (chunks, spatials) = persist_block_chunks(
            &db, "docA", "ocr-docA", "ocr_text", &blocks, COORD_MARKER,
        )
        .unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].chunk_id, "chunk-docA-0");
        assert_eq!(chunks[1].chunk_id, "chunk-docA-1");
        assert_eq!(spatials.len(), 2, "marker path writes a spatial row per chunk");

        // Roundtrip the spatial row from the DB.
        let got = store::get_chunk_spatial(&db, "chunk-docA-0").unwrap().unwrap();
        assert_eq!(got.coord_space, "marker");
        assert_eq!(got.page_num, 1);
        let listed = store::list_chunks_for_doc(&db, "docA").unwrap();
        assert_eq!(listed.len(), 2);
    }

    #[test]
    fn persist_flat_text_path_writes_no_spatial() {
        let db = test_db();
        crate::schema::ensure_doc_schema(&db).unwrap();
        let text = "Short one.\n\nShort two.";
        let offsets = vec![PageOffset { page: 1, char_start: 0, char_end: text.len() }];
        let blocks = blocks_from_text(text, &offsets);
        let (chunks, spatials) = persist_block_chunks(
            &db, "docB", "ocr-docB", "ocr_text", &blocks, COORD_NONE,
        )
        .unwrap();
        assert_eq!(chunks.len(), 1, "small text → one merged chunk");
        assert!(spatials.is_empty(), "flat-text path writes no spatial rows");
        assert!(store::get_chunk_spatial(&db, "chunk-docB-0").unwrap().is_none());
    }

    #[test]
    fn locators_are_captured_and_stripped_from_chunks() {
        let db = test_db();
        crate::schema::ensure_doc_schema(&db).unwrap();
        let blocks = vec![
            Block { block_type: BlockType::Text, text: "Body about energeia.".into(), bbox: [1.0, 2.0, 3.0, 4.0], page: 1 },
            Block { block_type: BlockType::Text, text: "1147a".into(), bbox: [5.0, 6.0, 7.0, 8.0], page: 1 },
            Block { block_type: BlockType::Text, text: "More body.".into(), bbox: [1.0, 2.0, 3.0, 4.0], page: 2 },
        ];
        let (chunks, _spatials) = persist_block_chunks(
            &db, "docL", "ocr-docL", "ocr_text", &blocks, COORD_MARKER,
        )
        .unwrap();
        // The Bekker block left the body text...
        assert!(!chunks.iter().any(|c| c.content.contains("1147a")), "Bekker stripped from body");
        assert!(chunks.iter().any(|c| c.content.contains("energeia")));
        // ...but is captured as a first-class locator with its bbox.
        let locs = store::list_locators_for_doc(&db, "docL").unwrap();
        assert_eq!(locs.len(), 1);
        assert_eq!(locs[0].kind, LocatorKind::Bekker);
        assert_eq!(locs[0].value, "1147a");
        assert_eq!(locs[0].bbox, "[5.0,6.0,7.0,8.0]");
    }
}
