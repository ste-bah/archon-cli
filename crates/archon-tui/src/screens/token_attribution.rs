//! The `/context` attribution overlay (#192 scope B).
//!
//! Layer 1 module — no imports from screens/ or app/.
//!
//! `token_surface.rs` has computed per-message attribution since #189 Phase 3.
//! Exactly one number of it reached the screen: `top Nk` in the status bar, the
//! size of the single worst message. So the bar could say *how much* the worst
//! message costs but never *which* message, and `top_contributors` — the
//! function whose doc comment says it answers "what is filling it" — had no
//! caller outside its own tests.
//!
//! This is that caller. Two sources meet here and neither invents the other's
//! half: the agent supplies the ranking, because only it has the calibrated
//! surface, and `/context` supplies the message text from the session log,
//! because the agent's event carries indices rather than prose. A row whose
//! text never arrived still shows its index and its cost — the number is the
//! actionable part, and withholding it because the label is missing would be
//! the wrong trade.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::widgets::Row;

use crate::theme::Theme;
use crate::virtual_list::VirtualList;

/// One message's contribution to the context window.
#[derive(Debug, Clone, PartialEq)]
pub struct Contributor {
    /// Position in the conversation, as the agent counts it.
    pub message_index: usize,
    /// Calibrated token estimate.
    pub tokens: u64,
    /// Share of the whole attributed surface, 0.0–100.0.
    pub share_percent: f64,
    /// `user` / `assistant`, or empty when the log could not be read.
    pub role: String,
    /// One line of the message, or empty when the log could not be read.
    pub summary: String,
}

impl Contributor {
    /// Tokens as a short human string: `42.1k`, or the raw count below 1000.
    ///
    /// Rendering `42104` in a column beside `38` makes the ranking harder to
    /// read than the rounding costs.
    pub fn tokens_label(&self) -> String {
        if self.tokens >= 1000 {
            format!("{:.1}k", self.tokens as f64 / 1000.0)
        } else {
            self.tokens.to_string()
        }
    }
}

/// Ranked list of what is filling the context window.
#[derive(Debug)]
pub struct TokenAttributionOverlay {
    list: VirtualList<Contributor>,
    /// Attributed tokens across every message, including unlisted ones.
    total: u64,
}

