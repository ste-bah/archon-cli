//! The halfblock avatar renderer and its embedded PNG.
//!
//! Split out of `splash.rs` to keep that file under the 500-line ceiling. The
//! division is along a real seam rather than an arbitrary line count: this is
//! raster-to-terminal-cell conversion, `splash.rs` is the text composition, and
//! the two share nothing but a `Buffer`.
//!
//! The startup screen prefers the ASCII art in `splash.rs` (`use_ascii_fallback`
//! is unconditionally true) because it stays crisp on WSL TTYs. This renderer is
//! kept for the compatibility tests and for the image branch that path still
//! carries.

use std::sync::OnceLock;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

// ---------------------------------------------------------------------------
// Embedded avatar
// ---------------------------------------------------------------------------

const AVATAR_PNG: &[u8] = include_bytes!("../../../archon-avatar.png");

// ---------------------------------------------------------------------------
// Cached decoded image (decoded once, reused across frames)
// ---------------------------------------------------------------------------

static AVATAR_IMAGE: OnceLock<image::DynamicImage> = OnceLock::new();

pub(crate) fn get_avatar() -> &'static image::DynamicImage {
    AVATAR_IMAGE.get_or_init(|| {
        image::load_from_memory(AVATAR_PNG)
            .expect("archon-avatar.png must be a valid PNG at compile time")
    })
}

// ---------------------------------------------------------------------------
// Halfblock image renderer
// ---------------------------------------------------------------------------

/// Render an image into a rectangular region using unicode halfblock characters.
///
/// Each terminal cell covers 2 vertical pixels: the top pixel becomes the
/// foreground color, the bottom pixel becomes the background color, and the
/// glyph is `▀` (U+2580 UPPER HALF BLOCK).
pub(crate) fn render_halfblock_image(buf: &mut Buffer, area: Rect, img: &image::DynamicImage) {
    let cell_w = area.width as u32;
    let cell_h = area.height as u32;
    if cell_w == 0 || cell_h == 0 {
        return;
    }

    // Terminal area in pixels: each cell = 1px wide × 2px tall (halfblock).
    let max_px_w = cell_w;
    let max_px_h = cell_h * 2;

    // Preserve source aspect ratio.
    let (src_w, src_h) = (img.width(), img.height());
    let scale_w = max_px_w as f64 / src_w as f64;
    let scale_h = max_px_h as f64 / src_h as f64;
    let scale = scale_w.min(scale_h);

    let render_px_w = ((src_w as f64 * scale).round() as u32).max(1);
    let render_px_h = ((src_h as f64 * scale).round() as u32).max(1);

    // Center the rendered image within the area (letterbox / pillarbox).
    let pad_px_x = (max_px_w - render_px_w) / 2;
    let pad_px_y = (max_px_h - render_px_h) / 2;

    let resized = img.resize_exact(
        render_px_w,
        render_px_h,
        image::imageops::FilterType::Lanczos3,
    );
    let rgba = resized.to_rgba8();

    // Render row by row — cells outside the image region fill with black.
    for cell_row in 0..cell_h {
        let row_y = area.y + cell_row as u16;
        let mut spans = Vec::with_capacity(cell_w as usize);

        let img_top_y = (cell_row * 2) as i64 - pad_px_y as i64;
        let img_bot_y = img_top_y + 1;

        for col in 0..cell_w {
            let img_x = col as i64 - pad_px_x as i64;

            let in_x = img_x >= 0 && (img_x as u32) < render_px_w;
            let top_in = in_x && img_top_y >= 0 && (img_top_y as u32) < render_px_h;
            let bot_in = in_x && img_bot_y >= 0 && (img_bot_y as u32) < render_px_h;

            let fg = if top_in {
                let p = rgba.get_pixel(img_x as u32, img_top_y as u32);
                Color::Rgb(p[0], p[1], p[2])
            } else {
                Color::Black
            };
            let bg = if bot_in {
                let p = rgba.get_pixel(img_x as u32, img_bot_y as u32);
                Color::Rgb(p[0], p[1], p[2])
            } else {
                Color::Black
            };
            spans.push(Span::styled("▀", Style::default().fg(fg).bg(bg)));
        }

        let line = Line::from(spans);
        let row_area = Rect::new(area.x, row_y, area.width, 1);
        Paragraph::new(line).render(row_area, buf);
    }
}
