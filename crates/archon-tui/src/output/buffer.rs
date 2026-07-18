//! Append-only streaming output buffer with scroll and word-wrap math.
//!
//! Relocated from `src/output.rs` (OutputBuffer section, L210-L374 + tests
//! L380-L495) per REM-2h.

use std::cell::{Cell, RefCell};

use ratatui::text::Line;

use crate::markdown::render_markdown_line;
use crate::output::render_cache::{
    RenderCache, RenderedOutputView, WrapCache, count_wrapped_rows, line_suffix, line_text,
    visible_line_range, wrapped_suffix_after_rows,
};
use crate::output::sanitize::sanitize_output_text;
use crate::theme::Theme;

#[derive(Debug, Clone)]
pub(super) struct SyntheticLine {
    pub(super) text: String,
    pub(super) current_index: usize,
}

/// Output buffer -- append-only text buffer for streaming display.
#[derive(Debug)]
pub struct OutputBuffer {
    pub(super) lines: Vec<String>,
    current_line: String,
    revision: u64,
    render_cache: RefCell<Option<RenderCache>>,
    wrap_cache: RefCell<Option<WrapCache>>,
    /// Vertical scroll offset (lines from the top). When `scroll_locked` is
    /// false this is ignored and we auto-scroll to the bottom.
    pub scroll_offset: usize,
    /// When true the user has scrolled away from the bottom; new content does
    /// not auto-scroll.
    pub scroll_locked: bool,
    /// Logical transcript snapshot captured when scrolling first locks.
    pub(super) lock_lines: Option<Vec<String>>,
    /// Synthetic transcript rows inserted since scrolling locked.
    pub(super) lock_inserted_lines: Vec<SyntheticLine>,
    /// Synthetic transcript rows removed since scrolling locked.
    pub(super) lock_removed_lines: Vec<String>,
    /// Synthetic rows inserted/removed before the current viewport anchor.
    pub(super) anchor_inserted_lines: Vec<String>,
    pub(super) anchor_removed_lines: Vec<String>,
    /// Absolute wrapped-row position used after jumping to the transcript top.
    pub(super) scroll_from_top: Option<usize>,
    /// Whether the absolute row follows lock-snapshot synthetic edits.
    pub(super) scroll_from_top_tracks_snapshot: bool,
    /// Last rendered maximum scroll, used to unlock top-origin navigation.
    last_max_scroll: Cell<usize>,
    /// Last rendered global row and logical line, used to anchor synthetic edits.
    pub(super) last_global_scroll_y: Cell<usize>,
    pub(super) last_visible_line_start: Cell<usize>,
}

impl Default for OutputBuffer {
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            current_line: String::new(),
            revision: 0,
            render_cache: RefCell::new(None),
            wrap_cache: RefCell::new(None),
            scroll_offset: 0,
            scroll_locked: false,
            lock_lines: None,
            lock_inserted_lines: Vec::new(),
            lock_removed_lines: Vec::new(),
            anchor_inserted_lines: Vec::new(),
            anchor_removed_lines: Vec::new(),
            scroll_from_top: None,
            scroll_from_top_tracks_snapshot: false,
            last_max_scroll: Cell::new(0),
            last_global_scroll_y: Cell::new(0),
            last_visible_line_start: Cell::new(0),
        }
    }
}