impl TokenAttributionOverlay {
    pub fn new(total: u64) -> Self {
        Self {
            list: VirtualList::new(Vec::new(), 10),
            total,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    crate::virtual_list::delegate_virtual_list!(list, Contributor);

    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn set_contributors(&mut self, contributors: Vec<Contributor>) {
        self.list.set_items(contributors);
    }

    /// The share the listed rows account for between them.
    ///
    /// The ranking is truncated, so this is what says whether dropping
    /// everything on screen would actually help — "the top ten are 12% of the
    /// window" is a different situation from "the top ten are 91%".
    pub fn listed_share_percent(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        let listed: u64 = self.list.items().iter().map(|row| row.tokens).sum();
        (listed as f64 / self.total as f64) * 100.0
    }

    /// Draw the ranking into a centred rect inside `area`.
    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        const TITLE: &str = " Context — Up/Down select · Esc close ";

        if self.list.is_empty() {
            crate::overlay::message(
                f,
                area,
                TITLE,
                "No per-message attribution yet. It is measured on the next \
                 request, so send something and run /context again.",
                theme,
            );
            return;
        }

        // rows + header + footer + two border lines.
        let (region, block) =
            crate::overlay::open(f, area, self.list.len() as u16 + 4, TITLE, theme);

        let widths = [
            Constraint::Length(4),
            Constraint::Length(8),
            Constraint::Length(7),
            Constraint::Length(10),
            Constraint::Min(20),
        ];

        let mut rows: Vec<Row> = self
            .list
            .items()
            .iter()
            .map(|entry| {
                Row::new([
                    entry.message_index.to_string(),
                    entry.tokens_label(),
                    format!("{:.1}%", entry.share_percent),
                    entry.role.clone(),
                    entry.summary.clone(),
                ])
                .style(crate::overlay::body_style(theme))
            })
            .collect();

        // A footer row rather than a separate widget: it has to scroll with
        // nothing and align with the columns above it.
        rows.push(
            Row::new([
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                format!(
                    "listed: {:.1}% of {} attributed tokens",
                    self.listed_share_percent(),
                    self.total
                ),
            ])
            .style(crate::overlay::header_style(theme)),
        );

        crate::overlay::render_table(
            f,
            region,
            block,
            Row::new(["#", "tokens", "share", "role", "message"]),
            rows,
            &widths,
            self.list.selected_index(),
            theme,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contributor(index: usize, tokens: u64, share: f64) -> Contributor {
        Contributor {
            message_index: index,
            tokens,
            share_percent: share,
            role: "user".into(),
            summary: format!("message {index}"),
        }
    }

    #[test]
    fn a_new_overlay_is_empty() {
        assert!(TokenAttributionOverlay::new(0).is_empty());
    }

    #[test]
    fn thousands_are_abbreviated_and_small_counts_are_not() {
        assert_eq!(contributor(0, 42_104, 0.0).tokens_label(), "42.1k");
        assert_eq!(contributor(0, 999, 0.0).tokens_label(), "999");
        assert_eq!(contributor(0, 1000, 0.0).tokens_label(), "1.0k");
    }

    /// The truncated ranking's own share is what says whether acting on the
    /// listed rows would help at all.
    #[test]
    fn the_listed_share_is_measured_against_the_whole_surface() {
        let mut overlay = TokenAttributionOverlay::new(1000);
        overlay.set_contributors(vec![contributor(0, 300, 30.0), contributor(1, 200, 20.0)]);
        assert!((overlay.listed_share_percent() - 50.0).abs() < 1e-6);
    }

    #[test]
    fn an_unmeasured_surface_reports_no_share_rather_than_dividing_by_zero() {
        let mut overlay = TokenAttributionOverlay::new(0);
        overlay.set_contributors(vec![contributor(0, 300, 0.0)]);
        assert_eq!(overlay.listed_share_percent(), 0.0);
    }

    #[test]
    fn the_cursor_wraps() {
        let mut overlay = TokenAttributionOverlay::new(100);
        overlay.set_contributors(vec![contributor(0, 60, 60.0), contributor(1, 40, 40.0)]);
        overlay.move_down();
        assert_eq!(overlay.selected_index(), 1);
        overlay.move_down();
        assert_eq!(overlay.selected_index(), 0);
        assert_eq!(overlay.selected().expect("selected").message_index, 0);
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn overlay() -> TokenAttributionOverlay {
        let mut overlay = TokenAttributionOverlay::new(100_000);
        overlay.set_contributors(vec![
            Contributor {
                message_index: 12,
                tokens: 42_000,
                share_percent: 42.0,
                role: "assistant".into(),
                summary: "the enormous build log".into(),
            },
            Contributor {
                message_index: 3,
                tokens: 8_000,
                share_percent: 8.0,
                role: "user".into(),
                summary: "pasted the config".into(),
            },
        ]);
        overlay
    }

    fn draw(overlay: &TokenAttributionOverlay) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).expect("terminal");
        terminal
            .draw(|frame| overlay.render(frame, frame.area(), &crate::theme::dark_theme()))
            .expect("draw attribution overlay");
        terminal
    }

    fn text(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn style_of(terminal: &Terminal<TestBackend>, needle: &str) -> Option<ratatui::style::Style> {
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        for y in 0..area.height {
            let line: String = (0..area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect();
            if let Some(column) = line.find(needle) {
                return Some(buffer[(column as u16, y)].style());
            }
        }
        None
    }

    /// The whole point: which message, not only how big the worst one is.
    #[test]
    fn each_row_names_the_message_its_cost_and_its_share() {
        let rendered = text(&draw(&overlay()));
        assert!(rendered.contains("42.0k"), "{rendered}");
        assert!(rendered.contains("42.0%"), "{rendered}");
        assert!(rendered.contains("the enormous build log"), "{rendered}");
        assert!(rendered.contains("12"), "the message index is missing");
    }

    #[test]
    fn the_footer_says_what_the_listed_rows_add_up_to() {
        let rendered = text(&draw(&overlay()));
        assert!(rendered.contains("listed: 50.0%"), "{rendered}");
    }

    #[test]
    fn the_selected_row_is_visibly_selected_and_moves_with_the_keys() {
        let mut overlay = overlay();
        let first = draw(&overlay);
        let one = style_of(&first, "the enormous build log").expect("first row");
        let two = style_of(&first, "pasted the config").expect("second row");
        assert_ne!(one, two, "selection is invisible");

        overlay.move_down();
        assert_ne!(
            one,
            style_of(&draw(&overlay), "the enormous build log").expect("still drawn"),
            "moving the selection changed nothing on screen"
        );
    }

    /// An empty bordered box is indistinguishable from a broken widget.
    #[test]
    fn nothing_measured_yet_is_said_in_words() {
        let rendered = text(&draw(&TokenAttributionOverlay::new(0)));
        assert!(
            rendered.contains("No per-message attribution yet"),
            "{rendered}"
        );
    }

    #[test]
    fn a_row_with_no_message_text_still_shows_its_cost() {
        let mut overlay = TokenAttributionOverlay::new(10_000);
        overlay.set_contributors(vec![Contributor {
            message_index: 7,
            tokens: 5_000,
            share_percent: 50.0,
            role: String::new(),
            summary: String::new(),
        }]);
        let rendered = text(&draw(&overlay));
        assert!(rendered.contains("5.0k"), "{rendered}");
        assert!(rendered.contains("50.0%"), "{rendered}");
    }

    #[test]
    fn the_overlay_does_not_cover_the_whole_frame() {
        let terminal = draw(&overlay());
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        let bottom: String = (0..area.width)
            .map(|x| buffer[(x, area.height - 1)].symbol().to_string())
            .collect();
        assert!(bottom.trim().is_empty(), "painted the status-bar row");
    }
}
