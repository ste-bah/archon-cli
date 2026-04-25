//! TASK-TUI-625: QR code rendering helper.
//!
//! Encapsulates the `qrcode` dependency so callers (e.g. the
//! `/session` slash-command handler in the bin-crate) don't need to
//! depend on `qrcode` directly. Exposes a single helper that renders
//! a URL into a terminal-friendly Unicode half-block QR code string.
//!
//! The helper uses the `qrcode::render::unicode::Dense1x2` renderer
//! which emits two-pixels-per-character output using the Unicode
//! block characters ` `, `▀`, `▄`, `█` — so the rendered QR stays
//! readable in standard terminal fonts.
//!
//! Errors are stringified at the crate boundary to avoid leaking
//! `qrcode::types::QrError` into the bin-crate's public surface.

use qrcode::QrCode;
use qrcode::render::unicode::Dense1x2;

/// Render the given URL (or arbitrary UTF-8 payload) as a Unicode
/// half-block QR code suitable for display in a terminal.
///
/// Returns the rendered multi-line string on success, or a
/// human-readable error message on failure (e.g. payload too large
/// for the largest QR version).
pub fn render_url_as_qr(url: &str) -> Result<String, String> {
    let code = QrCode::new(url.as_bytes()).map_err(|e| format!("{}", e))?;
    let image = code
        .render::<Dense1x2>()
        .dark_color(Dense1x2::Dark)
        .light_color(Dense1x2::Light)
        .quiet_zone(true)
        .build();
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_small_url_with_half_blocks() {
        let out = render_url_as_qr("https://example.test/").unwrap();
        // The Dense1x2 renderer emits one of these glyphs for any
        // non-empty QR: space, ▀, ▄, or █. At least one "dark" glyph
        // must appear — the QR is never fully blank.
        assert!(
            out.contains('\u{2580}') || out.contains('\u{2584}') || out.contains('\u{2588}'),
            "expected a Unicode half-block glyph in QR; got: {}",
            out
        );
    }
}
