//! TASK-TUI-627 skills-menu overlay (screen module).
//!
//! Mirrors `screens/message_selector.rs` (TUI-620) exactly — scrollable
//! list with up/down navigation. Render + input routing deferred to
//! TUI-627-followup.

use crate::events::SkillEntry;

pub struct SkillsMenu {
    pub skills: Vec<SkillEntry>,
    pub selected_index: usize,
}

impl SkillsMenu {
    pub fn new(skills: Vec<SkillEntry>) -> Self {
        Self { skills, selected_index: 0 }
    }

    pub fn select_next(&mut self) {
        if self.skills.is_empty() { return; }
        self.selected_index = (self.selected_index + 1) % self.skills.len();
    }

    pub fn select_prev(&mut self) {
        if self.skills.is_empty() { return; }
        self.selected_index = if self.selected_index == 0 {
            self.skills.len() - 1
        } else {
            self.selected_index - 1
        };
    }

    pub fn selected(&self) -> Option<&SkillEntry> {
        self.skills.get(self.selected_index)
    }

    // TODO(TUI-627-followup): render(frame, area) — numbered list,
    // reverse-video on selected_index. See screens/message_selector.rs.
    // TODO(TUI-627-followup): on-enter handler — inject /skill-name
    // into state.prompt_input_buffer. Requires input priority-branch
    // routing in event_loop/input.rs.
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(n: usize) -> Vec<SkillEntry> {
        (0..n).map(|i| SkillEntry {
            name: format!("skill-{}", i),
            description: format!("desc-{}", i),
        }).collect()
    }

    #[test]
    fn new_starts_at_zero() {
        let m = SkillsMenu::new(fixture(3));
        assert_eq!(m.selected_index, 0);
    }

    #[test]
    fn select_next_advances() {
        let mut m = SkillsMenu::new(fixture(3));
        m.select_next();
        assert_eq!(m.selected_index, 1);
    }

    #[test]
    fn select_next_wraps_at_end() {
        let mut m = SkillsMenu::new(fixture(3));
        m.selected_index = 2;
        m.select_next();
        assert_eq!(m.selected_index, 0);
    }

    #[test]
    fn select_prev_wraps_at_start() {
        let mut m = SkillsMenu::new(fixture(3));
        m.select_prev();
        assert_eq!(m.selected_index, 2);
    }

    #[test]
    fn empty_list_noop() {
        let mut m = SkillsMenu::new(vec![]);
        m.select_next();
        m.select_prev();
        assert_eq!(m.selected_index, 0);
        assert!(m.selected().is_none());
    }
}
