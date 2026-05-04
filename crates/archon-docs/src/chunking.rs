//! Page-aware chunking with document lineage.
//!
//! Splits document text at paragraph boundaries while preserving
//! page-start / page-end provenance for every chunk.

use crate::hash::sha256_str;
use crate::models::{ChunkArtifact, PageOffset};

/// A chunk of document content with page-anchored lineage.
#[derive(Clone, Debug)]
pub struct PageChunk {
    pub content: String,
    pub page_start: u32,
    pub page_end: u32,
}

/// Split text into chunks with page-range annotations.
///
/// Uses blank-line paragraph splitting with minimum chunk size ~200 chars.
/// Page offsets determine which page(s) each chunk spans.
pub fn chunk_with_page_anchors(text: &str, page_offsets: &[PageOffset]) -> Vec<PageChunk> {
    // Collect paragraphs with their actual byte offsets in the original text.
    // Uses a running cursor rather than `text.find(para)` so repeated
    // paragraph text (headers, "Yes", short labels) resolves to the
    // correct occurrence.
    let mut para_entries: Vec<(usize, &str)> = Vec::new();
    let mut byte_pos: usize = 0;
    let raw_segments: Vec<&str> = text.split("\n\n").collect();
    let seg_count = raw_segments.len();

    for (i, raw) in raw_segments.iter().enumerate() {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let local_offset = raw.find(trimmed).unwrap_or(0);
            para_entries.push((byte_pos + local_offset, trimmed));
        }
        byte_pos += raw.len();
        if i + 1 < seg_count {
            byte_pos += 2; // "\n\n" separator
        }
    }

    let mut chunks = Vec::new();
    let mut buffer = String::new();
    let mut buffer_start_page: u32 = 1;
    let mut first_para_in_buffer = true;

    for (para_start, para) in &para_entries {
        let para_end = para_start + para.len();
        let para_page = page_for_offset(*para_start, page_offsets);

        if first_para_in_buffer {
            buffer_start_page = para_page;
            first_para_in_buffer = false;
        }

        if buffer.is_empty() {
            buffer.push_str(para);
        } else {
            buffer.push_str("\n\n");
            buffer.push_str(para);
        }

        // Flush when buffer is large enough
        if buffer.len() >= 200 {
            let para_end_page = page_for_offset(para_end, page_offsets);
            chunks.push(PageChunk {
                content: buffer.clone(),
                page_start: buffer_start_page,
                page_end: para_end_page,
            });
            buffer.clear();
            first_para_in_buffer = true;
        }
    }

    // Flush remaining
    if !buffer.is_empty() {
        let last_page = page_offsets.last().map(|o| o.page).unwrap_or(1);
        chunks.push(PageChunk {
            content: buffer,
            page_start: buffer_start_page,
            page_end: last_page,
        });
    }

    chunks
}

/// Build ChunkArtifact structs from PageChunks, assigning stable IDs
/// and content hashes.
pub fn build_chunk_artifacts(
    document_id: &str,
    artifact_id: &str,
    chunks: &[PageChunk],
) -> Vec<ChunkArtifact> {
    chunks
        .iter()
        .enumerate()
        .map(|(i, pc)| ChunkArtifact {
            chunk_id: format!("chunk-{}-{}", document_id, i),
            document_id: document_id.to_string(),
            artifact_id: artifact_id.to_string(),
            chunk_index: i as u32,
            page_start: pc.page_start,
            page_end: pc.page_end,
            content: pc.content.clone(),
            content_hash: sha256_str(&pc.content),
            embedding_status: "pending".to_string(),
        })
        .collect()
}

