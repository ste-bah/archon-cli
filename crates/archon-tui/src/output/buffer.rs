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

#[derive(Debug, Clone, Copy)]
pub(super) struct ViewportAnchor {
    pub(super) line_index: usize,
    pub(super) wrapped_row_offset: usize,
}

#[derive(Debug, Clone)]
pub(super) struct LockBaselineCache {
    width: u16,
    theme: Theme,
    total_wrapped: usize,
}

/// Output buffer -- append-only text buffer for streaming display.
#[derive(Debug)]
pub struct OutputBuffer {
    pub(super) lines: Vec<String>,
    current_line: String,
    revision: u64,
    pub(super) render_cache: RefCell<Option<RenderCache>>,
    pub(super) wrap_cache: RefCell<Option<WrapCache>>,
    append_render_dirty_from: Cell<Option<usize>>,
    append_wrap_dirty_from: Cell<Option<usize>>,
    #[cfg(test)]
    pub(super) rendered_line_work: Cell<usize>,
    #[cfg(test)]
    pub(super) wrapped_line_work: Cell<usize>,
    /// Vertical scroll offset (lines from the top). When `scroll_locked` is
    /// false this is ignored and we auto-scroll to the bottom.
    pub scroll_offset: usize,
    /// When true the user has scrolled away from the bottom; new content does
    /// not auto-scroll.
    pub scroll_locked: bool,
    /// Logical transcript snapshot captured when scrolling first locks.
    pub(super) lock_lines: Option<Vec<String>>,
    pub(super) lock_baseline_cache: RefCell<Option<LockBaselineCache>>,
    #[cfg(test)]
    pub(super) lock_baseline_line_work: Cell<usize>,
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
    pub(super) last_max_scroll: Cell<usize>,
    /// Last rendered global row and logical line, used to anchor synthetic edits.
    pub(super) last_global_scroll_y: Cell<usize>,
    pub(super) last_visible_line_start: Cell<usize>,
    pub(super) viewport_anchor: Cell<Option<ViewportAnchor>>,
}

