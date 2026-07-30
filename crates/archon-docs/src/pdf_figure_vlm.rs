//! C4 — figure-region VLM (opt-in, `[policy.docs.pdf] figure_region_vlm`).
//!
//! Scanned books bake figures INTO the page scans, so there is no discrete embedded image for the
//! standard VLM path to describe — and the scanned-book guard skips the page scans anyway. This
//! path instead takes Marker's detected **figure regions** (page + bbox), renders each page, crops
//! the figure out of the render, and VLM-describes the crop, persisting it exactly like an embedded
//! image description so the caption is retrievable and folds into `chunks_root`.
//!
//! Coordinates: Marker `bbox` is `[x0, y0, x1, y1]` in PDF **points, top-left origin** — the same
//! orientation as a `pdftoppm` render — so the mapping to pixels is a straight `pt * dpi/72` scale
//! with **no y-flip** (unlike the pypdfium2 bottom-left `get_pos` path).

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::Path;

use cozo::DbInstance;
use tokio::process::Command;

use archon_ingest_ext::chunk::FigureRegion;

use crate::errors::DocsError;
use crate::ingest::PipelineOutcome;
use crate::ingest_multimodal::persist_vlm_description;
use crate::models::ChunkArtifact;
use crate::pdf_image_progress::emit_pdf_progress;
use crate::pdf_image_vlm::{VlmImageResult, describe_image};
use crate::tool_path::command_path;

/// Render DPI for figure crops. 150 DPI keeps a figure legible for the VLM without huge PNGs.
const FIGURE_RENDER_DPI: u32 = 150;
/// Skip absurdly small regions (a few pt) — usually detector noise, never a real figure.
const MIN_FIGURE_SIDE_PT: f32 = 24.0;

/// Crop + VLM-describe each Marker figure region, persisting the descriptions (which fold into
/// `chunks_root` via the caller). Pages are rendered once each and shared across their figures.
pub(crate) async fn enrich_figure_regions(
    db: &DbInstance,
    document_id: &str,
    figures: &[FigureRegion],
    pdf_path: &Path,
    policy: &archon_policy::EffectivePolicy,
    page_ids_by_number: &BTreeMap<u32, String>,
    outcome: &mut PipelineOutcome,
) -> Result<Vec<ChunkArtifact>, DocsError> {
    let mut collected = Vec::new();
    // Group figures by page so each page renders once.
    let mut by_page: BTreeMap<u32, Vec<&FigureRegion>> = BTreeMap::new();
    for f in figures {
        if is_usable(f) {
            by_page.entry(f.page).or_default().push(f);
        }
    }
    let total: usize = by_page.values().map(Vec::len).sum();
    if total == 0 {
        return Ok(collected);
    }
    emit_pdf_progress(format!(
        "PDF figure-region VLM: doc={document_id} regions={total} across {} page(s)",
        by_page.len()
    ));

    let mut done = 0usize;
    for (page, regions) in by_page {
        let Some(page_id) = page_ids_by_number.get(&page).cloned() else {
            outcome.warnings.push(format!(
                "figure-region VLM: page {page} has no page artifact — skipped {} region(s)",
                regions.len()
            ));
            continue;
        };
        let page_png = match render_page_png(pdf_path, page).await {
            Ok(bytes) => bytes,
            Err(e) => {
                outcome
                    .warnings
                    .push(format!("figure-region VLM: page {page} render failed: {e}"));
                continue;
            }
        };
        for region in regions {
            done += 1;
            let Some(crop) = crop_region(&page_png, region.bbox, FIGURE_RENDER_DPI) else {
                outcome.warnings.push(format!(
                    "figure-region VLM: page {page} crop out of bounds — skipped"
                ));
                continue;
            };
            match describe_image(policy.clone(), crop).await {
                VlmImageResult::Described(description) => {
                    collected.extend(persist_vlm_description(
                        db,
                        document_id,
                        std::slice::from_ref(&page_id),
                        &description,
                    )?);
                    outcome.vlm_descriptions += 1;
                    emit_pdf_progress(format!(
                        "PDF figure-region VLM: doc={document_id} {done}/{total} page={page} ok via {}/{}",
                        description.provider, description.model
                    ));
                }
                VlmImageResult::Fatal(e) => return Err(e),
                VlmImageResult::Failed(msg) => {
                    outcome.pdf_image_vlm_failures += 1;
                    outcome
                        .warnings
                        .push(format!("figure-region VLM page {page}: {msg}"));
                }
                VlmImageResult::Disabled(_) | VlmImageResult::NoProvider => {
                    // VLM off/absent — nothing to do (the caller only runs this when VLM is on).
                }
                VlmImageResult::Empty => {
                    outcome.warnings.push(format!(
                        "figure-region VLM page {page}: empty description — skipped"
                    ));
                }
            }
        }
    }
    Ok(collected)
}

