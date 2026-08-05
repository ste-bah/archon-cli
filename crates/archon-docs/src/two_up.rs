//! 2-up (landscape spread) → logical book-page remapping (P2).
//!
//! A scanned journal/book article is often digitized "2-up": one physical PDF sheet holds TWO
//! book pages side by side (verso | recto). Marker assigns each block the PHYSICAL sheet number, so
//! a quote's cited "page" would be the sheet (e.g. sheet 1 = book pp.16|17) — ambiguous and wrong.
//!
//! This module remaps each block's `page` to its true book page BEFORE chunking, by:
//!   1. detecting the central gutter on each physical sheet (a wide horizontal gap near the content
//!      centre — self-contained, using the sheet's own block extent, no physical page-dims needed),
//!   2. assigning each block a side (left=verso, right=recto) by its x-centre vs the gutter, and
//!   3. numbering `book_page = first_page + 2·sheet_index + side`, where `first_page` is a per-doc
//!      seed (the book page of the first sheet's LEFT half) supplied in
//!      `.archon/two-up-first-pages.json`.
//!
//! Why a seed and not OCR: on real 2-up scans (verified on O'Gorman) the printed running-head page
//! numbers are frequently NOT captured by Marker, so the offset cannot be derived from the text. The
//! seed is a single number per doc (the scholar knows it), making the cite EXACT rather than guessed.
//! Side assignment is per-block, so it is correct even if a sheet's reading order interleaves columns.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use archon_ingest_ext::chunk::Block;

/// A gutter's gap must be at least this fraction of the sheet's content width to count as a page
/// seam (not ordinary inter-block spacing).
const MIN_GAP_FRAC: f32 = 0.03;
/// The gutter's centre must fall within this fraction of the content width from the content centre —
/// a 2-up seam sits near the middle of the spread, not at a column's outer margin.
const CENTRAL_FRAC: f32 = 0.15;
/// A sheet needs at least this many boxes to reason about columns.
const MIN_BLOCKS: usize = 4;
/// A candidate seam is only a real 2-up column break if BOTH sides carry at least this many blocks
/// (H-2): a lone right-hand block (a marginal note, a page number) must not fabricate a seam.
const MIN_SIDE_BLOCKS: usize = 2;
/// The sheet is split into this many horizontal bands to test column consistency down the page (H-2).
const GUTTER_Y_BANDS: usize = 3;
/// A candidate seam must show two-column structure (a block on EACH side) in at least this many
/// independent vertical bands (H-2). A one-off horizontal gap on a sparse single-column page — a
/// short heading, an indented list item, a clustered marginal note — corroborates in only one band
/// and is rejected. Kept at 2 so a minimal genuine 2-up sheet (two short columns) still qualifies.
const MIN_CORROBORATING_BANDS: usize = 2;
/// Doc-wide gate for the seeded remap (CR-4): a stray seed on a 1-up doc must NOT silently double
/// the page stride, so `remap_two_up` is applied only when a central gutter is detected on at least
/// this fraction of the doc's sheets. A genuine 2-up scan scores ~1.0; a single-column doc scores
/// ~0. `ARCHON_FORCE_TWO_UP` overrides the gate (see [`should_remap_two_up`]).
pub const MIN_TWO_UP_SHEET_FRACTION: f32 = 0.5;

/// Sentinel book page for a 2-up half that computes to <= 0 under a SIGNED `first_page` seed —
/// i.e. roman/unnumbered front matter in a book scanned 2-up (D1). 0 means "no clean arabic page".
/// NOTE: this is a FRESH sentinel — elsewhere the codebase defaults a missing page to 1, not 0 — so
/// consumers must be sentinel-aware. `quote_verify::build_location` clamps it out of a body quote's
/// page range so a body cite never collapses to "p.0". A quote that genuinely lands in front matter
/// still renders literally as "p.0" until the roman page-label work lands (D2) — see
/// plans/two-up-roman-front-matter-D2-TODO.md.
pub const FRONT_MATTER_SENTINEL: u32 = 0;

