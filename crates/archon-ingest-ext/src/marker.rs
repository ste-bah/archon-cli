//! Marker JSON → normalized `Block` stream (Port #2 / S-0 substrate).
//!
//! Faithful port of `markdown_chunker.py::_walk_blocks:250` — an iterative,
//! stack-based walk of Marker's nested block tree (avoids recursion on huge docs,
//! e.g. von Uexküll 188 MB). It emits the typed text blocks that feed the chunker
//! (`chunk::chunk_blocks`), the table gate (`table`), and `doc_chunk_spatial` bboxes.
//!
//! Rules (verbatim to the reference):
//! - A `Page` block's `id` (`/page/N/…`) sets the current page to `N + 1`
//!   (Marker is 0-indexed; Archon lineage is 1-indexed) — `:269`.
//! - Only blocks whose `block_type` ∈ {Text, SectionHeader, Table, ListItem, Caption}
//!   are emitted, and only when they carry BOTH non-empty HTML-stripped text AND a bbox
//!   (`if text and bbox`, `:279`).
//! - Children are visited in document order (`:292`, reversed push onto the stack).

use regex::Regex;
use serde_json::Value;

use crate::chunk::{Block, BlockType};

/// Marker block types that carry structure and text (`_TEXT_BLOCK_TYPES:230`).
/// Returns `None` for structural/non-text blocks (Page, Document, Figure, …).
fn block_type_from_str(s: &str) -> Option<BlockType> {
    match s {
        "Text" => Some(BlockType::Text),
        "SectionHeader" => Some(BlockType::SectionHeader),
        "Table" => Some(BlockType::Table),
        "ListItem" => Some(BlockType::ListItem),
        "Caption" => Some(BlockType::Caption),
        _ => None,
    }
}

/// Strip HTML tags — faithful to `_extract_text_from_html:245`
/// (`re.sub(r'<[^>]+>', '', html).strip()`). Note `<[^>]+>` requires ≥1 char
/// between the brackets, so a literal `<>` is left intact, exactly like the reference.
pub fn strip_html(html: &str) -> String {
    let re = Regex::new(r"<[^>]+>").expect("static tag regex");
    re.replace_all(html, "").trim().to_string()
}

/// Parse a Marker `bbox` value `[x0, y0, x1, y1]` into `[f32; 4]`.
/// A missing / non-4-element / non-numeric bbox yields `None` → the block is skipped
/// (the reference's `if text and bbox` guard, and `merge_bboxes` needs 4 coords).
fn parse_bbox(v: &Value) -> Option<[f32; 4]> {
    let arr = v.as_array()?;
    if arr.len() != 4 {
        return None;
    }
    let mut out = [0f32; 4];
    for (i, e) in arr.iter().enumerate() {
        out[i] = e.as_f64()? as f32;
    }
    Some(out)
}

/// Build a `Block` for an emittable Marker block. `Table` blocks (Port T) are parsed into a
/// cell grid and, if they pass `table::is_real_table`, rendered as a `[TABLE] …` chunk so
/// tables flow through the chunker as first-class, citable chunks; a rejected table (prose
/// false-positive) falls back to stripped text. Returns `None` when there is no text.
fn build_block(block_type: BlockType, html: &str, bbox: [f32; 4], page: u32) -> Option<Block> {
    if block_type == BlockType::Table {
        let grid = crate::table::parse_table_html(html);
        if crate::table::is_real_table(&grid, &crate::table::default_title_markers()) {
            let g = crate::table::TableGrid {
                page_num: page,
                rows: grid,
                bbox,
            };
            return Some(Block {
                block_type: BlockType::Table,
                text: crate::table::table_chunk_text(&g, "", ""),
                bbox,
                page,
            });
        }
        // Not a genuine data table → keep the content as prose text rather than drop it.
        let text = strip_html(html);
        return if text.is_empty() {
            None
        } else {
            Some(Block {
                block_type: BlockType::Text,
                text,
                bbox,
                page,
            })
        };
    }
    let text = strip_html(html);
    if text.is_empty() {
        None
    } else {
        Some(Block {
            block_type,
            text,
            bbox,
            page,
        })
    }
}

