//! World-model inspection screen.
//!
//! Read-only surface for the local trace world model: backend and model
//! status, candidate/promotion state, and JEPA evaluation runs. Mutating
//! controls (train, promote, rollback, guard) stay behind the `/world`
//! approval gate rather than being driven from this browser — promotion and
//! rollback change which model the advisory runs against.

use ratatui::Frame;
use ratatui::layout::Rect;

use super::evidence_browser::{EvidenceBrowser, EvidenceRow, title_with_query};
use crate::theme::Theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldRow {
    pub id: String,
    pub label: String,
    pub status: String,
    pub detail: String,
}

#[derive(Debug)]
pub struct WorldScreen {
    browser: EvidenceBrowser<WorldRow>,
}

impl WorldScreen {
    pub fn model() -> Self {
        Self {
            browser: EvidenceBrowser::new(12),
        }
    }

    pub fn set_rows(&mut self, rows: Vec<WorldRow>) {
        self.browser.set_rows(rows);
    }

    pub fn set_query(&mut self, query: &str) {
        self.browser.set_query(query);
    }

    pub fn len(&self) -> usize {
        self.browser.len()
    }

    pub fn is_empty(&self) -> bool {
        self.browser.is_empty()
    }

    pub fn selected(&self) -> Option<&WorldRow> {
        self.browser.selected()
    }

    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        self.browser.render(
            f,
            area,
            theme,
            title_with_query("World Model", self.browser.query()),
        );
    }
}

impl EvidenceRow for WorldRow {
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
    fn world_screen_accepts_model_rows() {
        let mut screen = WorldScreen::model();
        screen.set_rows(vec![WorldRow {
            id: "model".into(),
            label: "active model".into(),
            status: "promoted".into(),
            detail: "jepa-v3 (build 3f2f0e11c)".into(),
        }]);

        assert_eq!(screen.len(), 1);
        assert!(!screen.is_empty());
        assert_eq!(screen.selected().map(|row| row.id.as_str()), Some("model"));
    }

    #[test]
    fn world_screen_starts_empty() {
        let screen = WorldScreen::model();
        assert!(screen.is_empty());
        assert!(screen.selected().is_none());
    }
}
