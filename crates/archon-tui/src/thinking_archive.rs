use crate::app::App;
use crate::output::ThinkingBlock;

impl App {
    pub fn open_thinking_archive(&mut self) {
        self.thinking_archive = self.thinking_blocks.len().checked_sub(1);
    }

    pub fn close_thinking_archive(&mut self) {
        self.thinking_archive = None;
    }

    pub fn select_previous_thinking_block(&mut self) {
        if let Some(selected) = self.thinking_archive.as_mut() {
            *selected = selected.saturating_sub(1);
        }
    }

    pub fn select_next_thinking_block(&mut self) {
        if let Some(selected) = self.thinking_archive.as_mut() {
            *selected = (*selected + 1).min(self.thinking_blocks.len().saturating_sub(1));
        }
    }

    pub fn thinking_archive_selection(&self) -> Option<usize> {
        self.thinking_archive
    }

    pub fn thinking_archive_block(&self) -> Option<&ThinkingBlock> {
        self.thinking_archive
            .and_then(|selected| self.thinking_blocks.get(selected))
    }

    pub fn expand_selected_thinking_block(&mut self) {
        let Some(selected) = self.thinking_archive.take() else {
            return;
        };
        self.expand_thinking_block(selected);
    }

    pub(crate) fn toggle_latest_thinking_block(&mut self) {
        let Some(index) = self.thinking_blocks.len().checked_sub(1) else {
            return;
        };
        if self.thinking_blocks[index].expanded {
            self.collapse_thinking_block(index);
        } else {
            self.expand_thinking_block(index);
        }
    }

    fn expand_thinking_block(&mut self, index: usize) {
        if self
            .thinking_blocks
            .get(index)
            .is_none_or(|block| block.expanded)
        {
            return;
        }
        self.collapse_all_thinking_blocks();
        let (marker_line, lines) = {
            let block = &self.thinking_blocks[index];
            (
                block.marker_line,
                block
                    .text
                    .lines()
                    .map(|line| format!("  {line}"))
                    .collect::<Vec<_>>(),
            )
        };
        self.output.insert_lines_after(marker_line, &lines);
        self.thinking_blocks[index].expanded = true;
        self.shift_markers_after(marker_line, lines.len() as isize);
    }

    pub(crate) fn collapse_all_thinking_blocks(&mut self) {
        if let Some(index) = self.thinking_blocks.iter().position(|block| block.expanded) {
            self.collapse_thinking_block(index);
        }
    }

    fn collapse_thinking_block(&mut self, index: usize) {
        let Some(block) = self.thinking_blocks.get(index) else {
            return;
        };
        if !block.expanded {
            return;
        }
        let marker_line = block.marker_line;
        let line_count = block.text.lines().count();
        self.output.remove_lines_after(marker_line, line_count);
        self.thinking_blocks[index].expanded = false;
        self.shift_markers_after(marker_line, -(line_count as isize));
    }
}
