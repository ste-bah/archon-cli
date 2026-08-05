//! Token-aware, bbox-carrying chunker (Port C — the spine).
//!
//! Faithful to `chunk_marker_json` (`markdown_chunker.py`) per spec §2:
//! - constants TARGET_MIN=800 / TARGET_MAX=1200 / HARD_MAX=1400 tokens, chars/4
//! - flush when adding a block would exceed `max`, OR at a SectionHeader once `>= min`
//! - per-page bbox super-box on flush; merge undersized trailing chunks into the next
//!   iff same/adjacent page (`next.page_start <= cur.page_end + 1`)
//!
//! `ChunkOut` maps 1:1 to Archon's `PageChunk { content, page_start, page_end }`, and
//! `ChunkOut.bboxes` feeds `doc_chunk_spatial` (verbatim-provenance spec).

pub const TARGET_MIN: usize = 800;
pub const TARGET_MAX: usize = 1200;
pub const HARD_MAX: usize = 1400;
const CHARS_PER_TOKEN: usize = 4;

/// Marker block types that carry structure (`markdown_chunker.py:230`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockType {
    Text,
    SectionHeader,
    Table,
    ListItem,
    Caption,
}

/// A normalized block from Marker's JSON tree: `{ type, text, bbox, page }`.
#[derive(Clone, Debug)]
pub struct Block {
    pub block_type: BlockType,
    pub text: String,
    pub bbox: [f32; 4],
    pub page: u32,
}

/// A figure/picture region detected by Marker (image content, no text). `bbox` is in Marker's
/// coordinate space — PDF points, top-left origin. Collected by a SEPARATE walk from the text-block
/// stream (so chunk parity is untouched) and fed to the opt-in figure-region VLM path, which crops
/// the region from a page render and describes it — the only way to caption figures baked into a
/// scanned page (no discrete embedded image to describe).
#[derive(Clone, Debug, PartialEq)]
pub struct FigureRegion {
    pub page: u32,
    pub bbox: [f32; 4],
}

/// Per-page bbox group: the union super-box plus the individual block boxes.
#[derive(Clone, Debug, PartialEq)]
pub struct PageBoxes {
    pub page_num: u32,
    pub super_box: [f32; 4],
    pub blocks: Vec<[f32; 4]>,
}

/// One block's placement inside the emitted chunk (P2): its BYTE range within `ChunkOut.text`
/// (`&text[char_start..char_end]` is byte-exact), plus its page, bbox and type. Drives
/// sentence-tight bbox + per-block locator offsets. Offsets are recomputed on merge.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockSpan {
    pub char_start: usize,
    pub char_end: usize,
    pub page: u32,
    pub bbox: [f32; 4],
    pub block_type: BlockType,
}

/// One emitted chunk. Maps to Archon `PageChunk`; `bboxes` → `doc_chunk_spatial`, and
/// `blocks` → `doc_chunk_blocks` + `doc_chunk_page_breaks` (P2).
#[derive(Clone, Debug, PartialEq)]
pub struct ChunkOut {
    pub text: String,
    pub page_start: u32,
    pub page_end: u32,
    pub bboxes: Vec<PageBoxes>,
    pub blocks: Vec<BlockSpan>,
}

/// Token estimate: code-points / 4 (CHARS_PER_TOKEN=4). Counts Unicode CODE POINTS to match
/// the Python reference `_estimate_tokens` (`len(text)//4`) — byte length (`s.len()`) would
/// over-count multibyte text (Greek ἐνέργεια, German umlauts) and shift every flush boundary
/// on exactly this corpus, diverging from the reference's chunk boundaries/hashes.
pub fn est_tokens(s: &str) -> usize {
    s.chars().count() / CHARS_PER_TOKEN
}

/// `merge_bboxes:233` — min x0, min y0, max x1, max y1.
fn merge_bboxes(boxes: &[[f32; 4]]) -> [f32; 4] {
    let mut r = boxes[0];
    for b in &boxes[1..] {
        r[0] = r[0].min(b[0]);
        r[1] = r[1].min(b[1]);
        r[2] = r[2].max(b[2]);
        r[3] = r[3].max(b[3]);
    }
    r
}

