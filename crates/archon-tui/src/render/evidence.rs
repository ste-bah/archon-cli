//! Evidence Engine overlay rendering.

use ratatui::Frame;

use crate::app::{App, EvidenceViewState};

pub fn draw_evidence_view(frame: &mut Frame, app: &App) {
    let Some(view) = &app.evidence_view else {
        return;
    };
    let area = centered_overlay(frame.area());
    frame.render_widget(ratatui::widgets::Clear, area);
    match view {
        EvidenceViewState::Docs(screen) => screen.render(frame, area, &app.theme),
        EvidenceViewState::GameTheory(screen) => screen.render(frame, area, &app.theme),
        EvidenceViewState::Learning(screen) => screen.render(frame, area, &app.theme),
    }
}

fn centered_overlay(area: ratatui::layout::Rect) -> ratatui::layout::Rect {
    let width = (area.width * 9 / 10)
        .max(70)
        .min(area.width.saturating_sub(2));
    let height = (area.height * 8 / 10)
        .max(10)
        .min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    ratatui::layout::Rect::new(x, y, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centered_overlay_stays_inside_frame() {
        let frame = ratatui::layout::Rect::new(0, 0, 120, 40);
        let overlay = centered_overlay(frame);
        assert!(overlay.width <= frame.width);
        assert!(overlay.height <= frame.height);
        assert!(overlay.x > 0);
        assert!(overlay.y > 0);
    }
}