impl OutputBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append text (may contain newlines).
    pub fn append(&mut self, text: &str) {
        let text = sanitize_output_text(text);
        if text.is_empty() {
            return;
        }
        for ch in text.chars() {
            if ch == '\n' {
                self.lines.push(std::mem::take(&mut self.current_line));
            } else {
                self.current_line.push(ch);
            }
        }
        self.mark_dirty();
    }

    /// Append a complete line.
    pub fn append_line(&mut self, line: &str) {
        let line = sanitize_output_text(line);
        if !self.current_line.is_empty() {
            self.lines.push(std::mem::take(&mut self.current_line));
        }
        for segment in line.split('\n') {
            self.lines.push(segment.to_string());
        }
        self.mark_dirty();
    }

    /// Get all completed lines plus the current partial line.
    pub fn all_lines(&self) -> Vec<&str> {
        let mut result: Vec<&str> = self.lines.iter().map(|s| s.as_str()).collect();
        if !self.current_line.is_empty() {
            result.push(&self.current_line);
        }
        result
    }

    /// Total line count (including partial current line).
    pub fn line_count(&self) -> usize {
        self.lines.len() + if self.current_line.is_empty() { 0 } else { 1 }
    }

    /// Clear all content.
    pub fn clear(&mut self) {
        self.lines.clear();
        self.current_line.clear();
        self.scroll_offset = 0;
        self.scroll_locked = false;
        self.lock_lines = None;
        self.lock_inserted_lines.clear();
        self.lock_removed_lines.clear();
        self.anchor_inserted_lines.clear();
        self.anchor_removed_lines.clear();
        self.scroll_from_top = None;
        self.scroll_from_top_tracks_snapshot = false;
        self.last_max_scroll.set(0);
        self.last_global_scroll_y.set(0);
        self.last_visible_line_start.set(0);
        self.mark_dirty();
    }

    /// Render output lines with a revision/theme cache so the draw loop does
    /// not re-run markdown parsing for the whole transcript every frame.
    pub fn rendered_lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        self.refresh_render_cache(theme);
        self.render_cache
            .borrow()
            .as_ref()
            .map(|cache| cache.lines.clone())
            .unwrap_or_default()
    }

    /// Plain text matching [`Self::rendered_lines`], used for wrapped-row
    /// scroll math without re-stringifying ratatui spans every draw.
    pub fn rendered_raw_lines(&self, theme: &Theme) -> Vec<String> {
        self.refresh_render_cache(theme);
        self.render_cache
            .borrow()
            .as_ref()
            .map(|cache| cache.raw_lines.clone())
            .unwrap_or_default()
    }

    /// Return only the logical lines needed for the current viewport.
    ///
    /// Markdown parsing and wrap-row offsets are cached by content revision,
    /// theme, and width. Per-frame work is reduced to a binary-ish scan over
    /// cached offsets plus cloning the visible line slice.
    pub fn rendered_view(
        &self,
        theme: &Theme,
        width: u16,
        visible_height: u16,
    ) -> RenderedOutputView {
        self.refresh_wrap_cache(theme, width);

        let total_wrapped = self
            .wrap_cache
            .borrow()
            .as_ref()
            .map(|cache| cache.total_wrapped)
            .unwrap_or(0);
        self.last_max_scroll
            .set(total_wrapped.saturating_sub(visible_height as usize));
        let global_scroll_y = self.effective_scroll(total_wrapped, visible_height, width, theme);

        let (start, end, paragraph_scroll_y) = {
            let wrap_ref = self.wrap_cache.borrow();
            let Some(wrap) = wrap_ref.as_ref() else {
                return RenderedOutputView {
                    lines: Vec::new(),
                    total_wrapped,
                    global_scroll_y,
                    paragraph_scroll_y: 0,
                };
            };
            visible_line_range(wrap, global_scroll_y, visible_height)
        };

        self.last_global_scroll_y.set(global_scroll_y);
        self.last_visible_line_start.set(start);

        let (lines, paragraph_scroll_y) = self.visible_lines(start, end, paragraph_scroll_y, width);
        RenderedOutputView {
            lines,
            total_wrapped,
            global_scroll_y,
            paragraph_scroll_y,
        }
    }

    fn visible_lines(
        &self,
        start: usize,
        end: usize,
        paragraph_scroll_y: usize,
        width: u16,
    ) -> (Vec<Line<'static>>, u16) {
        let mut lines = self
            .render_cache
            .borrow()
            .as_ref()
            .map(|cache| cache.lines[start..end].to_vec())
            .unwrap_or_default();
        if paragraph_scroll_y <= u16::MAX as usize {
            return (lines, paragraph_scroll_y as u16);
        }
        let skipped_rows = paragraph_scroll_y - u16::MAX as usize;
        if let Some(first) = lines.first_mut()
            && let Some(cache) = self.render_cache.borrow().as_ref()
            && let (Some(line), Some(text)) =
                (cache.lines.get(start), cache.rendered_text.get(start))
        {
            let suffix = wrapped_suffix_after_rows(text, width, skipped_rows);
            *first = line_suffix(line, text.len().saturating_sub(suffix.len()));
        }
        (lines, u16::MAX)
    }

    pub fn new_wrapped_rows(&self, total_wrapped: usize, width: u16, theme: &Theme) -> usize {
        let Some(lines) = self.lock_lines.as_ref() else {
            return 0;
        };
        let baseline = Self::count_rendered_wrapped_rows(lines, width, theme);
        let inserted = Self::count_rendered_wrapped_rows(
            &self
                .lock_inserted_lines
                .iter()
                .map(|line| line.text.clone())
                .collect::<Vec<_>>(),
            width,
            theme,
        );
        let removed = Self::count_rendered_wrapped_rows(&self.lock_removed_lines, width, theme);
        total_wrapped
            .saturating_add(removed)
            .saturating_sub(baseline.saturating_add(inserted))
    }

    fn count_rendered_wrapped_rows(lines: &[String], width: u16, theme: &Theme) -> usize {
        let rendered = lines
            .iter()
            .map(|line| line_text(&render_markdown_line(line, theme)))
            .collect::<Vec<_>>();
        count_wrapped_rows(
            &rendered.iter().map(String::as_str).collect::<Vec<_>>(),
            width,
        )
    }

    pub(super) fn mark_dirty(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        *self.render_cache.borrow_mut() = None;
        *self.wrap_cache.borrow_mut() = None;
    }

    fn refresh_render_cache(&self, theme: &Theme) {
        let cache_is_current = self
            .render_cache
            .borrow()
            .as_ref()
            .map(|cache| cache.revision == self.revision && cache.theme == *theme)
            .unwrap_or(false);
        if cache_is_current {
            return;
        }

        let raw_lines = self
            .all_lines()
            .into_iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let lines: Vec<_> = raw_lines
            .iter()
            .map(|line| render_markdown_line(line, theme))
            .collect();
        let rendered_text = lines.iter().map(line_text).collect();
        *self.render_cache.borrow_mut() = Some(RenderCache {
            revision: self.revision,
            theme: theme.clone(),
            lines,
            raw_lines,
            rendered_text,
        });
    }

    fn refresh_wrap_cache(&self, theme: &Theme, width: u16) {
        self.refresh_render_cache(theme);
        let cache_is_current = self
            .wrap_cache
            .borrow()
            .as_ref()
            .map(|cache| cache.revision == self.revision && cache.width == width)
            .unwrap_or(false);
        if cache_is_current {
            return;
        }

        let rendered_text = self
            .render_cache
            .borrow()
            .as_ref()
            .map(|cache| cache.rendered_text.clone())
            .unwrap_or_default();
        let mut offsets = Vec::with_capacity(rendered_text.len());
        let mut total: usize = 0;
        for text in &rendered_text {
            offsets.push(total);
            let rows = count_wrapped_rows(&[text.as_str()], width).max(1);
            total = total.saturating_add(rows);
        }
        *self.wrap_cache.borrow_mut() = Some(WrapCache {
            revision: self.revision,
            width,
            offsets,
            total_wrapped: total,
        });
    }

    // -- scroll helpers -----------------------------------------------------

    /// Scroll up by `amount` lines (see earlier content). Locks auto-scroll.
    /// `scroll_offset` = lines scrolled UP from the bottom.
    pub fn scroll_up(&mut self, amount: u16) {
        if !self.scroll_locked {
            self.lock_lines = Some(self.all_lines().into_iter().map(str::to_owned).collect());
        }
        if self.has_anchor_delta() {
            self.scroll_from_top = Some(
                self.last_global_scroll_y
                    .get()
                    .saturating_sub(amount as usize),
            );
            self.clear_anchor_delta();
        } else if let Some(position) = self.scroll_from_top.as_mut() {
            *position = position.saturating_sub(amount as usize);
        } else {
            self.scroll_offset = self.scroll_offset.saturating_add(amount as usize);
        }
        self.scroll_locked = true;
    }

    /// Scroll down by `amount` lines (toward newer content).
    /// If offset reaches 0, unlocks auto-scroll.
    pub fn scroll_down(&mut self, amount: u16) {
        if self.has_anchor_delta() {
            let position = self
                .last_global_scroll_y
                .get()
                .saturating_add(amount as usize);
            self.clear_anchor_delta();
            if position >= self.last_max_scroll.get() {
                self.scroll_to_bottom();
            } else {
                self.scroll_from_top = Some(position);
            }
            return;
        }
        if let Some(position) = self.scroll_from_top.as_mut() {
            *position = position.saturating_add(amount as usize);
            if *position >= self.last_max_scroll.get() {
                self.scroll_to_bottom();
            }
            return;
        }
        self.scroll_offset = self.scroll_offset.saturating_sub(amount as usize);
        if self.scroll_offset == 0 {
            self.scroll_to_bottom();
        }
    }

    pub fn scroll_to_top(&mut self) {
        if !self.scroll_locked {
            self.lock_lines = Some(self.all_lines().into_iter().map(str::to_owned).collect());
        }
        self.scroll_offset = 0;
        self.scroll_from_top = Some(0);
        self.scroll_from_top_tracks_snapshot = true;
        self.scroll_locked = true;
    }

    /// Jump to the bottom and unlock auto-scroll.
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
        self.scroll_from_top = None;
        self.scroll_from_top_tracks_snapshot = false;
        self.scroll_locked = false;
        self.lock_lines = None;
        self.lock_inserted_lines.clear();
        self.lock_removed_lines.clear();
        self.anchor_inserted_lines.clear();
        self.anchor_removed_lines.clear();
    }

    /// Compute the actual scroll position for the `Paragraph::scroll()` call.
    ///
    /// `scroll_offset` = lines scrolled UP from the bottom.
    /// `Paragraph::scroll((y, 0))` expects physical rows from the TOP.
    /// NOTE: ratatui does NOT clamp — passing a value past content shows blank.
    ///
    /// When not scroll-locked: auto-scroll to bottom (return max_scroll).
    /// When locked: return max_scroll - scroll_offset (clamped to zero).
    pub fn effective_scroll(
        &self,
        total_wrapped_rows: usize,
        visible_height: u16,
        width: u16,
        theme: &Theme,
    ) -> usize {
        let max_scroll = total_wrapped_rows.saturating_sub(visible_height as usize);
        if !self.scroll_locked {
            max_scroll
        } else if let Some(position) = self.scroll_from_top {
            if self.scroll_from_top_tracks_snapshot {
                position
                    .saturating_add_signed(self.synthetic_anchor_delta(width, theme))
                    .min(max_scroll)
            } else {
                position.min(max_scroll)
            }
        } else if let Some(lines) = self.lock_lines.as_ref() {
            Self::count_rendered_wrapped_rows(lines, width, theme)
                .saturating_sub(visible_height as usize)
                .saturating_sub(self.scroll_offset)
                .saturating_add_signed(self.synthetic_anchor_delta(width, theme))
                .min(max_scroll)
        } else {
            max_scroll.saturating_sub(self.scroll_offset)
        }
    }

    fn has_anchor_delta(&self) -> bool {
        !self.anchor_inserted_lines.is_empty() || !self.anchor_removed_lines.is_empty()
    }

    fn clear_anchor_delta(&mut self) {
        self.anchor_inserted_lines.clear();
        self.anchor_removed_lines.clear();
        self.scroll_from_top_tracks_snapshot = false;
    }

    fn synthetic_anchor_delta(&self, width: u16, theme: &Theme) -> isize {
        let inserted =
            Self::count_rendered_wrapped_rows(&self.anchor_inserted_lines, width, theme) as isize;
        let removed =
            Self::count_rendered_wrapped_rows(&self.anchor_removed_lines, width, theme) as isize;
        inserted.saturating_sub(removed)
    }

    /// Compatibility wrapper retaining the original public `u16` API.
    pub fn count_wrapped_rows(lines: &[&str], width: u16) -> u16 {
        count_wrapped_rows(lines, width).min(u16::MAX as usize) as u16
    }
}