impl Default for OutputBuffer {
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            current_line: String::new(),
            revision: 0,
            render_cache: RefCell::new(None),
            wrap_cache: RefCell::new(None),
            append_render_dirty_from: Cell::new(None),
            append_wrap_dirty_from: Cell::new(None),
            #[cfg(test)]
            rendered_line_work: Cell::new(0),
            #[cfg(test)]
            wrapped_line_work: Cell::new(0),
            scroll_offset: 0,
            scroll_locked: false,
            lock_lines: None,
            lock_baseline_cache: RefCell::new(None),
            #[cfg(test)]
            lock_baseline_line_work: Cell::new(0),
            lock_inserted_lines: Vec::new(),
            lock_removed_lines: Vec::new(),
            anchor_inserted_lines: Vec::new(),
            anchor_removed_lines: Vec::new(),
            scroll_from_top: None,
            scroll_from_top_tracks_snapshot: false,
            last_max_scroll: Cell::new(0),
            last_global_scroll_y: Cell::new(0),
            last_visible_line_start: Cell::new(0),
            viewport_anchor: Cell::new(None),
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
        let dirty_from = self.lines.len();
        for ch in text.chars() {
            if ch == '\n' {
                self.lines.push(std::mem::take(&mut self.current_line));
            } else {
                self.current_line.push(ch);
            }
        }
        self.mark_append_dirty(dirty_from);
    }

    /// Append a complete line.
    pub fn append_line(&mut self, line: &str) {
        let line = sanitize_output_text(line);
        let dirty_from = self.lines.len();
        if !self.current_line.is_empty() {
            self.lines.push(std::mem::take(&mut self.current_line));
        }
        for segment in line.split('\n') {
            self.lines.push(segment.to_string());
        }
        self.mark_append_dirty(dirty_from);
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
        *self.lock_baseline_cache.borrow_mut() = None;
        self.lock_inserted_lines.clear();
        self.lock_removed_lines.clear();
        self.anchor_inserted_lines.clear();
        self.anchor_removed_lines.clear();
        self.scroll_from_top = None;
        self.scroll_from_top_tracks_snapshot = false;
        self.last_max_scroll.set(0);
        self.last_global_scroll_y.set(0);
        self.last_visible_line_start.set(0);
        self.viewport_anchor.set(None);
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
        if self.scroll_locked {
            self.viewport_anchor.set(Some(ViewportAnchor {
                line_index: start,
                wrapped_row_offset: paragraph_scroll_y,
            }));
        }

        let (lines, paragraph_scroll_y) =
            self.visible_lines(start, end, paragraph_scroll_y, width, visible_height);
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
        visible_height: u16,
    ) -> (Vec<Line<'static>>, u16) {
        let mut lines = self
            .render_cache
            .borrow()
            .as_ref()
            .map(|cache| cache.lines[start..end].to_vec())
            .unwrap_or_default();
        let max_paragraph_scroll = u16::MAX.saturating_sub(visible_height);
        if paragraph_scroll_y <= max_paragraph_scroll as usize {
            return (lines, paragraph_scroll_y as u16);
        }
        let skipped_rows = paragraph_scroll_y - max_paragraph_scroll as usize;
        if let Some(first) = lines.first_mut()
            && let Some(cache) = self.render_cache.borrow().as_ref()
            && let (Some(line), Some(text)) =
                (cache.lines.get(start), cache.rendered_text.get(start))
        {
            let suffix = wrapped_suffix_after_rows(text, width, skipped_rows);
            *first = line_suffix(line, text.len().saturating_sub(suffix.len()));
        }
        (lines, max_paragraph_scroll)
    }

    pub fn new_wrapped_rows(&self, total_wrapped: usize, width: u16, theme: &Theme) -> usize {
        let Some(lines) = self.lock_lines.as_ref() else {
            return 0;
        };
        let baseline = self.lock_baseline_wrapped_rows(lines, width, theme);
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

    pub(super) fn lock_baseline_wrapped_rows(
        &self,
        lines: &[String],
        width: u16,
        theme: &Theme,
    ) -> usize {
        if let Some(cache) = self.lock_baseline_cache.borrow().as_ref()
            && cache.width == width
            && cache.theme == *theme
        {
            return cache.total_wrapped;
        }

        let total_wrapped = Self::count_rendered_wrapped_rows(lines, width, theme);
        #[cfg(test)]
        self.lock_baseline_line_work
            .set(self.lock_baseline_line_work.get() + lines.len());
        *self.lock_baseline_cache.borrow_mut() = Some(LockBaselineCache {
            width,
            theme: theme.clone(),
            total_wrapped,
        });
        total_wrapped
    }

    pub(super) fn count_rendered_wrapped_rows(
        lines: &[String],
        width: u16,
        theme: &Theme,
    ) -> usize {
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
        self.append_render_dirty_from.set(None);
        self.append_wrap_dirty_from.set(None);
        *self.render_cache.borrow_mut() = None;
        *self.wrap_cache.borrow_mut() = None;
    }

    fn mark_append_dirty(&mut self, dirty_from: usize) {
        self.revision = self.revision.wrapping_add(1);
        Self::merge_dirty_from(&self.append_render_dirty_from, dirty_from);
        Self::merge_dirty_from(&self.append_wrap_dirty_from, dirty_from);
    }

    fn merge_dirty_from(dirty: &Cell<Option<usize>>, dirty_from: usize) {
        dirty.set(Some(
            dirty
                .get()
                .map_or(dirty_from, |existing| existing.min(dirty_from)),
        ));
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

        let total_lines = self.line_count();
        let dirty_from = self.append_render_dirty_from.get().filter(|_| {
            self.render_cache
                .borrow()
                .as_ref()
                .is_some_and(|cache| cache.theme == *theme)
        });
        let mut cache_ref = self.render_cache.borrow_mut();
        let cache = cache_ref.get_or_insert_with(|| RenderCache {
            revision: self.revision,
            theme: theme.clone(),
            lines: Vec::with_capacity(total_lines),
            raw_lines: Vec::with_capacity(total_lines),
            rendered_text: Vec::with_capacity(total_lines),
        });
        let start = dirty_from.unwrap_or(0).min(cache.lines.len());
        cache.lines.truncate(start);
        cache.raw_lines.truncate(start);
        cache.rendered_text.truncate(start);
        for index in start..total_lines {
            let raw_line = self
                .lines
                .get(index)
                .map(String::as_str)
                .unwrap_or(self.current_line.as_str());
            let line = render_markdown_line(raw_line, theme);
            cache.rendered_text.push(line_text(&line));
            cache.lines.push(line);
            cache.raw_lines.push(raw_line.to_string());
        }
        #[cfg(test)]
        self.rendered_line_work
            .set(self.rendered_line_work.get() + total_lines - start);
        cache.revision = self.revision;
        cache.theme = theme.clone();
        self.append_render_dirty_from.set(None);
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

        let render_ref = self.render_cache.borrow();
        let rendered_text = render_ref
            .as_ref()
            .map(|cache| cache.rendered_text.as_slice())
            .unwrap_or_default();
        let dirty_from = self.append_wrap_dirty_from.get().filter(|_| {
            self.wrap_cache
                .borrow()
                .as_ref()
                .is_some_and(|cache| cache.width == width)
        });
        let mut wrap_ref = self.wrap_cache.borrow_mut();
        let wrap = wrap_ref.get_or_insert_with(|| WrapCache {
            revision: self.revision,
            width,
            offsets: Vec::with_capacity(rendered_text.len()),
            total_wrapped: 0,
        });
        let start = dirty_from.unwrap_or(0).min(wrap.offsets.len());
        wrap.offsets.truncate(start);
        let mut total = if start == 0 {
            0
        } else {
            wrap.offsets[start - 1].saturating_add(
                count_wrapped_rows(&[rendered_text[start - 1].as_str()], width).max(1),
            )
        };
        for text in &rendered_text[start..] {
            wrap.offsets.push(total);
            total = total.saturating_add(count_wrapped_rows(&[text.as_str()], width).max(1));
        }
        #[cfg(test)]
        self.wrapped_line_work
            .set(self.wrapped_line_work.get() + rendered_text.len() - start);
        wrap.revision = self.revision;
        wrap.width = width;
        wrap.total_wrapped = total;
        self.append_wrap_dirty_from.set(None);
    }
}
