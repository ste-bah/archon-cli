//! Cached rendered-output helper types.

use std::collections::VecDeque;

use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::theme::Theme;

#[derive(Debug, Clone)]
pub(super) struct RenderCache {
    pub(super) revision: u64,
    pub(super) theme: Theme,
    pub(super) lines: Vec<Line<'static>>,
    pub(super) raw_lines: Vec<String>,
    pub(super) rendered_text: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct WrapCache {
    pub(super) revision: u64,
    pub(super) width: u16,
    pub(super) offsets: Vec<usize>,
    pub(super) total_wrapped: usize,
}

#[derive(Debug, Clone)]
pub struct RenderedOutputView {
    pub lines: Vec<Line<'static>>,
    pub total_wrapped: usize,
    pub global_scroll_y: usize,
    pub paragraph_scroll_y: u16,
}

pub(super) fn line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

pub(super) fn line_suffix(line: &Line<'static>, start: usize) -> Line<'static> {
    let mut consumed = 0;
    let spans = line
        .spans
        .iter()
        .filter_map(|span| {
            let end = consumed + span.content.len();
            let suffix = if end <= start {
                None
            } else {
                Some(Span::styled(
                    span.content[start.saturating_sub(consumed)..].to_string(),
                    span.style,
                ))
            };
            consumed = end;
            suffix
        })
        .collect::<Vec<_>>();
    let mut suffix = Line::from(spans);
    suffix.alignment = line.alignment;
    suffix.style = line.style;
    suffix
}

pub(super) fn count_wrapped_rows(lines: &[&str], width: u16) -> usize {
    if width == 0 {
        return lines.len();
    }
    lines
        .iter()
        .map(|line| wrapped_row_starts(line, width as usize).len())
        .sum()
}

pub(super) fn wrapped_suffix_after_rows(line: &str, width: u16, rows_to_skip: usize) -> &str {
    if rows_to_skip == 0 || width == 0 {
        return line;
    }
    let starts = wrapped_row_starts(line, width as usize);
    starts
        .get(rows_to_skip)
        .map_or(&line[line.len()..], |start| &line[*start..])
}

fn wrapped_row_starts(line: &str, width: usize) -> Vec<usize> {
    if line.is_empty() {
        return vec![0];
    }
    let mut state = WrapState::new(width);
    for (start, grapheme) in line.grapheme_indices(true) {
        state.push(start, grapheme);
    }
    state.finish()
}

struct WrapState {
    width: usize,
    starts: Vec<usize>,
    line_start: Option<usize>,
    line_width: usize,
    word_width: usize,
    whitespace_width: usize,
    pending_word: VecDeque<(usize, usize)>,
    pending_whitespace: VecDeque<(usize, usize)>,
    non_whitespace_previous: bool,
}

impl WrapState {
    fn new(width: usize) -> Self {
        Self {
            width,
            starts: Vec::new(),
            line_start: None,
            line_width: 0,
            word_width: 0,
            whitespace_width: 0,
            pending_word: VecDeque::new(),
            pending_whitespace: VecDeque::new(),
            non_whitespace_previous: false,
        }
    }

    fn push(&mut self, start: usize, grapheme: &str) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if grapheme_width > self.width {
            return;
        }
        let whitespace = is_wrap_whitespace(grapheme);
        if self.should_flush(grapheme_width, whitespace) {
            self.flush_pending();
        }
        if self.should_wrap(grapheme_width) && self.wrap_pending(start, whitespace) {
            return;
        }
        if whitespace {
            self.whitespace_width = self.whitespace_width.saturating_add(grapheme_width);
            self.pending_whitespace.push_back((start, grapheme_width));
        } else {
            self.word_width = self.word_width.saturating_add(grapheme_width);
            self.pending_word.push_back((start, grapheme_width));
        }
        self.non_whitespace_previous = !whitespace;
    }

    fn should_flush(&self, grapheme_width: usize, whitespace: bool) -> bool {
        let word_found = self.non_whitespace_previous && whitespace;
        let untrimmed_overflow = self.line_start.is_none()
            && self
                .word_width
                .saturating_add(self.whitespace_width)
                .saturating_add(grapheme_width)
                > self.width;
        word_found || untrimmed_overflow
    }

    fn should_wrap(&self, grapheme_width: usize) -> bool {
        self.line_width >= self.width
            || (grapheme_width > 0
                && self
                    .line_width
                    .saturating_add(self.whitespace_width)
                    .saturating_add(self.word_width)
                    >= self.width)
    }

    fn wrap_pending(&mut self, start: usize, whitespace: bool) -> bool {
        self.starts.push(self.line_start.take().unwrap_or(start));
        let mut remaining = self.width.saturating_sub(self.line_width);
        self.line_width = 0;
        while let Some((_, pending_width)) = self.pending_whitespace.front().copied() {
            if pending_width > remaining {
                break;
            }
            self.whitespace_width = self.whitespace_width.saturating_sub(pending_width);
            remaining = remaining.saturating_sub(pending_width);
            self.pending_whitespace.pop_front();
        }
        if whitespace && self.pending_whitespace.is_empty() {
            self.non_whitespace_previous = false;
            return true;
        }
        false
    }

    fn flush_pending(&mut self) {
        extend_pending(
            &mut self.line_start,
            &mut self.line_width,
            &mut self.pending_whitespace,
            &mut self.whitespace_width,
        );
        extend_pending(
            &mut self.line_start,
            &mut self.line_width,
            &mut self.pending_word,
            &mut self.word_width,
        );
    }

    fn finish(mut self) -> Vec<usize> {
        if self.line_start.is_none()
            && self.pending_word.is_empty()
            && !self.pending_whitespace.is_empty()
        {
            self.starts.push(
                self.pending_whitespace
                    .front()
                    .map_or(0, |(start, _)| *start),
            );
        }
        self.flush_pending();
        if let Some(start) = self.line_start {
            self.starts.push(start);
        }
        if self.starts.is_empty() {
            self.starts.push(0);
        }
        self.starts
    }
}

fn extend_pending(
    line_start: &mut Option<usize>,
    line_width: &mut usize,
    pending: &mut VecDeque<(usize, usize)>,
    pending_width: &mut usize,
) {
    if line_start.is_none() {
        *line_start = pending.front().map(|(start, _)| *start);
    }
    pending.clear();
    *line_width = line_width.saturating_add(*pending_width);
    *pending_width = 0;
}

fn is_wrap_whitespace(grapheme: &str) -> bool {
    grapheme == "\u{200b}" || (grapheme != "\u{00a0}" && grapheme.chars().all(char::is_whitespace))
}

#[cfg(test)]
mod tests {
    use super::{count_wrapped_rows, wrapped_suffix_after_rows};