/// Diagnostics from a remap, for logging/validation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RemapDiag {
    pub sheets: usize,
    pub two_up_sheets: usize,
    pub min_page: u32,
    pub max_page: u32,
    /// Blocks that fell in front matter (computed page <= 0) → FRONT_MATTER_SENTINEL.
    pub front_matter_blocks: usize,
    /// Sheets in an otherwise-2-up doc where no gutter was detected, so verso/recto could not be
    /// resolved. Their blocks are marked unresolved (H-1) rather than folded onto the verso page.
    pub unresolved_sheets: usize,
    /// Blocks on `unresolved_sheets`, stamped `FRONT_MATTER_SENTINEL` instead of an off-by-one page.
    pub unresolved_blocks: usize,
}

/// The x of the central gutter on a sheet, from its block boxes (`[x0,y0,x1,y1]`), or `None` when
/// there is no clear central seam (a single-column sheet). Self-contained: thresholds are relative
/// to the sheet's own content extent, so no physical page width is needed.
// `!(width > 0.0)` intentionally also rejects NaN width; `<= 0.0` would not.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn detect_gutter(boxes: &[[f32; 4]]) -> Option<f32> {
    if boxes.len() < MIN_BLOCKS {
        return None;
    }
    let min_x0 = boxes.iter().map(|b| b[0]).fold(f32::INFINITY, f32::min);
    let max_x1 = boxes.iter().map(|b| b[2]).fold(f32::NEG_INFINITY, f32::max);
    let width = max_x1 - min_x0;
    if !(width > 0.0) {
        return None;
    }
    // Largest gap in the blocks' horizontal coverage (sweep by x0, tracking the running max x1).
    let mut intervals: Vec<(f32, f32)> = boxes.iter().map(|b| (b[0], b[2])).collect();
    intervals.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut running_max = intervals[0].1;
    let mut best_gap = 0.0f32;
    let mut gutter = None;
    for &(x0, x1) in &intervals[1..] {
        if x0 > running_max {
            let gap = x0 - running_max;
            if gap > best_gap {
                best_gap = gap;
                gutter = Some((running_max + x0) / 2.0);
            }
        }
        running_max = running_max.max(x1);
    }
    let gutter = gutter?;
    let centre = min_x0 + width / 2.0;
    let central = (gutter - centre).abs() <= CENTRAL_FRAC * width;
    if !(best_gap >= MIN_GAP_FRAC * width && central) {
        return None;
    }
    // H-2: a single global x-gap is not enough — a sparse single-column page (verse, Q&A, a list,
    // a clustered marginal note) can show a central gap that is NOT a page seam. Corroborate the
    // candidate seam before trusting it:
    //   1. both columns must carry enough blocks (MIN_SIDE_BLOCKS each), and
    //   2. the two-column structure must recur down the page across independent vertical bands.
    let side_of = |b: &[f32; 4]| (b[0] + b[2]) / 2.0 >= gutter; // true = right (recto)
    let right_blocks = boxes.iter().filter(|b| side_of(b)).count();
    let left_blocks = boxes.len() - right_blocks;
    if left_blocks < MIN_SIDE_BLOCKS || right_blocks < MIN_SIDE_BLOCKS {
        return None;
    }
    let min_y = boxes.iter().map(|b| b[1]).fold(f32::INFINITY, f32::min);
    let max_y = boxes.iter().map(|b| b[3]).fold(f32::NEG_INFINITY, f32::max);
    let height = max_y - min_y;
    if height > 0.0 {
        let band_h = height / GUTTER_Y_BANDS as f32;
        let mut left_in_band = [false; GUTTER_Y_BANDS];
        let mut right_in_band = [false; GUTTER_Y_BANDS];
        for b in boxes {
            let yc = (b[1] + b[3]) / 2.0;
            let band = (((yc - min_y) / band_h) as usize).min(GUTTER_Y_BANDS - 1);
            if side_of(b) {
                right_in_band[band] = true;
            } else {
                left_in_band[band] = true;
            }
        }
        let corroborating = (0..GUTTER_Y_BANDS)
            .filter(|&i| left_in_band[i] && right_in_band[i])
            .count();
        if corroborating < MIN_CORROBORATING_BANDS {
            return None;
        }
    }
    Some(gutter)
}

