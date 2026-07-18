use super::buffer::{OutputBuffer, SyntheticLine};

impl OutputBuffer {
    /// Insert complete logical lines directly after an existing transcript line.
    pub fn insert_lines_after(&mut self, index: usize, lines: &[String]) {
        if index >= self.lines.len() || lines.is_empty() {
            return;
        }
        self.lines
            .splice(index + 1..index + 1, lines.iter().cloned());
        if self.scroll_locked {
            let insertion_index = index.saturating_add(1);
            let before_viewport = insertion_index <= self.last_visible_line_start.get();
            for inserted in &mut self.lock_inserted_lines {
                if inserted.current_index >= insertion_index {
                    inserted.current_index = inserted.current_index.saturating_add(lines.len());
                }
            }
            self.lock_inserted_lines
                .extend(
                    lines
                        .iter()
                        .enumerate()
                        .map(|(offset, text)| SyntheticLine {
                            text: text.clone(),
                            current_index: insertion_index.saturating_add(offset),
                        }),
                );
            if before_viewport {
                self.anchor_inserted_lines.extend(lines.iter().cloned());
            }
        }
        self.mark_dirty();
    }

    /// Remove a bounded range of complete logical lines directly after an existing line.
    pub fn remove_lines_after(&mut self, index: usize, count: usize) {
        let start = index.saturating_add(1);
        let end = start.saturating_add(count).min(self.lines.len());
        if start >= end {
            return;
        }
        let removed = self.lines.drain(start..end).collect::<Vec<_>>();
        if self.scroll_locked {
            let before_viewport = start < self.last_visible_line_start.get();
            for (offset, line) in removed.iter().enumerate() {
                let current_index = start.saturating_add(offset);
                if let Some(position) = self.lock_inserted_lines.iter().position(|inserted| {
                    inserted.current_index == current_index && inserted.text == *line
                }) {
                    self.lock_inserted_lines.remove(position);
                    if before_viewport
                        && let Some(anchor_position) = self
                            .anchor_inserted_lines
                            .iter()
                            .position(|inserted| inserted == line)
                    {
                        self.anchor_inserted_lines.remove(anchor_position);
                    }
                } else {
                    self.lock_removed_lines.push(line.clone());
                    if before_viewport {
                        self.anchor_removed_lines.push(line.clone());
                    }
                }
            }
            for inserted in &mut self.lock_inserted_lines {
                if inserted.current_index >= end {
                    inserted.current_index = inserted.current_index.saturating_sub(end - start);
                }
            }
        }
        self.mark_dirty();
    }
}
