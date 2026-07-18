use super::OutputBuffer;

impl OutputBuffer {
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