/// Remap each block's `page` from its physical sheet to its book page, using `first_page` (the book
/// page of the first sheet's LEFT half). Sheets are numbered by their sorted physical order, so a
/// doc whose first physical page isn't 1 still maps correctly. A sheet with no detectable gutter has
/// an UNRESOLVED side: its blocks are stamped `FRONT_MATTER_SENTINEL` (counted in
/// `unresolved_sheets`/`unresolved_blocks`) rather than folded onto the verso page — folding recto
/// content to verso would cite it as the page BEFORE the one it is printed on (H-1). The sheet still
/// consumes its two book-page slots in the stride, so later sheets keep their numbering.
///
/// `first_page` is SIGNED (D1): a book scanned 2-up with roman front matter needs a NEGATIVE seed
/// so its arabic body (starting at a low page deep in the PDF) maps correctly; the front-matter
/// halves then compute to <= 0 and are stamped `FRONT_MATTER_SENTINEL` (no clean arabic cite). The
/// arabic body always yields positive `u32` pages, so nothing negative is ever stored.
pub fn remap_two_up(mut blocks: Vec<Block>, first_page: i32) -> (Vec<Block>, RemapDiag) {
    // Sorted, de-duplicated physical sheet numbers → their 0-based index.
    let mut sheets: Vec<u32> = blocks.iter().map(|b| b.page).collect();
    sheets.sort_unstable();
    sheets.dedup();
    let sheet_index: BTreeMap<u32, usize> =
        sheets.iter().enumerate().map(|(i, p)| (*p, i)).collect();

    // Per-sheet gutter (computed once over that sheet's boxes).
    let mut gutter_of: BTreeMap<u32, Option<f32>> = BTreeMap::new();
    for &sheet in &sheets {
        let boxes: Vec<[f32; 4]> = blocks
            .iter()
            .filter(|b| b.page == sheet)
            .map(|b| b.bbox)
            .collect();
        gutter_of.insert(sheet, detect_gutter(&boxes));
    }

    let mut diag = RemapDiag {
        sheets: sheets.len(),
        two_up_sheets: gutter_of.values().filter(|g| g.is_some()).count(),
        min_page: u32::MAX,
        max_page: 0,
        front_matter_blocks: 0,
        unresolved_sheets: gutter_of.values().filter(|g| g.is_none()).count(),
        unresolved_blocks: 0,
    };

    for b in &mut blocks {
        match gutter_of[&b.page] {
            Some(g) => {
                let idx = sheet_index[&b.page] as i32;
                let side: i32 = if (b.bbox[0] + b.bbox[2]) / 2.0 >= g {
                    1
                } else {
                    0
                }; // right = recto
                let signed = first_page + 2 * idx + side;
                if signed >= 1 {
                    let page = signed as u32;
                    b.page = page;
                    diag.min_page = diag.min_page.min(page);
                    diag.max_page = diag.max_page.max(page);
                } else {
                    // Front matter (roman / unnumbered) under a negative seed — no clean arabic page.
                    b.page = FRONT_MATTER_SENTINEL;
                    diag.front_matter_blocks += 1;
                }
            }
            // H-1: no gutter on this sheet → verso/recto is unknown. Marking the block unresolved
            // (a loud sentinel) is safer than folding recto content onto the verso page, which would
            // silently cite it one page early.
            None => {
                b.page = FRONT_MATTER_SENTINEL;
                diag.unresolved_blocks += 1;
            }
        }
    }
    if diag.unresolved_sheets > 0 {
        tracing::warn!(
            unresolved_sheets = diag.unresolved_sheets,
            unresolved_blocks = diag.unresolved_blocks,
            sentinel = FRONT_MATTER_SENTINEL,
            "2-up remap: {} sheet(s) had no detectable gutter; their blocks are marked unresolved \
             (page {}) rather than folded onto the verso page (H-1)",
            diag.unresolved_sheets,
            FRONT_MATTER_SENTINEL
        );
    }
    if diag.min_page == u32::MAX {
        // No positively-numbered block (empty doc, or a doc that is all front matter).
        diag.min_page = 0;
    }
    (blocks, diag)
}