/// Accumulates blocks into the current chunk, tracking page lineage + per-page boxes.
#[derive(Default)]
struct Accum {
    text: String,
    page_start: Option<u32>,
    page_end: u32,
    /// Insertion-ordered page → boxes.
    pages: Vec<(u32, Vec<[f32; 4]>)>,
    /// Per-block placement (P2): byte range within `text`, page, bbox, type.
    blocks: Vec<BlockSpan>,
}

impl Accum {
    fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    fn add(&mut self, b: &Block) {
        // Capture the block's BYTE range within the accumulating chunk text (P2). The "\n\n"
        // separator is inserted BEFORE the block, so `char_start` is the byte offset just after it
        // and `&text[char_start..char_end]` is byte-exact for the block.
        let char_start = if self.text.is_empty() {
            self.page_start = Some(b.page);
            0
        } else {
            self.text.push_str("\n\n");
            self.text.len()
        };
        self.text.push_str(&b.text);
        let char_end = self.text.len();
        self.blocks.push(BlockSpan {
            char_start,
            char_end,
            page: b.page,
            bbox: b.bbox,
            block_type: b.block_type,
        });
        self.page_end = b.page;
        if let Some(entry) = self.pages.iter_mut().find(|(p, _)| *p == b.page) {
            entry.1.push(b.bbox);
        } else {
            self.pages.push((b.page, vec![b.bbox]));
        }
    }

    fn flush(&mut self) -> ChunkOut {
        let bboxes = self
            .pages
            .iter()
            .map(|(p, boxes)| PageBoxes {
                page_num: *p,
                super_box: merge_bboxes(boxes),
                blocks: boxes.clone(),
            })
            .collect();
        let out = ChunkOut {
            text: std::mem::take(&mut self.text),
            page_start: self.page_start.take().unwrap_or(0),
            page_end: self.page_end,
            bboxes,
            blocks: std::mem::take(&mut self.blocks),
        };
        self.pages.clear();
        self.page_end = 0;
        out
    }
}

/// Chunk a Marker block stream. `min`/`max`/`hard` are token budgets (see constants).
/// `hard` is reserved for the markdown-fallback path's oversized-paragraph split; the
/// block-tree path splits on `max` and heading boundaries per spec §2.
pub fn chunk_blocks(blocks: &[Block], min: usize, max: usize, _hard: usize) -> Vec<ChunkOut> {
    let mut chunks: Vec<ChunkOut> = Vec::new();
    let mut acc = Accum::default();

    for b in blocks {
        if !acc.is_empty() {
            // est_tokens(current + "\n\n" + text) over code points (parity with the reference).
            let combined = acc.text.chars().count() + 2 + b.text.chars().count();
            let would_exceed = combined / CHARS_PER_TOKEN > max;
            let heading_split =
                b.block_type == BlockType::SectionHeader && est_tokens(&acc.text) >= min;
            if would_exceed || heading_split {
                chunks.push(acc.flush());
            }
        }
        acc.add(b);
    }
    if !acc.is_empty() {
        chunks.push(acc.flush());
    }

    merge_undersized(chunks, min)
}

/// Convenience wrapper with the spec's default token budgets.
pub fn chunk_blocks_default(blocks: &[Block]) -> Vec<ChunkOut> {
    chunk_blocks(blocks, TARGET_MIN, TARGET_MAX, HARD_MAX)
}

/// Union two adjacent chunks: text joined with "\n\n", page span widened, boxes merged per page.
fn merge_two(a: ChunkOut, b: ChunkOut) -> ChunkOut {
    // b's block byte-offsets shift by a.text + the "\n\n" joiner (P2: keep spans byte-exact).
    let shift = a.text.len() + 2;
    let text = format!("{}\n\n{}", a.text, b.text);
    let page_start = a.page_start.min(b.page_start);
    let page_end = a.page_end.max(b.page_end);
    let mut blocks = a.blocks;
    blocks.extend(b.blocks.into_iter().map(|mut bl| {
        bl.char_start += shift;
        bl.char_end += shift;
        bl
    }));
    let mut pages: Vec<(u32, Vec<[f32; 4]>)> = Vec::new();
    for pb in a.bboxes.into_iter().chain(b.bboxes) {
        if let Some(e) = pages.iter_mut().find(|(p, _)| *p == pb.page_num) {
            e.1.extend(pb.blocks);
        } else {
            pages.push((pb.page_num, pb.blocks));
        }
    }
    let bboxes = pages
        .into_iter()
        .map(|(p, boxes)| PageBoxes {
            page_num: p,
            super_box: merge_bboxes(&boxes),
            blocks: boxes,
        })
        .collect();
    ChunkOut {
        text,
        page_start,
        page_end,
        bboxes,
        blocks,
    }
}

