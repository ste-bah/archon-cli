use super::OutputBuffer;

impl OutputBuffer {
    /// Move the viewport to a proportional row inside the output scrollbar.
    pub fn scroll_to_viewport_row(
        &mut self,
        total_wrapped_rows: u16,
        visible_height: u16,
        row: u16,
        area_height: u16,
    ) {
        let max_scroll = total_wrapped_rows.saturating_sub(visible_height);
        if max_scroll == 0 {
            self.scroll_to_bottom();
            return;
        }
        let denominator = area_height.saturating_sub(1).max(1);
        let row = row.min(denominator);
        let global_scroll = ((row as u32 * max_scroll as u32) / denominator as u32) as u16;
        self.scroll_offset = max_scroll.saturating_sub(global_scroll);
        self.scroll_locked = self.scroll_offset > 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