/// A region is usable if both sides exceed [`MIN_FIGURE_SIDE_PT`] (filters detector noise).
fn is_usable(f: &FigureRegion) -> bool {
    let [x0, y0, x1, y1] = f.bbox;
    (x1 - x0) >= MIN_FIGURE_SIDE_PT && (y1 - y0) >= MIN_FIGURE_SIDE_PT
}

/// Render one 1-indexed page to a PNG at [`FIGURE_RENDER_DPI`] via `pdftoppm -singlefile`.
async fn render_page_png(pdf_path: &Path, page: u32) -> Result<Vec<u8>, DocsError> {
    let dir = std::env::temp_dir().join(format!("archon-figure-page-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir)?;
    let prefix = dir.join("page");
    let output = Command::new(command_path("pdftoppm", "ARCHON_PDFTOPPM_BIN"))
        .arg("-png")
        .arg("-r")
        .arg(FIGURE_RENDER_DPI.to_string())
        .arg("-f")
        .arg(page.to_string())
        .arg("-l")
        .arg(page.to_string())
        .arg("-singlefile")
        .arg(pdf_path)
        .arg(&prefix)
        .output()
        .await
        .map_err(|e| DocsError::OcrApi {
            message: format!("pdftoppm figure render failed to start: {e}"),
            status_code: None,
        })?;
    if !output.status.success() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(DocsError::OcrApi {
            message: format!(
                "pdftoppm figure render failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            status_code: output.status.code().map(|c| c as u16),
        });
    }
    let bytes = std::fs::read(dir.join("page.png"));
    let _ = std::fs::remove_dir_all(&dir);
    bytes.map_err(DocsError::from)
}

/// Crop a figure bbox (PDF points, top-left origin) from a page render at `dpi`. `None` when the
/// mapped box is degenerate or entirely outside the rendered page.
fn crop_region(page_png: &[u8], bbox_pt: [f32; 4], dpi: u32) -> Option<Vec<u8>> {
    let img = image::load_from_memory(page_png).ok()?;
    let (w, h) = (img.width(), img.height());
    let scale = dpi as f32 / 72.0;
    // points → pixels (both top-left origin), clamped to the rendered page.
    let x0 = (bbox_pt[0] * scale).floor().max(0.0) as u32;
    let y0 = (bbox_pt[1] * scale).floor().max(0.0) as u32;
    let x1 = ((bbox_pt[2] * scale).ceil() as i64).clamp(0, w as i64) as u32;
    let y1 = ((bbox_pt[3] * scale).ceil() as i64).clamp(0, h as i64) as u32;
    if x1 <= x0 || y1 <= y0 || x0 >= w || y0 >= h {
        return None;
    }
    let crop = img.crop_imm(x0, y0, x1 - x0, y1 - y0);
    let mut out = Vec::new();
    crop.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A solid 300x400px test PNG (page render stand-in at 150 dpi = 144x192 pt).
    fn page_png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([200, 100, 50]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn crop_maps_points_to_pixels_top_left() {
        // 150 dpi → scale 2.0833. A bbox of [10,20,50,80] pt → ~[21,42,104,167] px on a 300x400 img.
        let png = page_png(300, 400);
        let crop = crop_region(&png, [10.0, 20.0, 50.0, 80.0], 150).expect("in bounds");
        let img = image::load_from_memory(&crop).unwrap();
        // width  ≈ (50-10)*150/72 = 83 px; height ≈ (80-20)*150/72 = 125 px (±1 for floor/ceil).
        assert!((img.width() as i64 - 83).abs() <= 2, "w={}", img.width());
        assert!((img.height() as i64 - 125).abs() <= 2, "h={}", img.height());
    }

    #[test]
    fn crop_clamps_to_page_bounds() {
        // A bbox extending past the page is clamped, not rejected.
        let png = page_png(300, 400);
        let crop = crop_region(&png, [100.0, 100.0, 1000.0, 1000.0], 150).expect("clamped");
        let img = image::load_from_memory(&crop).unwrap();
        assert!(img.width() <= 300 && img.height() <= 400);
    }

    #[test]
    fn crop_rejects_out_of_bounds_and_inverted() {
        let png = page_png(300, 400);
        // Entirely past the right edge (x0 in px = 200*2.08 = 417 > 300).
        assert!(crop_region(&png, [200.0, 10.0, 260.0, 60.0], 150).is_none());
        // Inverted bbox (x1 < x0 in points → x1 <= x0 in px) is rejected.
        assert!(crop_region(&png, [80.0, 50.0, 40.0, 80.0], 150).is_none());
    }

    #[test]
    fn tiny_regions_are_filtered() {
        assert!(!is_usable(&FigureRegion {
            page: 1,
            bbox: [10.0, 10.0, 25.0, 100.0] // 15pt wide < 24 → noise
        }));
        assert!(is_usable(&FigureRegion {
            page: 1,
            bbox: [10.0, 10.0, 200.0, 300.0]
        }));
    }
}
