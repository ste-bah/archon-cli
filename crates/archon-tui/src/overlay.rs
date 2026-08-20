//! Shared chrome for modal overlays.
//!
//! Layer 0 — imports `ratatui` and `theme`, nothing else.
//!
//! # Why this exists
//!
//! Every overlay in this crate needs the same four things and, until #192, each
//! one decided for itself whether to bother:
//!
//! | | had it |
//! |---|---|
//! | `Clear` before drawing (opaque) | 3 of 10 |
//! | uses the theme | 1 of 10 |
//! | `render_stateful_widget` so the selection is visible | 0 of 10 |
//! | `highlight_style` | 0 of 10 |
//!
//! The result was overlays that painted over live content without covering it,
//! ignored the user's theme, and tracked a selection index nothing drew — so
//! `Up`/`Down` looked like dead keys. Each of those was individually a one-line
//! omission, which is exactly why they need to stop being per-screen decisions.
//!
//! A screen that renders through [`open`] cannot forget the background, cannot
//! miss the theme, and gets the same geometry and the same selection colour as
//! every other screen. Polish becomes structural rather than a thing each
//! author has to remember.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::theme::Theme;

/// Widest an overlay may be, in columns.
///
/// Full-width modals are hard to read and hide the conversation behind them for
/// no benefit; this is about the width of comfortable prose.
const MAX_WIDTH: u16 = 84;

/// Centre a rect inside `area`, sized to content and bounded by the frame.
///
/// `content_height` is what the caller would like; the result is clamped so the
/// overlay never reaches the frame edge. That margin is load-bearing — the
/// tasks overlay used to render into `frame.area()` and covered the status bar
/// and the input line, so the user could not see what they were typing.
pub(crate) fn centred(area: Rect, content_height: u16) -> Rect {
    let width = area.width.saturating_sub(4).min(MAX_WIDTH);
    let height = content_height.max(3).min(area.height.saturating_sub(2));
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

/// Clear a centred region and return it with a themed block to draw into.
///
/// The `Clear` is the whole point: `ratatui` widgets composite over whatever
/// occupies their cells, so an overlay that skips it shows the screen behind it
/// through every gap in its own content.
///
/// `title` should name the keys that work — an overlay whose bindings are
/// undiscoverable is only marginally better than one that does not open.
pub(crate) fn open<'a>(
    frame: &mut Frame,
    area: Rect,
    content_height: u16,
    title: &'a str,
    theme: &Theme,
) -> (Rect, Block<'a>) {
    let region = centred(area, content_height);
    frame.render_widget(Clear, region);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(theme.accent));
    (region, block)
}

/// Marker drawn against the selected row.
///
/// Colour alone is not enough. The first cut of these overlays marked the
/// selection with `highlight_style` only, and on a real terminal it was not
/// perceptible — which made the arrow keys look dead, because the only
/// evidence they had worked was a colour nobody could see. A glyph cannot fail
/// to render, does not depend on the theme, and survives a terminal that
/// mangles background colours.
pub(crate) const HIGHLIGHT_SYMBOL: &str = "▸ ";

/// Style for the selected row of a list or table.
///
/// Inverted rather than merely coloured, so it reads as a cursor at a glance
/// and survives themes whose accent is close to the foreground. Paired with
/// [`HIGHLIGHT_SYMBOL`] — the colour is the emphasis, the glyph is the proof.
pub(crate) fn selection_style(theme: &Theme) -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(theme.accent)
        .add_modifier(Modifier::BOLD)
}

/// Style for a table or list header row.
pub(crate) fn header_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.header)
        .add_modifier(Modifier::BOLD)
}

/// Style for ordinary, unselected content.
pub(crate) fn body_style(theme: &Theme) -> Style {
    Style::default().fg(theme.fg)
}

/// Draw an overlay whose only content is a message.
///
/// For the empty case, which every list overlay has and none of them used to
/// handle: an empty bordered box is indistinguishable from a broken widget, so
/// the reason there is nothing to show has to be said in words.
pub(crate) fn message(frame: &mut Frame, area: Rect, title: &str, body: &str, theme: &Theme) {
    // Sized from the wrapped height, not a fixed 5. The permissions overlay's
    // empty state ran past the right border and was clipped mid-word on a real
    // terminal — "Add them under [permissions] in co". A message worth writing
    // is worth reading to the end of.
    let usable = centred(area, 5).width.saturating_sub(4).max(1);
    let lines = wrapped_line_count(body, usable);
    let (region, block) = open(frame, area, lines.saturating_add(4), title, theme);
    let paragraph = Paragraph::new(format!("\n  {body}"))
        .style(Style::default().fg(theme.muted))
        .wrap(ratatui::widgets::Wrap { trim: false })
        .block(block);
    frame.render_widget(paragraph, region);
}