fn page_for_offset(char_offset: usize, offsets: &[PageOffset]) -> u32 {
    for o in offsets {
        if char_offset >= o.char_start && char_offset < o.char_end {
            return o.page;
        }
    }
    offsets.last().map(|o| o.page).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_page_chunking() {
        let text = "Paragraph one.\n\nParagraph two.\n\nParagraph three.";
        let offsets = vec![PageOffset {
            page: 1,
            char_start: 0,
            char_end: text.len(),
        }];

        let chunks = chunk_with_page_anchors(text, &offsets);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].page_start, 1);
        assert_eq!(chunks[0].page_end, 1);
    }

    #[test]
    fn test_multi_page_chunking() {
        let text = "Page one content.\n\nMore page one.\n\x0C\nPage two content.";
        let offsets = vec![
            PageOffset {
                page: 1,
                char_start: 0,
                char_end: 32,
            },
            PageOffset {
                page: 2,
                char_start: 33,
                char_end: text.len(),
            },
        ];

        let chunks = chunk_with_page_anchors(text, &offsets);
        assert!(!chunks.is_empty());
        assert_eq!(chunks[0].page_start, 1);
        assert_eq!(chunks[0].page_end, 2);
    }

    #[test]
    fn test_build_chunk_artifacts_lineage() {
        let chunks = vec![
            PageChunk {
                content: "Chunk A content.".into(),
                page_start: 1,
                page_end: 2,
            },
            PageChunk {
                content: "Chunk B content here.".into(),
                page_start: 3,
                page_end: 3,
            },
        ];

        let artifacts = build_chunk_artifacts("doc-1", "art-1", &chunks);
        assert_eq!(artifacts.len(), 2);
        assert_eq!(artifacts[0].document_id, "doc-1");
        assert_eq!(artifacts[0].chunk_id, "chunk-doc-1-0");
        assert_eq!(artifacts[0].page_start, 1);
        assert_eq!(artifacts[0].page_end, 2);
        assert_eq!(artifacts[0].embedding_status, "pending");
        assert!(!artifacts[0].content_hash.is_empty());
        assert_eq!(artifacts[1].chunk_id, "chunk-doc-1-1");
        assert_eq!(artifacts[1].page_start, 3);
        assert_eq!(artifacts[1].page_end, 3);
        assert_ne!(artifacts[0].content_hash, artifacts[1].content_hash);
    }

    #[test]
    fn test_repeated_paragraph_gets_correct_offset() {
        // "Header" appears twice — must get different offsets
        let text = "Header\n\nBody paragraph.\n\nHeader\n\nFooter text.";
        let offsets = vec![PageOffset {
            page: 1,
            char_start: 0,
            char_end: text.len(),
        }];

        let chunks = chunk_with_page_anchors(text, &offsets);
        // All paragraphs are short, so one chunk containing all
        assert_eq!(chunks.len(), 1);
        // Content should have both Headers (joined by \n\n)
        assert!(chunks[0].content.contains("Header"));
        // Two occurrences
        assert_eq!(chunks[0].content.matches("Header").count(), 2);
    }

    #[test]
    fn test_repeated_header_across_page_boundary() {
        // Build a two-page fixture: "Header" on page 1, form-feed, "Header" on page 2.
        // Each page has enough content to force separate chunks.
        let a200 = "a".repeat(200);
        let b200 = "b".repeat(200);
        let text = format!("Header\n\n{}\n\x0C\nHeader\n\n{}", a200, b200);

        // Page 1: "Header\n\n" + a200 + "\n" + "\x0C" (exclusive)
        // Page 2: "\n" + "Header\n\n" + b200
        let page1_end = "Header\n\n".len() + a200.len() + 1 + 1; // +1 for \n, +1 for \x0C
        let offsets = vec![
            PageOffset {
                page: 1,
                char_start: 0,
                char_end: page1_end,
            },
            PageOffset {
                page: 2,
                char_start: page1_end,
                char_end: text.len(),
            },
        ];

        let chunks = chunk_with_page_anchors(&text, &offsets);
        // Should produce at least 2 chunks (one per page)
        assert!(
            chunks.len() >= 2,
            "expected ≥2 chunks, got {}",
            chunks.len()
        );

        // Find the chunk containing the second "Header" — it must be on page 2
        let second_header_chunk = chunks
            .iter()
            .find(|c| c.content.contains("Header") && c.page_start == 2)
            .or_else(|| {
                chunks
                    .iter()
                    .find(|c| c.content.contains("Header") && c.page_end == 2)
            });
        assert!(
            second_header_chunk.is_some(),
            "second Header must be in a chunk assigned to page 2"
        );
    }
}
