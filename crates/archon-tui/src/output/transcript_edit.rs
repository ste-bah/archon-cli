use super::buffer::OutputBuffer;

impl OutputBuffer {
    /// Insert complete logical lines directly after an existing transcript line.
    pub fn insert_lines_after(&mut self, index: usize, lines: &[String]) {
        if index >= self.lines.len() || lines.is_empty() {
            return;
        }
        self.lines
            .splice(index + 1..index + 1, lines.iter().cloned());
        self.mark_dirty();
    }

    /// Remove a bounded range of complete logical lines directly after an existing line.
    pub fn remove_lines_after(&mut self, index: usize, count: usize) {
        let start = index.saturating_add(1);
        let end = start.saturating_add(count).min(self.lines.len());
        if start >= end {
            return;
        }
        self.lines.drain(start..end);
        self.mark_dirty();
    }
}