/// Walk Marker's parsed JSON tree into a flat `Vec<Block>` in document order.
/// Faithful port of `_walk_blocks` — see module docs.
pub fn parse_marker_json(root: &Value) -> Vec<Block> {
    let page_re = Regex::new(r"^/page/(\d+)/").expect("static page-id regex");
    let mut blocks: Vec<Block> = Vec::new();
    // Stack of (node, current_page). Start at page 1 (reference default).
    let mut stack: Vec<(&Value, u32)> = vec![(root, 1)];

    while let Some((node, cur_page)) = stack.pop() {
        let bt = node.get("block_type").and_then(Value::as_str).unwrap_or("");

        // Track the current page from `Page` blocks (0-indexed id → 1-indexed page).
        let mut page = cur_page;
        if bt == "Page"
            && let Some(id) = node.get("id").and_then(Value::as_str)
            && let Some(caps) = page_re.captures(id)
            && let Ok(n) = caps[1].parse::<u32>()
        {
            page = n + 1;
        }

        // Emit text-bearing blocks with a bbox (Table blocks become [TABLE] chunks).
        if let Some(block_type) = block_type_from_str(bt) {
            let html = node.get("html").and_then(Value::as_str).unwrap_or("");
            if let Some(bbox) = node.get("bbox").and_then(parse_bbox)
                && let Some(block) = build_block(block_type, html, bbox, page)
            {
                blocks.push(block);
            }
        }

        // Push children reversed so they pop (and emit) in document order.
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            for child in children.iter().rev() {
                stack.push((child, page));
            }
        }
    }

    blocks
}