/// Physical-sheet → book-page map that reproduces `remap_two_up`'s exact sheet ordering, used to
/// translate image-enrichment chunks (D3) — which carry the physical sheet — onto book pages on a
/// 2-up doc. Verso/left (`side = 0`) is used because a `PdfImage` has no sheet-coordinate bbox, so
/// a figure can only be attributed to its sheet's verso page; a figure on the recto half is cited
/// one page early (B6, warned once per doc by the enrichment caller — not fixable without per-image
/// coordinates). Must be called on the SAME pre-remap `blocks` the text remap indexes, so image
/// pages stay aligned with text pages. Front-matter sheets (computed page <= 0) map to
/// `FRONT_MATTER_SENTINEL`. Only sheets that carry TEXT blocks appear here; a plate/zero-OCR sheet
/// is absent, so the enrichment caller must resolve it by interpolation, NOT by the raw physical
/// sheet index (a different numbering domain) — see `pdf_image_enrichment::resolve_image_book_page`
/// (B7).
pub fn sheet_book_page_map(blocks: &[Block], first_page: i32) -> BTreeMap<u32, u32> {
    let mut sheets: Vec<u32> = blocks.iter().map(|b| b.page).collect();
    sheets.sort_unstable();
    sheets.dedup();
    sheets
        .iter()
        .enumerate()
        .map(|(i, &sheet)| {
            let signed = first_page + 2 * i as i32; // verso (side = 0)
            let book = if signed >= 1 {
                signed as u32
            } else {
                FRONT_MATTER_SENTINEL
            };
            (sheet, book)
        })
        .collect()
}

/// Whether a block stream (pre-remap, physical sheets) LOOKS 2-up — its first sheet has a gutter.
/// Used to warn when a doc is likely 2-up but has no first-page seed.
pub fn looks_two_up(blocks: &[Block]) -> bool {
    let Some(first) = blocks.iter().map(|b| b.page).min() else {
        return false;
    };
    let boxes: Vec<[f32; 4]> = blocks
        .iter()
        .filter(|b| b.page == first)
        .map(|b| b.bbox)
        .collect();
    detect_gutter(&boxes).is_some()
}

/// Fraction of a doc's physical sheets that carry a detectable central gutter — the doc-wide
/// "is this genuinely 2-up?" signal (CR-4). Unlike [`looks_two_up`] (first sheet only, which a
/// leading title/blank/plate sheet would defeat), this samples every sheet, so a real 2-up scan
/// scores near 1.0 while a single-column doc that was mistakenly given a seed scores ~0. A handful
/// of sparse sheets that fail gutter detection (H-1/H-2) only nudge the score down.
pub fn two_up_sheet_fraction(blocks: &[Block]) -> f32 {
    let mut sheets: Vec<u32> = blocks.iter().map(|b| b.page).collect();
    sheets.sort_unstable();
    sheets.dedup();
    if sheets.is_empty() {
        return 0.0;
    }
    let with_gutter = sheets
        .iter()
        .filter(|&&sheet| {
            let boxes: Vec<[f32; 4]> = blocks
                .iter()
                .filter(|b| b.page == sheet)
                .map(|b| b.bbox)
                .collect();
            detect_gutter(&boxes).is_some()
        })
        .count();
    with_gutter as f32 / sheets.len() as f32
}

/// Gate for the seeded 2-up remap (CR-4): apply the remap only when the doc genuinely LOOKS 2-up
/// (a central gutter on at least [`MIN_TWO_UP_SHEET_FRACTION`] of its sheets), unless `force`
/// overrides. A stray seed on a 1-up doc would otherwise silently double the page stride —
/// dropping every even physical page from the citable range and mis-citing every odd one. Pure and
/// unit-testable; the caller supplies `force` (e.g. `ARCHON_FORCE_TWO_UP`).
pub fn should_remap_two_up(blocks: &[Block], force: bool) -> bool {
    force || two_up_sheet_fraction(blocks) >= MIN_TWO_UP_SHEET_FRACTION
}

/// Resolve the two-up seed file: `$ARCHON_TWO_UP_SEED_FILE`, else `.archon/two-up-first-pages.json`.
pub fn seed_map_path() -> PathBuf {
    if let Some(p) = std::env::var_os("ARCHON_TWO_UP_SEED_FILE") {
        return PathBuf::from(p);
    }
    PathBuf::from(".archon/two-up-first-pages.json")
}

