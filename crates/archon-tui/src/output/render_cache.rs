//! Cached rendered-output helper types.

use ratatui::text::Line;

use crate::theme::Theme;

#[derive(Debug, Clone)]
pub(super) struct RenderCache {
    pub(super) revision: u64,
    pub(super) theme: Theme,
    pub(super) lines: Vec<Line<'static>>,
    pub(super) raw_lines: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct WrapCache {
    pub(super) revision: u64,
    pub(super) width: u16,
    pub(super) offsets: Vec<u16>,
    pub(super) total_wrapped: u16,
}

#[derive(Debug, Clone)]
pub struct RenderedOutputView {
    pub lines: Vec<Line<'static>>,
    pub total_wrapped: u16,
    pub global_scroll_y: u16,
    pub paragraph_scroll_y: u16,
}

pub(super) fn visible_line_range(
    wrap: &WrapCache,
    scroll_y: u16,
    visible_height: u16,
) -> (usize, usize, u16) {
    if wrap.offsets.is_empty() {
        return (0, 0, 0);
    }

    let viewport_end = scroll_y.saturating_add(visible_height).saturating_add(1);
    let start = wrap
        .offsets
        .iter()
        .enumerate()
        .find(|(idx, offset)| {
            let next = wrap
                .offsets
                .get(idx + 1)
                .copied()
                .unwrap_or(wrap.total_wrapped);
            (**offset <= scroll_y && next > scroll_y) || **offset > scroll_y
        })
        .map(|(idx, _)| idx)
        .unwrap_or_else(|| wrap.offsets.len().saturating_sub(1));

    let end = wrap
        .offsets
        .iter()
        .enumerate()
        .skip(start)
        .find(|(_, offset)| **offset >= viewport_end)
        .map(|(idx, _)| idx + 1)
        .unwrap_or(wrap.offsets.len());

    let paragraph_scroll_y = scroll_y.saturating_sub(wrap.offsets[start]);
    (
        start,
        end.max(start + 1).min(wrap.offsets.len()),
        paragraph_scroll_y,
    )
}
