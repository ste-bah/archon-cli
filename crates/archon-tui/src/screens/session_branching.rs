//! Session branching / branch picker screen.
//! Layer 1 module — no imports from screens/ or app/.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, List, ListItem};

use crate::virtual_list::VirtualList;
use crate::theme::Theme;

/// Reference to a message / branch point.
#[derive(Debug, Clone)]
pub struct MessageRef {
    pub id: String,
    pub summary: String,
    pub timestamp: String,
}

/// Branch picker for session branching UI.
#[derive(Debug)]
pub struct BranchPicker {
    parent_id: String,
    list: VirtualList<MessageRef>,
}

impl BranchPicker {
    pub fn new(parent_id: String) -> Self {
        Self { parent_id, list: VirtualList::new(Vec::new(), 10) }
    }

    pub fn parent_id(&self) -> &str {
        &self.parent_id
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

    /// Render branch picker into area.
    pub fn render(&self, f: &mut Frame, area: Rect, _theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!("Branch from {}", self.parent_id));

        let items: Vec<ListItem> = self.list.visible_items().iter().map(|m| {
            ListItem::new(format!("{} — {}", m.summary, m.timestamp))
        }).collect();

        let list = List::new(items).block(block);
        f.render_widget(list, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_picker_empty() {
        let picker = BranchPicker::new("parent-1".to_string());
        assert!(picker.is_empty());
        assert_eq!(picker.parent_id(), "parent-1");
    }

    #[test]
    fn set_candidates_updates_list() {
        let mut picker = BranchPicker::new("p".to_string());
        let refs = vec![
            MessageRef { id: "1".into(), summary: "A".into(), timestamp: "10:00".into() },
            MessageRef { id: "2".into(), summary: "B".into(), timestamp: "10:05".into() },
        ];
        picker.set_candidates(refs);
        assert_eq!(picker.len(), 2);
    }

    #[test]
    fn cursor_wraps() {
        let mut picker = BranchPicker::new("p".to_string());
        picker.set_candidates(vec![
            MessageRef { id: "0".into(), summary: "A".into(), timestamp: "10:00".into() },
            MessageRef { id: "1".into(), summary: "B".into(), timestamp: "10:05".into() },
            MessageRef { id: "2".into(), summary: "C".into(), timestamp: "10:10".into() },
        ]);
        picker.move_down();
        assert_eq!(picker.selected_index(), 1);
        picker.move_down();
        assert_eq!(picker.selected_index(), 2);
        picker.move_down();
        assert_eq!(picker.selected_index(), 0); // wrap
    }

    #[test]
    fn single_candidate_wraps() {
        let mut picker = BranchPicker::new("p".to_string());
        picker.set_candidates(vec![
            MessageRef { id: "0".into(), summary: "A".into(), timestamp: "10:00".into() },
        ]);
        picker.move_down();
        assert_eq!(picker.selected_index(), 0); // wrap
        picker.move_up();
        assert_eq!(picker.selected_index(), 0); // wrap
    }
}