/// Merge each undersized chunk (`< min`) into the NEXT iff same/adjacent page
/// (`next.page_start <= cur.page_end + 1`); otherwise emit it as-is (`:405`).
///
/// STRICTLY PAIRWISE, matching the reference (`markdown_chunker.py:405-446`, `i += 2`):
/// once an undersized chunk is folded into its successor, the merged result is NOT
/// re-evaluated for further merging — three adjacent undersized chunks become `[A+B, C]`,
/// never `[A+B+C]`. (A running accumulator would chain them and diverge.)
fn merge_undersized(chunks: Vec<ChunkOut>, min: usize) -> Vec<ChunkOut> {
    let mut out: Vec<ChunkOut> = Vec::new();
    let mut iter = chunks.into_iter().peekable();
    while let Some(cur) = iter.next() {
        if est_tokens(&cur.text) < min {
            if let Some(next) = iter.peek() {
                if next.page_start <= cur.page_end + 1 {
                    let next = iter.next().expect("peeked");
                    out.push(merge_two(cur, next));
                    continue; // skip BOTH; never re-evaluate the merged chunk
                }
            }
        }
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blk(t: BlockType, text: &str, page: u32) -> Block {
        Block {
            block_type: t,
            text: text.to_string(),
            bbox: [0.0, page as f32, 10.0, page as f32 + 1.0],
            page,
        }
    }
    fn big(page: u32, n: usize) -> Block {
        blk(BlockType::Text, &"x".repeat(n), page)
    }

    #[test]
    fn est_tokens_is_chars_over_four() {
        assert_eq!(est_tokens("12345678"), 2);
        assert_eq!(est_tokens(""), 0);
    }

    #[test]
    fn est_tokens_counts_code_points_not_bytes() {
        // Greek: 8 code points → 2 tokens (matches Python len()//4), not byte length (~16 → 4).
        assert_eq!(est_tokens("ἐνέργεια"), 2);
    }

    #[test]
    fn merge_undersized_is_pairwise_not_chained() {
        let mk = |text: &str, p: u32| ChunkOut {
            text: text.into(),
            page_start: p,
            page_end: p,
            bboxes: vec![],
            blocks: vec![],
        };
        // Three adjacent undersized chunks → pairwise [A+B, C], never chained [A+B+C].
        let out = merge_undersized(vec![mk("a", 1), mk("b", 2), mk("c", 3)], TARGET_MIN);
        assert_eq!(out.len(), 2, "pairwise merge, not chained");
        assert_eq!(out[0].text, "a\n\nb");
        assert_eq!(out[0].page_start, 1);
        assert_eq!(out[0].page_end, 2);
        assert_eq!(out[1].text, "c");
        assert_eq!(out[1].page_start, 3);
    }

    #[test]
    fn small_doc_is_one_chunk_with_page_span() {
        let blocks = vec![
            blk(BlockType::Text, "alpha", 1),
            blk(BlockType::Text, "beta", 2),
        ];
        let chunks = chunk_blocks_default(&blocks);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "alpha\n\nbeta");
        assert_eq!(chunks[0].page_start, 1);
        assert_eq!(chunks[0].page_end, 2);
        assert_eq!(chunks[0].bboxes.len(), 2, "one PageBoxes per page");
    }

    #[test]
    fn splits_when_exceeding_max() {
        // Two ~1200-char blocks (≈300 tokens each is small); make each > max so adding the
        // second exceeds max and forces a flush. max=1200 tokens → > 4800 chars combined.
        let blocks = vec![big(1, 4000), big(2, 4000)];
        let chunks = chunk_blocks_default(&blocks);
        assert_eq!(
            chunks.len(),
            2,
            "second block would exceed max → flush first"
        );
        assert_eq!(chunks[0].page_start, 1);
        assert_eq!(chunks[1].page_start, 2);
    }

    #[test]
    fn section_header_splits_once_min_reached() {
        // First block exceeds min (>3200 chars → >800 tokens); the SectionHeader then flushes.
        let blocks = vec![
            big(1, 3600),
            blk(BlockType::SectionHeader, "## New Section", 1),
            blk(BlockType::Text, "body of the new section", 1),
        ];
        let chunks = chunk_blocks_default(&blocks);
        assert_eq!(
            chunks.len(),
            2,
            "heading boundary splits after min is reached"
        );
        assert!(chunks[1].text.starts_with("## New Section"));
    }

    #[test]
    fn header_does_not_split_before_min() {
        // current is tiny (< min) when the header arrives → NO split (header joins current).
        let blocks = vec![
            blk(BlockType::Text, "tiny intro", 1),
            blk(BlockType::SectionHeader, "## Section", 1),
            blk(BlockType::Text, "body", 1),
        ];
        let chunks = chunk_blocks_default(&blocks);
        assert_eq!(chunks.len(), 1, "header below min does not force a split");
    }

    #[test]
    fn undersized_chunk_merges_into_next_adjacent() {
        // big1(p1, 1250 tok) | "mid"(p2, tiny) | big3(p3, 1250 tok).
        // Adding "mid" to big1 exceeds max → flush big1; adding big3 to "mid" exceeds max →
        // flush the undersized "mid"(p2). merge_undersized folds "mid" into big3 (p3 adjacent).
        let blocks = vec![big(1, 5000), blk(BlockType::Text, "mid", 2), big(3, 5000)];
        let chunks = chunk_blocks_default(&blocks);
        assert_eq!(
            chunks.len(),
            2,
            "undersized middle folds into the adjacent next"
        );
        let merged = chunks
            .iter()
            .find(|c| c.text.contains("mid"))
            .expect("mid present");
        assert!(
            merged.text.contains(&"x".repeat(10)),
            "mid merged with the page-3 chunk text"
        );
        assert!(
            merged.page_start <= 2 && merged.page_end >= 3,
            "page span widened across merge"
        );
    }

    #[test]
    fn undersized_tail_with_no_next_is_emitted_as_is() {
        // big1(p1) then a tiny tail that simply joins it (1000+ε tok < max) → single chunk.
        let blocks = vec![big(1, 4000), blk(BlockType::Text, "tail", 2)];
        let chunks = chunk_blocks_default(&blocks);
        assert_eq!(
            chunks.len(),
            1,
            "tail under max joins rather than splitting"
        );
        assert_eq!(chunks[0].page_end, 2);
    }

    #[test]
    fn block_spans_are_byte_exact_including_after_merge() {
        // Force a pairwise merge: big1(p1) | MID(p2) | big3(p3) → MID folds into the p3 chunk, so
        // its byte offsets are SHIFTED. Every span must still slice its exact block text.
        let big1 = "A".repeat(5000);
        let big3 = "C".repeat(5000);
        let blocks = vec![
            blk(BlockType::Text, &big1, 1),
            blk(BlockType::Text, "MID-BLOCK", 2),
            blk(BlockType::Text, &big3, 3),
        ];
        let chunks = chunk_blocks_default(&blocks);
        for c in &chunks {
            for s in &c.blocks {
                assert!(
                    s.char_start <= s.char_end && s.char_end <= c.text.len(),
                    "span in bounds"
                );
                assert!(
                    c.text.is_char_boundary(s.char_start) && c.text.is_char_boundary(s.char_end),
                    "span on char boundaries"
                );
            }
            let joined = c
                .blocks
                .iter()
                .map(|s| &c.text[s.char_start..s.char_end])
                .collect::<Vec<_>>()
                .join("\n\n");
            assert_eq!(joined, c.text, "block spans tile the chunk text exactly");
        }
        let merged = chunks
            .iter()
            .find(|c| c.text.contains("MID-BLOCK"))
            .expect("mid present");
        let mid = merged
            .blocks
            .iter()
            .find(|s| &merged.text[s.char_start..s.char_end] == "MID-BLOCK")
            .expect("mid span byte-exact after shift");
        assert_eq!(mid.page, 2, "shifted span keeps its page");
    }
}