/// Load the `{ "<basename>": <first_page>, … }` seed map. Missing/malformed file → empty map
/// (no doc is remapped), so a normal single-page corpus needs no config. Values are SIGNED (D1):
/// a positive seed is an offprint (the book page of the first sheet's left half, e.g. O'Gorman:16);
/// a negative seed maps the arabic body of a front-matter book (existing positive entries still
/// parse unchanged).
pub fn load_seed_map(path: &Path) -> BTreeMap<String, i32> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// The first-page seed for a document by its file path's basename, if any.
pub fn first_page_for(map: &BTreeMap<String, i32>, file_path: &str) -> Option<i32> {
    let base = Path::new(file_path).file_name()?.to_str()?;
    map.get(base).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_ingest_ext::chunk::BlockType;

    fn blk(page: u32, bbox: [f32; 4]) -> Block {
        Block {
            block_type: BlockType::Text,
            text: "x".into(),
            bbox,
            page,
        }
    }

    #[test]
    fn detects_a_central_gutter() {
        // Left column x[65,410], right column x[425,778] — a clear seam near the centre.
        let boxes = [
            [65.0, 60.0, 410.0, 120.0],
            [66.0, 130.0, 405.0, 250.0],
            [457.0, 70.0, 778.0, 140.0],
            [458.0, 150.0, 777.0, 400.0],
        ];
        let g = detect_gutter(&boxes).expect("gutter");
        assert!(g > 410.0 && g < 457.0, "gutter between the columns: {g}");
    }

    #[test]
    fn single_column_has_no_gutter() {
        let boxes = [
            [65.0, 60.0, 410.0, 120.0],
            [66.0, 130.0, 405.0, 250.0],
            [64.0, 260.0, 408.0, 400.0],
            [67.0, 410.0, 409.0, 500.0],
        ];
        assert!(detect_gutter(&boxes).is_none());
    }

    // H-2: a single-column page with ONE isolated far-right block (a page number / marginal note)
    // opens a wide central x-gap that the old single-projection detector accepted as a seam. The
    // "min blocks on both sides" guard rejects it (only one block sits right of the candidate seam).
    #[test]
    fn isolated_right_block_is_not_a_gutter() {
        let boxes = [
            [50.0, 60.0, 200.0, 90.0],   // left column
            [50.0, 100.0, 210.0, 130.0], // left column
            [50.0, 140.0, 205.0, 170.0], // left column
            [50.0, 180.0, 208.0, 210.0], // left column
            [600.0, 60.0, 750.0, 90.0],  // one lone right-hand block
        ];
        // Sanity: the raw gap IS central & wide (this is what the old detector keyed on).
        assert!(
            detect_gutter(&boxes).is_none(),
            "one right block must not fabricate a seam"
        );
    }

    // H-2: a left column spanning the whole page plus a right-hand cluster confined to the TOP
    // (e.g. a header note or a top-corner figure) passes the "≥2 blocks each side" check but shows
    // two-column structure in only ONE vertical band. The y-band corroboration guard rejects it.
    #[test]
    fn top_clustered_right_notes_are_not_a_gutter() {
        let boxes = [
            [50.0, 60.0, 300.0, 100.0],   // left, top
            [50.0, 200.0, 300.0, 240.0],  // left, middle
            [50.0, 340.0, 300.0, 380.0],  // left, bottom
            [600.0, 60.0, 750.0, 90.0],   // right, top only
            [600.0, 110.0, 750.0, 140.0], // right, top only
        ];
        assert!(
            detect_gutter(&boxes).is_none(),
            "right content confined to one band must not corroborate a full-height seam"
        );
    }

    // CR-4: a seeded doc that is actually SINGLE-column must NOT be remapped — the gate skips it so
    // the page stride is never silently doubled. `should_remap_two_up` is the pure decision the
    // ingest pipeline consults before calling `remap_two_up`.
    #[test]
    fn seeded_single_column_doc_skips_remap() {
        // Three single-column sheets (no central gutter on any).
        let col = |p: u32| {
            vec![
                blk(p, [60.0, 60.0, 520.0, 110.0]),
                blk(p, [60.0, 130.0, 515.0, 200.0]),
                blk(p, [60.0, 220.0, 518.0, 300.0]),
                blk(p, [60.0, 320.0, 516.0, 400.0]),
            ]
        };
        let mut blocks = Vec::new();
        blocks.extend(col(1));
        blocks.extend(col(2));
        blocks.extend(col(3));
        assert!(two_up_sheet_fraction(&blocks) < MIN_TWO_UP_SHEET_FRACTION);
        assert!(
            !should_remap_two_up(&blocks, false),
            "1-up doc must skip the seeded remap"
        );
        // The escape hatch still forces it when the scholar knows better.
        assert!(
            should_remap_two_up(&blocks, true),
            "ARCHON_FORCE_TWO_UP overrides the gate"
        );
    }

    // CR-4: a genuinely 2-up doc clears the gate.
    #[test]
    fn seeded_two_up_doc_passes_gate() {
        let sheet = |p: u32| {
            vec![
                blk(p, [65.0, 60.0, 410.0, 120.0]),   // left
                blk(p, [66.0, 130.0, 405.0, 250.0]),  // left
                blk(p, [457.0, 70.0, 778.0, 140.0]),  // right
                blk(p, [458.0, 150.0, 777.0, 400.0]), // right
            ]
        };
        let mut blocks = Vec::new();
        blocks.extend(sheet(1));
        blocks.extend(sheet(2));
        assert!(two_up_sheet_fraction(&blocks) >= MIN_TWO_UP_SHEET_FRACTION);
        assert!(should_remap_two_up(&blocks, false));
    }

    // H-1: a genuine 2-up doc with ONE gutter-undetected sheet must mark that sheet unresolved
    // (sentinel) rather than fold its recto content onto the verso page.
    #[test]
    fn gutter_undetected_sheet_is_unresolved_not_verso() {
        let two_up_sheet = |p: u32| {
            vec![
                blk(p, [65.0, 60.0, 410.0, 120.0]),   // left  (verso)
                blk(p, [66.0, 130.0, 405.0, 250.0]),  // left
                blk(p, [457.0, 70.0, 778.0, 140.0]),  // right (recto)
                blk(p, [458.0, 150.0, 777.0, 400.0]), // right
            ]
        };
        let mut blocks = two_up_sheet(1);
        // Sheet 2: a single-column (no gutter) sheet embedded in the 2-up doc — one recto-side and
        // one verso-side block, but no detectable seam.
        blocks.push(blk(2, [60.0, 60.0, 520.0, 110.0]));
        blocks.push(blk(2, [60.0, 130.0, 515.0, 200.0]));
        let (out, diag) = remap_two_up(blocks, 16);
        // Sheet 1 resolves normally: verso 16, recto 17.
        assert_eq!(out[0].page, 16);
        assert_eq!(out[2].page, 17);
        // Sheet 2's blocks are UNRESOLVED (sentinel), NOT folded to verso page 18.
        assert_eq!(out[4].page, FRONT_MATTER_SENTINEL);
        assert_eq!(out[5].page, FRONT_MATTER_SENTINEL);
        assert_eq!(diag.unresolved_sheets, 1);
        assert_eq!(diag.unresolved_blocks, 2);
        assert_eq!(diag.two_up_sheets, 1);
    }

    #[test]
    fn remaps_two_sheets_to_four_book_pages() {
        // sheet 1: 2 left + 1 right; sheet 2: 1 left + 1 right. first_page=16.
        let blocks = vec![
            blk(1, [65.0, 60.0, 410.0, 120.0]),   // s1 left  → 16
            blk(1, [66.0, 130.0, 405.0, 250.0]),  // s1 left  → 16
            blk(1, [457.0, 70.0, 778.0, 140.0]),  // s1 right → 17
            blk(1, [458.0, 150.0, 777.0, 400.0]), // s1 right → 17
            blk(2, [65.0, 60.0, 410.0, 120.0]),   // s2 left  → 18
            blk(2, [66.0, 130.0, 405.0, 250.0]),  // s2 left  → 18
            blk(2, [457.0, 70.0, 778.0, 140.0]),  // s2 right → 19
            blk(2, [458.0, 150.0, 777.0, 400.0]), // s2 right → 19
        ];
        let (out, diag) = remap_two_up(blocks, 16);
        let pages: Vec<u32> = out.iter().map(|b| b.page).collect();
        assert_eq!(pages, vec![16, 16, 17, 17, 18, 18, 19, 19]);
        assert_eq!(diag.two_up_sheets, 2);
        assert_eq!((diag.min_page, diag.max_page), (16, 19));
    }

    #[test]
    fn side_is_per_block_not_reading_order() {
        // Interleaved reading order (L,R,L,R) must still assign correct sides.
        let blocks = vec![
            blk(1, [65.0, 60.0, 410.0, 120.0]),   // left  → 16
            blk(1, [457.0, 70.0, 778.0, 140.0]),  // right → 17
            blk(1, [66.0, 130.0, 405.0, 250.0]),  // left  → 16
            blk(1, [458.0, 150.0, 777.0, 400.0]), // right → 17
        ];
        let (out, _) = remap_two_up(blocks, 16);
        assert_eq!(
            out.iter().map(|b| b.page).collect::<Vec<_>>(),
            vec![16, 17, 16, 17]
        );
    }

    #[test]
    fn seed_lookup_by_basename() {
        let mut map = BTreeMap::new();
        map.insert("Foo (2005) [My Copy].pdf".to_string(), 16i32);
        assert_eq!(
            first_page_for(&map, "/abs/path/Foo (2005) [My Copy].pdf"),
            Some(16)
        );
        assert_eq!(first_page_for(&map, "/abs/path/Other.pdf"), None);
    }

    #[test]
    fn negative_seed_maps_body_and_sentinels_front_matter() {
        // A front-matter book, seed -2: sheet idx0 = [-2|-1] → all sentinel; idx1 = [0|1] → left
        // sentinel, right = body p.1; idx2 = [2|3]. Body is always positive; nothing negative is
        // ever stored. Four blocks per sheet so detect_gutter (needs >=4) finds the seam.
        let sheet = |p: u32| {
            vec![
                blk(p, [65.0, 60.0, 410.0, 120.0]),   // left
                blk(p, [66.0, 130.0, 405.0, 250.0]),  // left
                blk(p, [457.0, 70.0, 778.0, 140.0]),  // right
                blk(p, [458.0, 150.0, 777.0, 400.0]), // right
            ]
        };
        let mut blocks = Vec::new();
        blocks.extend(sheet(1));
        blocks.extend(sheet(2));
        blocks.extend(sheet(3));
        let (out, diag) = remap_two_up(blocks, -2);
        let pages: Vec<u32> = out.iter().map(|b| b.page).collect();
        assert_eq!(pages, vec![0, 0, 0, 0, 0, 0, 1, 1, 2, 2, 3, 3]);
        assert_eq!(diag.front_matter_blocks, 6);
        assert_eq!((diag.min_page, diag.max_page), (1, 3));
    }

    #[test]
    fn sheet_book_page_map_matches_remap_verso() {
        let blocks = vec![
            blk(1, [65.0, 60.0, 410.0, 120.0]),
            blk(2, [65.0, 60.0, 410.0, 120.0]),
            blk(3, [65.0, 60.0, 410.0, 120.0]),
        ];
        // Negative seed: sheet 1 → verso -2 → sentinel 0; sheet 2 → 0 → sentinel 0; sheet 3 → 2.
        let m = sheet_book_page_map(&blocks, -2);
        assert_eq!(m.get(&1), Some(&0));
        assert_eq!(m.get(&2), Some(&0));
        assert_eq!(m.get(&3), Some(&2));
        // Positive seed (offprint): verso pages 16, 18, 20.
        let m2 = sheet_book_page_map(&blocks, 16);
        assert_eq!(m2.get(&1), Some(&16));
        assert_eq!(m2.get(&2), Some(&18));
        assert_eq!(m2.get(&3), Some(&20));
    }

    // Real-data proof against the O'Gorman Marker fixture (13 landscape 2-up sheets). Run with:
    //   cargo test -p archon-docs -- --ignored ogorman_fixture
    #[test]
    #[ignore = "needs the O'Gorman marker fixture at .tmp/ogorman-marker.json"]
    fn ogorman_fixture_remaps_to_book_pages_16_41() {
        let path = std::env::var("ARCHON_OGORMAN_MARKER_JSON")
            .unwrap_or_else(|_| ".tmp/ogorman-marker.json".to_string());
        let json = std::fs::read_to_string(&path).expect("read O'Gorman marker fixture");
        let blocks = archon_ingest_ext::marker::parse_marker_str(&json).expect("parse marker json");
        assert!(!blocks.is_empty(), "fixture yields blocks");

        let mut physical: Vec<u32> = blocks.iter().map(|b| b.page).collect();
        physical.sort_unstable();
        physical.dedup();
        assert_eq!(*physical.first().unwrap(), 1, "physical sheets start at 1");
        assert_eq!(*physical.last().unwrap(), 13, "13 physical sheets");

        let (out, diag) = remap_two_up(blocks, 16);
        assert_eq!(diag.sheets, 13);
        assert_eq!(diag.two_up_sheets, 13, "all 13 sheets detected as 2-up");
        assert_eq!(
            (diag.min_page, diag.max_page),
            (16, 41),
            "13 sheets → book pages 16..41"
        );
        assert!(
            out.iter().all(|b| (16..=41).contains(&b.page)),
            "every block maps into 16..=41"
        );
        // verso=even(left), recto=odd(right): a left-column block on sheet 1 → 16, right → 17.
        assert!(
            out.iter()
                .any(|b| b.page == 16 && (b.bbox[0] + b.bbox[2]) / 2.0 < 430.0),
            "a left-column sheet-1 block maps to book p.16"
        );
        assert!(
            out.iter()
                .any(|b| b.page == 17 && (b.bbox[0] + b.bbox[2]) / 2.0 > 430.0),
            "a right-column sheet-1 block maps to book p.17"
        );
    }

    // End-to-end: ingest the O'Gorman fixture (remap → chunk → persist), then verify a verso quote
    // cites book p.16 and a recto quote cites p.17 — even though they share a chunk that spans 16→17.
    // Run with: cargo test -p archon-docs -- --ignored ogorman_end_to_end --nocapture
    #[test]
    #[ignore = "needs the O'Gorman marker fixture at .tmp/ogorman-marker.json"]
    fn ogorman_end_to_end_subspan_page_cites() {
        use crate::block_chunking::{COORD_MARKER, persist_block_chunks};
        use crate::quote_verify::find_fragment_bboxes;

        let path = std::env::var("ARCHON_OGORMAN_MARKER_JSON")
            .unwrap_or_else(|_| ".tmp/ogorman-marker.json".to_string());
        let json = std::fs::read_to_string(&path).expect("read fixture");
        let blocks = archon_ingest_ext::marker::parse_marker_str(&json).expect("parse marker json");
        let (out, _diag) = remap_two_up(blocks, 16);

        let dbpath = format!("/tmp/test-2up-e2e-{}.db", uuid::Uuid::new_v4());
        let db = cozo::DbInstance::new("sqlite", &dbpath, "").unwrap();
        crate::schema::ensure_doc_schema(&db).unwrap();
        let (chunks, _spatials) = persist_block_chunks(
            &db,
            "ogorman",
            "ocr-ogorman",
            "ocr_text",
            &out,
            COORD_MARKER,
            false,
        )
        .unwrap();

        // A real 2-up chunk spans book pages 16→17 (verso+recto merged), so coarse page ranges would
        // report "16-17" for BOTH quotes — the sub-span narrowing is what makes each cite exact.
        assert!(
            chunks
                .iter()
                .any(|c| c.page_start == 16 && c.page_end == 17),
            "a chunk spans book pages 16→17 (so sub-span narrowing is what's under test)"
        );

        let p16 = out
            .iter()
            .find(|b| b.page == 16 && b.text.len() > 200)
            .expect("a substantial p16 block");
        let p17 = out
            .iter()
            .find(|b| b.page == 17 && b.text.len() > 200)
            .expect("a substantial p17 block");
        let needle16: String = p16.text.chars().skip(20).take(50).collect();
        let needle17: String = p17.text.chars().skip(20).take(50).collect();

        let loc16 = find_fragment_bboxes(&db, "ogorman", &needle16)
            .unwrap()
            .expect("verso quote located");
        assert_eq!(
            (loc16.page_start, loc16.page_end),
            (16, 16),
            "verso quote cites book p.16 (not 16-17): {needle16:?}"
        );
        assert!((loc16.similarity - 1.0).abs() < 1e-9, "exact-1.00 match");
        let bb16 = loc16
            .fragments
            .iter()
            .find_map(|f| f.bbox)
            .expect("p16 sentence bbox");
        assert!(bb16[2] <= 435.0, "verso box in the LEFT column: {bb16:?}");

        let loc17 = find_fragment_bboxes(&db, "ogorman", &needle17)
            .unwrap()
            .expect("recto quote located");
        assert_eq!(
            (loc17.page_start, loc17.page_end),
            (17, 17),
            "recto quote cites book p.17 (not 16-17): {needle17:?}"
        );
        let bb17 = loc17
            .fragments
            .iter()
            .find_map(|f| f.bbox)
            .expect("p17 sentence bbox");
        assert!(bb17[0] >= 435.0, "recto box in the RIGHT column: {bb17:?}");

        let _ = std::fs::remove_file(&dbpath);
    }
}
