//! Layout port (S-3) — running-head locator capture (Bekker numbers / page numbers).
//!
//! Inverts `layout_analyzer.py`'s strip-and-discard (`_PAGE_NUM_RE:53`, `_BEKKER_RE:54`):
//! a block whose entire text is a standalone page number or Bekker number is **captured
//! as a citation locator** and removed from the body stream (strip + capture). Bekker
//! numbers become first-class anchors the verbatim subsystem can resolve.
//!
//! Deviation from the reference (documented, citation-correctness): the reference
//! `_BEKKER_RE = \d{2,3}[a-b]?\d{0,2}` caps at 3 digits and makes the column letter
//! optional, so it (a) misses 4-digit Aristotle Bekker numbers like `1147a` and
//! (b) conflates letterless numbers with page numbers. We require the column letter and
//! allow 1–4 leading digits + up to 3 line digits — matching real Bekker citations
//! (`184b15`, `1147a`, `1147a13`) and keeping Bekker distinct from page numbers.

use regex::Regex;

use crate::chunk::Block;

/// What kind of running-head locator was captured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocatorKind {
    PageNumber,
    Bekker,
}

/// A captured locator: its page, kind, normalized value, and bbox (sentinel on non-Marker paths).
#[derive(Clone, Debug, PartialEq)]
pub struct LocatorHit {
    pub page: u32,
    pub kind: LocatorKind,
    pub value: String,
    pub bbox: [f32; 4],
}

/// Plain running-head page number, optionally with an OCR'd trailing `o`/`O`
/// (faithful to `_PAGE_NUM_RE:53`).
fn page_num_re() -> Regex {
    Regex::new(r"^\s*\d{1,3}[oO]?\s*$").expect("static page-num regex")
}

/// Bekker number — broadened from the reference (see module docs): 1–4 digits, a REQUIRED
/// column letter `a`/`b`, optional 1–3 line-number digits.
fn bekker_re() -> Regex {
    Regex::new(r"^\s*\d{1,4}[ab]\d{0,3}\s*$").expect("static bekker regex")
}

/// Split a block stream into (body blocks kept, locator hits removed). A block is pulled out
/// as a locator only when its ENTIRE trimmed text is a standalone Bekker or page number;
/// everything else passes through unchanged. Bekker is checked first (more specific).
pub fn extract_locators(blocks: &[Block]) -> (Vec<Block>, Vec<LocatorHit>) {
    let page_re = page_num_re();
    let bekker_re = bekker_re();
    let mut kept = Vec::with_capacity(blocks.len());
    let mut hits = Vec::new();
    for b in blocks {
        let t = b.text.trim();
        if bekker_re.is_match(t) {
            hits.push(LocatorHit {
                page: b.page,
                kind: LocatorKind::Bekker,
                value: t.to_string(),
                bbox: b.bbox,
            });
        } else if page_re.is_match(t) {
            hits.push(LocatorHit {
                page: b.page,
                kind: LocatorKind::PageNumber,
                value: t.to_string(),
                bbox: b.bbox,
            });
        } else {
            kept.push(b.clone());
        }
    }
    (kept, hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::BlockType;

    fn blk(text: &str, page: u32) -> Block {
        Block { block_type: BlockType::Text, text: text.to_string(), bbox: [1.0, 2.0, 3.0, 4.0], page }
    }

    #[test]
    fn captures_bekker_and_page_numbers_and_strips_them() {
        let blocks = vec![
            blk("Real prose about energeia and entelecheia.", 1),
            blk("1147a", 1),          // 4-digit Bekker (reference would MISS this)
            blk("184b15", 2),         // Bekker with line number
            blk("47", 2),             // plain page number
            blk("More body text here.", 3),
            blk("see 1147a below", 3), // NOT standalone → kept
        ];
        let (kept, hits) = extract_locators(&blocks);
        assert_eq!(kept.len(), 3, "two body paras + the inline-reference para remain");
        assert!(kept.iter().any(|b| b.text.starts_with("see 1147a")));

        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].kind, LocatorKind::Bekker);
        assert_eq!(hits[0].value, "1147a");
        assert_eq!(hits[0].page, 1);
        assert_eq!(hits[0].bbox, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(hits[1].kind, LocatorKind::Bekker);
        assert_eq!(hits[1].value, "184b15");
        assert_eq!(hits[2].kind, LocatorKind::PageNumber);
        assert_eq!(hits[2].value, "47");
    }

    #[test]
    fn letterless_number_is_a_page_number_not_bekker() {
        let (_, hits) = extract_locators(&[blk("123", 5)]);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, LocatorKind::PageNumber, "no column letter → page number");
    }

    #[test]
    fn empty_when_no_standalone_numbers() {
        let blocks = vec![blk("ordinary paragraph", 1), blk("another one", 1)];
        let (kept, hits) = extract_locators(&blocks);
        assert_eq!(kept.len(), 2);
        assert!(hits.is_empty());
    }
}
