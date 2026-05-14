use ratatui::layout::Rect;

pub(crate) fn overlay_area(area: Rect) -> Rect {
    let width = area.width.saturating_mul(9) / 10;
    let height = area.height.saturating_mul(4) / 5;
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width.max(40).min(area.width),
        height.max(12).min(area.height),
    )
}

pub(crate) fn inset(area: Rect, x: u16, y: u16) -> Rect {
    Rect::new(
        area.x.saturating_add(x),
        area.y.saturating_add(y),
        area.width.saturating_sub(x.saturating_mul(2)),
        area.height.saturating_sub(y.saturating_mul(2)),
    )
}