/// Convenience: parse a Marker JSON string into a `Vec<Block>`.
/// This is the contract the Marker sidecar client uses (stdout → blocks).
pub fn parse_marker_str(json: &str) -> Result<Vec<Block>, serde_json::Error> {
    let value: Value = serde_json::from_str(json)?;
    Ok(parse_marker_json(&value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::{BlockType, chunk_blocks_default};

    /// A two-page Marker tree exercising: page increment (0→1, 1→2), HTML strip,
    /// nested-tag text, document order, and skipping a non-text (Figure) block.
    fn fixture() -> Value {
        serde_json::json!({
            "block_type": "Document",
            "children": [
                {
                    "block_type": "Page",
                    "id": "/page/0/Page/0",
                    "children": [
                        {"block_type": "SectionHeader", "html": "<h1>Intro</h1>", "bbox": [10, 20, 100, 40]},
                        {"block_type": "Text", "html": "<p>Hello <i>world</i>.</p>", "bbox": [10, 50, 200, 80]}
                    ]
                },
                {
                    "block_type": "Page",
                    "id": "/page/1/Page/1",
                    "children": [
                        {"block_type": "Text", "html": "<p>Second page.</p>", "bbox": [10, 20, 200, 50]},
                        {"block_type": "Figure", "html": "<img/>", "bbox": [10, 60, 200, 200]},
                        {"block_type": "Caption", "html": "Fig 1.", "bbox": [10, 210, 200, 230]}
                    ]
                }
            ]
        })
    }

    #[test]
    fn strip_html_removes_tags_and_trims() {
        assert_eq!(strip_html("<p>Hello <i>world</i>.</p>"), "Hello world.");
        assert_eq!(strip_html("  plain  "), "plain");
        // `<[^>]+>` needs ≥1 char between brackets → a literal `<>` is preserved.
        assert_eq!(strip_html("a<>b"), "a<>b");
    }

    #[test]
    fn walks_blocks_in_document_order_with_pages() {
        let blocks = parse_marker_json(&fixture());
        // SectionHeader p1, Text p1, Text p2, Caption p2 — the Figure is skipped.
        assert_eq!(blocks.len(), 4, "Figure (non-text) skipped");

        assert_eq!(blocks[0].block_type, BlockType::SectionHeader);
        assert_eq!(blocks[0].text, "Intro");
        assert_eq!(blocks[0].page, 1);
        assert_eq!(blocks[0].bbox, [10.0, 20.0, 100.0, 40.0]);

        assert_eq!(blocks[1].block_type, BlockType::Text);
        assert_eq!(blocks[1].text, "Hello world.");
        assert_eq!(blocks[1].page, 1);

        assert_eq!(blocks[2].text, "Second page.");
        assert_eq!(blocks[2].page, 2, "page id /page/1/ → 1-indexed page 2");

        assert_eq!(blocks[3].block_type, BlockType::Caption);
        assert_eq!(blocks[3].text, "Fig 1.");
        assert_eq!(blocks[3].page, 2);
    }

    #[test]
    fn skips_blocks_missing_text_or_bbox() {
        let tree = serde_json::json!({
            "block_type": "Page",
            "id": "/page/0/Page/0",
            "children": [
                {"block_type": "Text", "html": "<p></p>", "bbox": [0, 0, 1, 1]},   // empty text
                {"block_type": "Text", "html": "<p>kept</p>"},                       // no bbox
                {"block_type": "Text", "html": "<p>good</p>", "bbox": [1, 2, 3, 4]} // emitted
            ]
        });
        let blocks = parse_marker_json(&tree);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, "good");
    }

    #[test]
    fn parse_str_then_chunk_preserves_page_span() {
        // S-0 → S-1 handshake: parsed blocks feed the chunker and keep page lineage.
        let json = fixture().to_string();
        let blocks = parse_marker_str(&json).expect("valid json");
        let chunks = chunk_blocks_default(&blocks);
        assert_eq!(chunks.len(), 1, "small doc → one chunk");
        assert_eq!(chunks[0].page_start, 1);
        assert_eq!(chunks[0].page_end, 2);
        assert_eq!(chunks[0].bboxes.len(), 2, "one PageBoxes per page");
    }

    #[test]
    fn table_block_becomes_table_chunk() {
        let tree = serde_json::json!({
            "block_type": "Page",
            "id": "/page/2/Page/0",
            "children": [{
                "block_type": "Table",
                "html": "<table><tr><th>Year</th><th>N</th></tr><tr><td>2019</td><td>12</td></tr>\
                         <tr><td>2020</td><td>8</td></tr><tr><td>2021</td><td>15</td></tr></table>",
                "bbox": [0.0, 0.0, 100.0, 50.0]
            }]
        });
        let blocks = parse_marker_json(&tree);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].block_type, BlockType::Table);
        assert_eq!(blocks[0].page, 3, "page id /page/2/ → 1-indexed page 3");
        assert!(
            blocks[0]
                .text
                .starts_with("[TABLE] Page 3, 4 rows × 2 columns"),
            "got: {}",
            blocks[0].text
        );
        assert!(
            blocks[0].text.contains("| Year | N |"),
            "markdown header present"
        );
    }

    #[test]
    fn chunk_parity_matches_python_reference() {
        // Golden gate (S-1): these exact chunks were produced by the Python reference
        // `chunk_marker_json` on this fixture (verified via scripts/chunk_parity_check.py).
        // The Rust port must reproduce them — same text, same page_start/page_end, same
        // bbox pages. Clean case: no cross-page max-flush, so the documented page_end
        // correction does not diverge here.
        let big1 = "a".repeat(3600); // ~900 tokens > TARGET_MIN → heading boundary splits
        let tree = serde_json::json!({
            "block_type": "Document",
            "children": [
                {"block_type": "Page", "id": "/page/0/Page/0", "children": [
                    {"block_type": "Text", "html": format!("<p>{big1}</p>"), "bbox": [10, 20, 500, 400]},
                    {"block_type": "SectionHeader", "html": "<h2>Section Two</h2>", "bbox": [10, 410, 500, 440]}
                ]},
                {"block_type": "Page", "id": "/page/1/Page/1", "children": [
                    {"block_type": "Text", "html": "<p>short tail body</p>", "bbox": [10, 20, 500, 60]}
                ]}
            ]
        });
        let chunks = chunk_blocks_default(&parse_marker_json(&tree));
        assert_eq!(chunks.len(), 2, "reference produced 2 chunks");
        assert_eq!(chunks[0].page_start, 1);
        assert_eq!(chunks[0].page_end, 1);
        assert_eq!(chunks[0].text.len(), 3600);
        assert_eq!(
            chunks[0]
                .bboxes
                .iter()
                .map(|b| b.page_num)
                .collect::<Vec<_>>(),
            vec![1]
        );
        assert_eq!(chunks[1].page_start, 1);
        assert_eq!(chunks[1].page_end, 2);
        assert_eq!(chunks[1].text, "Section Two\n\nshort tail body");
        assert_eq!(
            chunks[1]
                .bboxes
                .iter()
                .map(|b| b.page_num)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn parses_sidecar_selftest_fixture() {
        // Golden gate for the Marker sidecar contract: the script's `--selftest` output
        // (checked-in fixture) must parse to the expected blocks. Regenerate the fixture with
        // `python3 scripts/archon_marker_sidecar.py --selftest --output <this file>`.
        let json = include_str!("../tests/fixtures/marker_selftest.json");
        let blocks = parse_marker_str(json).expect("fixture parses");
        // SectionHeader, Text, Table→[TABLE] (p1), Text, Text("1147a") (p2).
        assert_eq!(blocks.len(), 5);
        assert_eq!(blocks[0].block_type, BlockType::SectionHeader);
        assert_eq!(blocks[0].text, "On the Soul");
        assert_eq!(blocks[0].page, 1);
        assert_eq!(blocks[1].text, "The energeia of a living body.");
        assert_eq!(blocks[2].block_type, BlockType::Table);
        assert!(
            blocks[2].text.starts_with("[TABLE] Page 1"),
            "got: {}",
            blocks[2].text
        );
        assert_eq!(blocks[4].text, "1147a");
        assert_eq!(blocks[4].page, 2);
    }

    #[test]
    fn prose_false_positive_table_falls_back_to_text() {
        // A 2-row "table" fails is_real_table (rows < 3) → kept as prose text, not dropped.
        let tree = serde_json::json!({
            "block_type": "Page",
            "id": "/page/0/Page/0",
            "children": [{
                "block_type": "Table",
                "html": "<table><tr><td>just</td><td>prose</td></tr></table>",
                "bbox": [0.0, 0.0, 10.0, 10.0]
            }]
        });
        let blocks = parse_marker_json(&tree);
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0].block_type,
            BlockType::Text,
            "rejected table → text"
        );
        assert!(!blocks[0].text.starts_with("[TABLE]"));
    }
}