/// Rows `body` needs at `width`, counting whole words.
///
/// Deliberately approximate: it only has to be right enough to reserve height,
/// and `Paragraph`'s own wrapping does the drawing.
fn wrapped_line_count(body: &str, width: u16) -> u16 {
    let width = width.max(1) as usize;
    let mut rows: u16 = 1;
    let mut used = 2; // the two-space indent the body is drawn with
    for word in body.split_whitespace() {
        let needed = word.chars().count() + usize::from(used > 2);
        if used + needed > width {
            rows = rows.saturating_add(1);
            used = word.chars().count();
        } else {
            used += needed;
        }
    }
    rows
}

#[cfg(test)]
mod message_wrap_tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn rendered(body: &str) -> String {
        let mut terminal = Terminal::new(TestBackend::new(96, 24)).expect("terminal");
        terminal
            .draw(|frame| {
                message(
                    frame,
                    frame.area(),
                    " Title ",
                    body,
                    &crate::theme::dark_theme(),
                )
            })
            .expect("draw message");
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

    /// Caught on a real terminal: the permissions empty state was clipped at
    /// "in co". Every word of a message has to survive to the screen.
    #[test]
    fn a_message_longer_than_the_overlay_still_ends_where_it_should() {
        const BODY: &str = "No rules configured — the mode alone decides. \
                            Add them under [permissions] in config.toml.";
        let out = rendered(BODY);

        for word in BODY.split_whitespace() {
            assert!(out.contains(word), "{word:?} was clipped away:\n{out}");
        }
    }

    #[test]
    fn a_short_message_does_not_grow_the_box() {
        let out = rendered("Nothing is running.");
        let borders = out.lines().filter(|line| line.contains('└')).count();
        assert_eq!(borders, 1, "more than one box was drawn:\n{out}");
    }

    #[test]
    fn the_wrap_estimate_counts_whole_words() {
        assert_eq!(super::wrapped_line_count("one two", 40), 1);
        assert_eq!(super::wrapped_line_count("aaaa bbbb cccc", 10), 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn theme() -> Theme {
        crate::theme::dark_theme()
    }

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
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

    #[test]
    fn a_centred_overlay_never_touches_the_frame_edge() {
        let frame = Rect::new(0, 0, 100, 30);
        let region = centred(frame, 12);
        assert!(region.x > 0, "{region:?}");
        assert!(region.y > 0, "{region:?}");
        assert!(region.x + region.width < frame.width, "{region:?}");
        assert!(
            region.y + region.height < frame.height,
            "the overlay reached the bottom row, where the status bar lives: {region:?}"
        );
    }

    #[test]
    fn width_is_capped_so_wide_terminals_do_not_get_a_full_width_modal() {
        let region = centred(Rect::new(0, 0, 400, 40), 10);
        assert_eq!(region.width, MAX_WIDTH);
    }

    #[test]
    fn a_tiny_frame_still_produces_a_drawable_region() {
        let region = centred(Rect::new(0, 0, 20, 6), 40);
        assert!(region.width > 0 && region.height > 0, "{region:?}");
        assert!(
            region.height <= 4,
            "must fit inside a 6-row frame: {region:?}"
        );
    }

    /// The defect this module exists to make impossible.
    #[test]
    fn open_clears_the_region_so_content_behind_does_not_show_through() {
        let mut terminal = Terminal::new(TestBackend::new(60, 20)).expect("terminal");
        terminal
            .draw(|frame| {
                let area = frame.area();
                // Something noisy underneath, as a real screen would have.
                frame.render_widget(
                    Paragraph::new(
                        "XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX\n".repeat(20),
                    ),
                    area,
                );
                let (region, block) = open(frame, area, 8, " Title ", &theme());
                frame.render_widget(block, region);
            })
            .expect("draw");

        let rendered = buffer_text(&terminal);
        let middle = rendered.lines().nth(10).unwrap_or_default();
        assert!(
            middle.contains(' '),
            "the overlay interior still shows the content behind it: {middle:?}"
        );
    }

    #[test]
    fn the_title_is_drawn_so_keys_are_discoverable() {
        let mut terminal = Terminal::new(TestBackend::new(60, 20)).expect("terminal");
        terminal
            .draw(|frame| {
                let area = frame.area();
                let (region, block) = open(frame, area, 8, " Up/Down · Esc close ", &theme());
                frame.render_widget(block, region);
            })
            .expect("draw");
        assert!(buffer_text(&terminal).contains("Esc close"));
    }

    #[test]
    fn message_says_why_there_is_nothing_to_show() {
        let mut terminal = Terminal::new(TestBackend::new(60, 20)).expect("terminal");
        terminal
            .draw(|frame| {
                let area = frame.area();
                message(frame, area, " Tasks ", "Nothing is running.", &theme());
            })
            .expect("draw");
        assert!(buffer_text(&terminal).contains("Nothing is running."));
    }

    #[test]
    fn the_selection_style_is_distinguishable_from_body_text() {
        let t = theme();
        assert_ne!(
            selection_style(&t),
            body_style(&t),
            "selected rows would render identically to unselected ones"
        );
    }
}
