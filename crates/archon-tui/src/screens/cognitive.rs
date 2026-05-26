//! Cognitive executive-loop inspection screen.

use ratatui::Frame;
use ratatui::layout::Rect;

use super::evidence_browser::{EvidenceBrowser, EvidenceRow, title_with_query};
use crate::theme::Theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CognitiveRow {
    pub id: String,
    pub label: String,
    pub status: String,
    pub detail: String,
}

#[derive(Debug)]
pub struct CognitiveScreen {
    browser: EvidenceBrowser<CognitiveRow>,
}

impl CognitiveScreen {
    pub fn executive() -> Self {
        Self {
            browser: EvidenceBrowser::new(12),
        }
    }

    pub fn set_rows(&mut self, rows: Vec<CognitiveRow>) {
        self.browser.set_rows(rows);
    }

    pub fn set_query(&mut self, query: &str) {
        self.browser.set_query(query);
    }

    pub fn len(&self) -> usize {
        self.browser.len()
    }

    pub fn selected(&self) -> Option<&CognitiveRow> {
        self.browser.selected()
    }

    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        self.browser.render(
            f,
            area,
            theme,
            title_with_query("Cognitive Executive State", self.browser.query()),
        );
    }
}

impl EvidenceRow for CognitiveRow {
    fn id(&self) -> &str {
        &self.id
    }

    fn title(&self) -> &str {
        &self.label
    }

    fn status(&self) -> &str {
        &self.status
    }

    fn detail(&self) -> &str {
        &self.detail
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cognitive_screen_accepts_executive_rows() {
        let mut screen = CognitiveScreen::executive();
        screen.set_rows(vec![CognitiveRow {
            id: "decision-1".into(),
            label: "decision".into(),
            status: "selected".into(),
            detail: "code_change -> run_tests".into(),
        }]);
        assert_eq!(screen.len(), 1);
        assert_eq!(screen.selected().unwrap().id, "decision-1");
    }

    #[test]
    fn cognitive_screen_filters_lessons() {
        let mut screen = CognitiveScreen::executive();
        screen.set_rows(vec![CognitiveRow {
            id: "reflection-1".into(),
            label: "reflection".into(),
            status: "lesson".into(),
            detail: "require verification evidence".into(),
        }]);
        screen.set_query("verification");
        assert_eq!(screen.len(), 1);
    }
}
