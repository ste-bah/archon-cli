//! The `/branch` overlay (#192).
//!
//! Layer 1 module — no imports from screens/ or app/.
//!
//! Restored heavily rewritten. The file that came back imported two types from
//! `session_browser`, a module Phase 0 deleted as genuinely redundant, so it did
//! not compile; and it carried a `SessionBranching` type for switching between
//! branches, which is a different feature from picking where to branch. What is
//! left is the picker, which now has something real to call: `fork_session_at`
//! was added for it, because `fork_session` copies the whole log and there was
//! no way to fork at an earlier point at all.
//!
//! Enter injects `/branch <index>`; the command does the forking.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::widgets::{Row, Table, TableState};

use crate::theme::Theme;
use crate::virtual_list::VirtualList;

/// One message that could be branched from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRef {
    /// Index in the session log — what `/branch` takes and what the fork keeps
    /// through, inclusive.
    pub index: usize,
    /// `user` or `assistant`.
    pub role: String,
    /// One line of the message, enough to recognise it by.
    pub summary: String,
}

impl MessageRef {
    /// The command Enter should put in the prompt.
    pub fn command(&self) -> String {
        format!("/branch {}", self.index)
    }
}

/// Picker over the points a session could be branched from.
#[derive(Debug)]
pub struct BranchPicker {
    list: VirtualList<MessageRef>,
}

impl BranchPicker {
    pub fn new() -> Self {
        Self {
            list: VirtualList::new(Vec::new(), 10),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn selected_index(&self) -> usize {
        self.list.selected_index()
    }

    pub fn selected(&self) -> Option<&MessageRef> {
        self.list.selected()
    }

    pub fn set_candidates(&mut self, candidates: Vec<MessageRef>) {
        self.list.set_items(candidates);
    }

    pub fn move_up(&mut self) {
        self.list.move_up();
    }

    pub fn move_down(&mut self) {
        self.list.move_down();
    }

    pub fn page_up(&mut self) {
        self.list.page_up();
    }

    pub fn page_down(&mut self) {
        self.list.page_down();
    }

    /// Draw the branch points into a centred rect inside `area`.
    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        const TITLE: &str = " Branch — Up/Down select · Enter branch here · Esc close ";

        if self.list.is_empty() {
            crate::overlay::message(
                f,
                area,
                TITLE,
                "This session has no messages to branch from.",
                theme,
            );
            return;
        }

        // rows + header + two border lines.
        let (region, block) =
            crate::overlay::open(f, area, self.list.len() as u16 + 3, TITLE, theme);

        let widths = [
            Constraint::Length(4),
            Constraint::Length(10),
            Constraint::Min(20),
        ];

        let rows: Vec<Row> = self
            .list
            .items()
            .iter()
            .map(|entry| {
                Row::new([
                    entry.index.to_string(),
                    entry.role.clone(),
                    entry.summary.clone(),
                ])
                .style(crate::overlay::body_style(theme))
            })
            .collect();

        let table = Table::new(rows, &widths)
            .header(Row::new(["#", "Role", "Message"]).style(crate::overlay::header_style(theme)))
            .block(block)
            .highlight_symbol(crate::overlay::HIGHLIGHT_SYMBOL)
            .row_highlight_style(crate::overlay::selection_style(theme));

        let mut state = TableState::default().with_selected(Some(self.list.selected_index()));
        f.render_stateful_widget(table, region, &mut state);
    }
}

impl Default for BranchPicker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(index: usize) -> MessageRef {
        MessageRef {
            index,
            role: "user".into(),
            summary: format!("message {index}"),
        }
    }

    #[test]
    fn a_new_picker_is_empty() {
        assert!(BranchPicker::new().is_empty());
    }

    /// The index is the whole payload: `/branch` takes it and the fork keeps
    /// through it.
    #[test]
    fn enter_names_the_index_to_branch_at() {
        assert_eq!(entry(7).command(), "/branch 7");
    }

    #[test]
    fn the_cursor_wraps() {
        let mut picker = BranchPicker::new();
        picker.set_candidates(vec![entry(0), entry(1)]);
        picker.move_down();
        assert_eq!(picker.selected_index(), 1);
        picker.move_down();
        assert_eq!(picker.selected_index(), 0);
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn picker() -> BranchPicker {
        let mut picker = BranchPicker::new();
        picker.set_candidates(vec![
            MessageRef {
                index: 0,
                role: "user".into(),
                summary: "add the parser".into(),
            },
            MessageRef {
                index: 1,
                role: "assistant".into(),
                summary: "here is the parser".into(),
            },
        ]);
        picker
    }

    fn draw(picker: &BranchPicker) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(96, 24)).expect("terminal");
        terminal
            .draw(|frame| picker.render(frame, frame.area(), &crate::theme::dark_theme()))
            .expect("draw branch picker");
        terminal
    }

    fn text(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn style_of(terminal: &Terminal<TestBackend>, needle: &str) -> Option<ratatui::style::Style> {
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        for y in 0..area.height {
            let line: String = (0..area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect();
            if let Some(column) = line.find(needle) {
                return Some(buffer[(column as u16, y)].style());
            }
        }
        None
    }

    #[test]
    fn indices_roles_and_summaries_are_drawn() {
        let rendered = text(&draw(&picker()));
        assert!(rendered.contains("add the parser"), "{rendered}");
        assert!(rendered.contains("assistant"), "{rendered}");
        assert!(rendered.contains("Role"), "header missing: {rendered}");
    }

    #[test]
    fn the_selected_row_is_highlighted_and_moves_with_the_keys() {
        let mut picker = picker();
        let first = draw(&picker);
        let one = style_of(&first, "add the parser").expect("first row");
        let two = style_of(&first, "here is the parser").expect("second row");
        assert_ne!(one, two, "selection is invisible");

        picker.move_down();
        assert_ne!(
            one,
            style_of(&draw(&picker), "add the parser").expect("still drawn"),
            "moving the selection changed nothing on screen"
        );
    }

    #[test]
    fn an_empty_session_is_stated_in_words() {
        assert!(text(&draw(&BranchPicker::new())).contains("no messages to branch from"));
    }

    #[test]
    fn the_overlay_does_not_cover_the_whole_frame() {
        let terminal = draw(&picker());
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        let bottom: String = (0..area.width)
            .map(|x| buffer[(x, area.height - 1)].symbol().to_string())
            .collect();
        assert!(bottom.trim().is_empty(), "painted the status-bar row");
    }
}
