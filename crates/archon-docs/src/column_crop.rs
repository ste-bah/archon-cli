//! 2-up column cropping for the `/curate` page-image viewer.
//!
//! A token-budgeted chunk's `super_box` is the axis-aligned UNION of every block box the chunk
//! occupies on a page. On a **2-up scanned source** (two book pages side-by-side per physical PDF
//! page) a chunk whose reading order crosses the gutter — the bottom of the left page-half into the
//! top of the right — has a `super_box` that spans BOTH columns, so cropping to it yields the whole
//! spread instead of the quote.
//!
//! The per-block boxes ARE persisted (`doc_chunk_spatial.blocks`), so we can do better: cluster the
//! blocks into columns by the horizontal gutter gap and crop to the single column that holds most of
//! the chunk. On an ordinary single-column page (no central gutter) this returns `None` and the
//! caller keeps the plain `super_box` crop.
//!
//! Coordinates are Marker-space PDF **points, top-left origin** — the same space `crop_region` maps
//! pt→px with `scale = dpi/72`, no y-flip.

/// A chunk must span more than this fraction of the page width to be a candidate 2-up spread; a
/// single body column is narrower, so its `super_box` is already tight and needs no refinement.
const SPREAD_MIN_FRAC: f32 = 0.55;
/// The gutter's mid-point must fall in this central band `[lo, hi]` of the page width — a 2-up seam
/// sits near the physical page center, not at a single column's outer margin.
const GUTTER_BAND: (f32, f32) = (0.35, 0.65);
/// A gutter gap must be at least this fraction of the page width, so ordinary inter-block spacing is
/// not mistaken for a page seam.
const MIN_GAP_FRAC: f32 = 0.02;

/// Per-block boxes a chunk occupies on `page`, parsed from the persisted `doc_chunk_spatial.blocks`
/// JSON (`[{"page_num", "super_box":[..], "blocks":[[x0,y0,x1,y1],..]}, ..]`). Empty when the JSON
/// is malformed or has no entry for `page`.
pub fn parse_page_block_boxes(blocks_json: &str, page: u32) -> Vec<[f32; 4]> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(blocks_json) else {
        return Vec::new();
    };
    let Some(pages) = value.as_array() else {
        return Vec::new();
    };
    for entry in pages {
        if entry.get("page_num").and_then(|p| p.as_u64()) != Some(page as u64) {
            continue;
        }
        let Some(blocks) = entry.get("blocks").and_then(|b| b.as_array()) else {
            return Vec::new();
        };
        return blocks.iter().filter_map(parse_box4).collect();
    }
    Vec::new()
}

fn parse_box4(v: &serde_json::Value) -> Option<[f32; 4]> {
    let a = v.as_array()?;
    if a.len() != 4 {
        return None;
    }
    let mut out = [0f32; 4];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = a[i].as_f64()? as f32;
    }
    Some(out)
}

/// The tighter crop bbox for a 2-up spread: cluster `block_boxes` into two page-halves at the gutter
/// and return the union of the denser half (most block area). `None` when there is no clear central
/// gutter — a single-column page — so the caller keeps the chunk `super_box`.
///
/// `page_width` is the physical PDF page width in points (derived from the render).
// `!(page_width > 0.0)` intentionally also rejects NaN width; `<= 0.0` would not.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn column_crop_bbox(block_boxes: &[[f32; 4]], page_width: f32) -> Option<[f32; 4]> {
    if block_boxes.len() < 2 || !(page_width > 0.0) {
        return None;
    }
    let super_box = merge_boxes(block_boxes)?;
    // Not wide enough to be two page-halves: the super_box is already a single column.
    if (super_box[2] - super_box[0]) <= SPREAD_MIN_FRAC * page_width {
        return None;
    }
    let gutter = largest_central_gap(block_boxes, page_width)?;
    // Split at the gutter and keep the side that carries the most block area (where the quote is).
    let (mut left, mut right): (Vec<[f32; 4]>, Vec<[f32; 4]>) = (Vec::new(), Vec::new());
    for &b in block_boxes {
        if (b[0] + b[2]) / 2.0 < gutter {
            left.push(b);
        } else {
            right.push(b);
        }
    }
    let denser = if area(&right) > area(&left) {
        &right
    } else {
        &left
    };
    merge_boxes(denser)
}

