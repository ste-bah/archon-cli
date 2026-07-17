use crate::app::App;

impl App {
    /// Store a safe, preflight-validated summary for a matching tool call.
    pub fn set_tool_summary(&mut self, id: &str, summary: Option<String>) {
        if let Some(tool) = self.tool_outputs.iter_mut().find(|tool| tool.tool_id == id) {
            tool.summary = summary;
        }
    }

    pub(crate) fn shift_markers_after(&mut self, marker_line: usize, delta: isize) {
        for tool in &mut self.tool_outputs {
            if let Some(line) = tool.marker_line.as_mut()
                && *line > marker_line
            {
                *line = line.saturating_add_signed(delta);
            }
        }
        for block in &mut self.thinking_blocks {
            if block.marker_line > marker_line {
                block.marker_line = block.marker_line.saturating_add_signed(delta);
            }
        }
    }

    pub fn toggle_tool_output(&mut self, index: Option<usize>) {
        let index = index.or_else(|| self.tool_outputs.len().checked_sub(1));
        let Some(index) = index else {
            return;
        };
        let Some(tool) = self.tool_outputs.get(index) else {
            return;
        };
        if tool.marker_line.is_none() {
            return;
        }
        if tool.expanded {
            self.collapse_tool_output(index);
        } else {
            self.expand_tool_output(index);
        }
    }

    fn expand_tool_output(&mut self, index: usize) {
        let (marker_line, lines) = {
            let tool = &self.tool_outputs[index];
            (
                tool.marker_line.expect("checked marker"),
                tool.excerpt_lines(),
            )
        };
        self.output.insert_lines_after(marker_line, &lines);
        self.tool_outputs[index].expanded = true;
        self.tool_outputs[index].expanded_line_count = lines.len();
        self.shift_markers_after(marker_line, lines.len() as isize);
    }

    fn collapse_tool_output(&mut self, index: usize) {
        let marker_line = self.tool_outputs[index]
            .marker_line
            .expect("checked marker");
        let line_count = self.tool_outputs[index].expanded_line_count;
        self.output.remove_lines_after(marker_line, line_count);
        self.tool_outputs[index].expanded = false;
        self.tool_outputs[index].expanded_line_count = 0;
        self.shift_markers_after(marker_line, -(line_count as isize));
    }
}
