use super::OutputBuffer;
use crate::output::render_cache::count_wrapped_rows;
use crate::theme::Theme;

impl OutputBuffer {
    /// Scroll up by `amount` lines (see earlier content). Locks auto-scroll.
    /// `scroll_offset` = lines scrolled UP from the bottom.
    pub fn scroll_up(&mut self, amount: u16) {
        if !self.scroll_locked {
            self.lock_lines = Some(self.all_lines().into_iter().map(str::to_owned).collect());
        }
        if self.viewport_anchor.take().is_some() || self.has_anchor_delta() {
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
        if self.viewport_anchor.take().is_some() || self.has_anchor_delta() {
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
        self.viewport_anchor.set(None);
        self.scroll_locked = true;
    }

    /// Jump to the bottom and unlock auto-scroll.
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
        self.scroll_from_top = None;
        self.scroll_from_top_tracks_snapshot = false;
        self.scroll_locked = false;
        self.lock_lines = None;
        *self.lock_baseline_cache.borrow_mut() = None;
        self.lock_inserted_lines.clear();
        self.lock_removed_lines.clear();
        self.anchor_inserted_lines.clear();
        self.anchor_removed_lines.clear();
        self.viewport_anchor.set(None);
    }

    /// Compute the actual scroll position for the `Paragraph::scroll()` call.
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
        } else if let Some(anchor) = self.viewport_anchor.get() {
            self.wrap_cache
                .borrow()
                .as_ref()
                .and_then(|cache| {
                    let line_start = *cache.offsets.get(anchor.line_index)?;
                    let line_end = cache
                        .offsets
                        .get(anchor.line_index + 1)
                        .copied()
                        .unwrap_or(cache.total_wrapped);
                    Some(
                        line_start.saturating_add(
                            anchor
                                .wrapped_row_offset
                                .min(line_end.saturating_sub(line_start).saturating_sub(1)),
                        ),
                    )
                })
                .unwrap_or(0)
                .min(max_scroll)
        } else if let Some(position) = self.scroll_from_top {
            if self.scroll_from_top_tracks_snapshot {
                position
                    .saturating_add_signed(self.synthetic_anchor_delta(width, theme))
                    .min(max_scroll)
            } else {
                position.min(max_scroll)
            }
        } else if let Some(lines) = self.lock_lines.as_ref() {
            self.lock_baseline_wrapped_rows(lines, width, theme)
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

    /// Move the viewport to a proportional row inside the output scrollbar.
    pub fn scroll_to_viewport_row(
        &mut self,
        total_wrapped_rows: usize,
        visible_height: u16,
        row: u16,
        area_height: u16,
    ) {
        let max_scroll = total_wrapped_rows.saturating_sub(visible_height as usize);
        if max_scroll == 0 {
            self.scroll_to_bottom();
            return;
        }
        let denominator = area_height.saturating_sub(1).max(1) as usize;
        let row = row.min(area_height.saturating_sub(1)) as usize;
        let global_scroll = row.saturating_mul(max_scroll) / denominator;
        let was_locked = self.scroll_locked;
        self.scroll_offset = max_scroll.saturating_sub(global_scroll);
        self.scroll_locked = self.scroll_offset > 0;
        self.scroll_from_top = self.scroll_locked.then_some(global_scroll);
        self.scroll_from_top_tracks_snapshot = self.scroll_locked;
        self.viewport_anchor.set(None);
        self.anchor_inserted_lines.clear();
        self.anchor_removed_lines.clear();
        if self.scroll_locked && !was_locked {
            self.lock_lines = Some(self.all_lines().into_iter().map(str::to_owned).collect());
            self.lock_inserted_lines.clear();
            self.lock_removed_lines.clear();
        } else if !self.scroll_locked {
            self.lock_lines = None;
            self.lock_inserted_lines.clear();
            self.lock_removed_lines.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrollbar_uses_current_absolute_rows_after_locked_arrivals() {
        let theme = crate::theme::intj_theme();
        let mut buf = OutputBuffer::new();
        for index in 0..100 {
            buf.append_line(&format!("line {index}"));
        }
        buf.rendered_view(&theme, 20, 10);
        buf.scroll_to_top();
        for index in 0..10 {
            buf.append_line(&format!("new {index}"));
        }
        let arrived = buf.rendered_view(&theme, 20, 10);
        assert_eq!(arrived.total_wrapped, 110);

        buf.scroll_to_viewport_row(arrived.total_wrapped, 10, 5, 11);

        assert_eq!(buf.rendered_view(&theme, 20, 10).global_scroll_y, 50);
    }

    #[test]
    fn scroll_to_viewport_row_maps_top_middle_bottom() {
        let mut buf = OutputBuffer::new();
        buf.scroll_to_viewport_row(110, 10, 0, 11);
        assert_eq!(buf.scroll_offset, 100);
        assert!(buf.scroll_locked);

        buf.scroll_to_viewport_row(110, 10, 5, 11);
        assert_eq!(buf.scroll_offset, 50);
        assert!(buf.scroll_locked);

        buf.scroll_to_viewport_row(110, 10, 10, 11);
        assert_eq!(buf.scroll_offset, 0);
        assert!(!buf.scroll_locked);
    }
}