/// The x-center of the widest horizontal gap in the blocks' x-coverage, if that gap is wide enough
/// and sits in the central band (a 2-up gutter). `None` otherwise.
fn largest_central_gap(block_boxes: &[[f32; 4]], page_width: f32) -> Option<f32> {
    let mut intervals: Vec<(f32, f32)> = block_boxes.iter().map(|b| (b[0], b[2])).collect();
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
    if best_gap < MIN_GAP_FRAC * page_width
        || gutter < GUTTER_BAND.0 * page_width
        || gutter > GUTTER_BAND.1 * page_width
    {
        return None;
    }
    Some(gutter)
}

fn area(boxes: &[[f32; 4]]) -> f32 {
    boxes
        .iter()
        .map(|b| (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0))
        .sum()
}

/// Axis-aligned union (min x0, min y0, max x1, max y1). `None` for an empty slice.
fn merge_boxes(boxes: &[[f32; 4]]) -> Option<[f32; 4]> {
    let mut it = boxes.iter();
    let mut r = *it.next()?;
    for b in it {
        r[0] = r[0].min(b[0]);
        r[1] = r[1].min(b[1]);
        r[2] = r[2].max(b[2]);
        r[3] = r[3].max(b[3]);
    }
    Some(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_per_page_block_boxes() {
        let json = r#"[
            {"page_num":168,"super_box":[50.0,50.0,550.0,500.0],"blocks":[[50.0,50.0,290.0,100.0],[310.0,60.0,550.0,120.0]]},
            {"page_num":169,"super_box":[50.0,50.0,290.0,300.0],"blocks":[[50.0,50.0,290.0,300.0]]}
        ]"#;
        let p168 = parse_page_block_boxes(json, 168);
        assert_eq!(p168.len(), 2);
        assert_eq!(p168[0], [50.0, 50.0, 290.0, 100.0]);
        let p169 = parse_page_block_boxes(json, 169);
        assert_eq!(p169.len(), 1);
        assert!(parse_page_block_boxes(json, 999).is_empty());
        assert!(parse_page_block_boxes("not json", 1).is_empty());
    }

    #[test]
    fn clean_2up_gutter_crossing_returns_one_column() {
        // 600pt-wide page; left book-page column ends ~290, right begins ~310 (gutter at 300).
        // The chunk crosses the gutter: two blocks at the BOTTOM of the left half, one at the TOP of
        // the right half. The super_box would span [50..550] (the whole spread) — we want one column.
        let blocks = [
            [50.0, 400.0, 290.0, 450.0], // left, bottom
            [50.0, 460.0, 290.0, 500.0], // left, bottom
            [310.0, 50.0, 550.0, 100.0], // right, top
        ];
        let col = column_crop_bbox(&blocks, 600.0).expect("gutter detected");
        // Denser side is the left (two blocks) → crop stays within the left page-half.
        assert!(col[2] <= 300.0, "column must not cross the gutter: {col:?}");
        assert_eq!(col, [50.0, 400.0, 290.0, 500.0]);
    }

    #[test]
    fn right_column_denser_picks_right() {
        let blocks = [
            [50.0, 400.0, 290.0, 450.0],  // left, one small block
            [310.0, 50.0, 550.0, 120.0],  // right, taller
            [310.0, 130.0, 550.0, 260.0], // right, taller
        ];
        let col = column_crop_bbox(&blocks, 600.0).expect("gutter detected");
        assert!(col[0] >= 300.0, "should pick the right half: {col:?}");
        assert_eq!(col, [310.0, 50.0, 550.0, 260.0]);
    }

    #[test]
    fn single_narrow_column_returns_none() {
        // All blocks inside one book-page column (~240pt) on a 600pt page → super_box already tight.
        let blocks = [[50.0, 100.0, 290.0, 150.0], [50.0, 160.0, 290.0, 220.0]];
        assert!(column_crop_bbox(&blocks, 600.0).is_none());
    }

    #[test]
    fn wide_single_column_without_central_gap_returns_none() {
        // Full-width single-column body text (not a 2-up): wide, but no central gutter gap.
        let blocks = [
            [50.0, 100.0, 550.0, 120.0],
            [50.0, 130.0, 550.0, 150.0],
            [50.0, 160.0, 550.0, 180.0],
        ];
        assert!(column_crop_bbox(&blocks, 600.0).is_none());
    }

    #[test]
    fn degenerate_inputs_return_none() {
        assert!(column_crop_bbox(&[], 600.0).is_none());
        assert!(column_crop_bbox(&[[50.0, 50.0, 550.0, 100.0]], 600.0).is_none());
        assert!(
            column_crop_bbox(
                &[[50.0, 50.0, 290.0, 100.0], [310.0, 50.0, 550.0, 100.0]],
                0.0
            )
            .is_none()
        );
    }
}
