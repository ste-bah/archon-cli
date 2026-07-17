use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::app::App;

pub(crate) fn thinking_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    if !app.show_thinking || !app.thinking.active {
        return Vec::new();
    }
    if app.thinking.expanded {
        return expanded_lines(&app.thinking.accumulated, app);
    }
    vec![
        animated_header(app),
        tail_preview(&app.thinking.accumulated, width, app),
    ]
}

fn animated_header(app: &App) -> Line<'static> {
    let bright = app.thinking.bright_dot_index();
    let mut spans = vec![Span::styled(
        "+ Thinking",
        Style::default().fg(app.theme.thinking_dot),
    )];
    for i in 0..3 {
        let color = if i == bright {
            app.theme.thinking_dot_bright
        } else {
            app.theme.thinking_dot
        };
        spans.push(Span::styled(".", Style::default().fg(color)));
    }
    Line::from(spans)
}

fn tail_preview(text: &str, width: u16, app: &App) -> Line<'static> {
    Line::from(Span::styled(
        format!(
            "  {}",
            display_width_tail(text, width.saturating_sub(2) as usize)
        ),
        Style::default()
            .fg(app.theme.muted)
            .add_modifier(Modifier::ITALIC),
    ))
}

fn expanded_lines(text: &str, app: &App) -> Vec<Line<'static>> {
    let style = Style::default()
        .fg(app.theme.muted)
        .add_modifier(Modifier::ITALIC);
    std::iter::once(Line::from(Span::styled("- Thinking:", style)))
        .chain(
            text.lines()
                .map(|line| Line::from(Span::styled(format!("  {line}"), style))),
        )
        .collect()
}

fn display_width_tail(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let mut tail = String::new();
    let mut used = 0;
    for grapheme in text.graphemes(true).rev() {
        if grapheme.contains('\n') || grapheme.contains('\r') {
            break;
        }
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if grapheme_width > width.saturating_sub(used) {
            break;
        }
        tail.insert_str(0, grapheme);
        used += grapheme_width;
    }
    tail
}

#[cfg(test)]
mod tests {
    use super::display_width_tail;

    #[test]
    fn tail_keeps_combining_marks_with_their_base_grapheme() {
        assert_eq!(display_width_tail("界界e\u{301}XYZ", 6), "界e\u{301}XYZ");
    }

    #[test]
    fn tail_stops_at_the_newest_newline() {
        assert_eq!(display_width_tail("old\nnew", 8), "new");
    }
}