    #[test]
    fn counts_boundary_whitespace_like_ratatui_word_wrap() {
        assert_eq!(count_wrapped_rows(&[" "], 1), 2);
        assert_eq!(count_wrapped_rows(&["a "], 1), 1);
        assert_eq!(count_wrapped_rows(&["  "], 1), 1);
    }

    #[test]
    fn counts_unicode_whitespace_like_ratatui_word_wrap() {
        assert_eq!(count_wrapped_rows(&["a\u{2003}bbb c"], 4), 3);
    }

    #[test]
    fn handles_ratatui_special_whitespace_and_overwide_graphemes() {
        assert_eq!(count_wrapped_rows(&["a\u{200b}b"], 1), 2);
        assert_eq!(wrapped_suffix_after_rows("a\u{200b}b", 1, 1), "b");
        assert_eq!(count_wrapped_rows(&["a\u{00a0}b"], 2), 2);
        assert_eq!(count_wrapped_rows(&["界a"], 1), 1);
    }

    #[test]
    fn skips_word_and_character_wrapped_rows() {
        assert_eq!(
            wrapped_suffix_after_rows("hi hello world", 7, 1),
            "hello world"
        );
        assert_eq!(wrapped_suffix_after_rows("abcdefghij", 3, 2), "ghij");
        assert_eq!(wrapped_suffix_after_rows("界界界", 2, 2), "界");
        assert_eq!(count_wrapped_rows(&["界界"], 2), 2);
    }
}

pub(super) fn visible_line_range(
    wrap: &WrapCache,
    scroll_y: usize,
    visible_height: u16,
) -> (usize, usize, usize) {
    if wrap.offsets.is_empty() {
        return (0, 0, 0);
    }

    let viewport_end = scroll_y
        .saturating_add(visible_height as usize)
        .saturating_add(1);
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
