//! Governed-learning inspection screens.
//! Layer 1 module - no imports from sibling screens or app.

use ratatui::Frame;
use ratatui::layout::Rect;

use super::evidence_browser::{EvidenceBrowser, EvidenceRow, title_with_query};
use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LearningView {
    Proposals,
    Manifests,
    Incidents,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LearningRow {
    pub id: String,
    pub kind: String,
    pub state: String,
    pub evidence: String,
}

pub trait LearningStore: Send + Sync {
    fn load_rows(&self, view: LearningView) -> Vec<LearningRow>;
}

#[derive(Debug)]
pub struct LearningScreen {
    view: LearningView,
    browser: EvidenceBrowser<LearningRow>,
}

impl LearningScreen {
    pub fn proposals() -> Self {
        Self::new(LearningView::Proposals)
    }

    pub fn manifests() -> Self {
        Self::new(LearningView::Manifests)
    }

    pub fn incidents() -> Self {
        Self::new(LearningView::Incidents)
    }

    pub fn new(view: LearningView) -> Self {
        Self {
            view,
            browser: EvidenceBrowser::new(12),
        }
    }

    pub fn load_from<S: LearningStore>(&mut self, store: &S) {
        self.set_rows(store.load_rows(self.view));
    }

    pub fn set_rows(&mut self, rows: Vec<LearningRow>) {
        self.browser.set_rows(rows);
    }

    pub fn set_query(&mut self, query: &str) {
        self.browser.set_query(query);
    }

    pub fn len(&self) -> usize {
        self.browser.len()
    }

    pub fn selected(&self) -> Option<&LearningRow> {
        self.browser.selected()
    }

    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        self.browser.render(f, area, theme, self.title());
    }

    fn title(&self) -> String {
        let base = match self.view {
            LearningView::Proposals => "Learning Proposals",
            LearningView::Manifests => "Learning Manifests",
            LearningView::Incidents => "Completion Incidents",
        };
        title_with_query(base, self.browser.query())
    }
}

impl EvidenceRow for LearningRow {
    fn id(&self) -> &str {
        &self.id
    }

    fn title(&self) -> &str {
        &self.kind
    }

    fn status(&self) -> &str {
        &self.state
    }

    fn detail(&self) -> &str {
        &self.evidence
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Store;

    impl LearningStore for Store {
        fn load_rows(&self, view: LearningView) -> Vec<LearningRow> {
            match view {
                LearningView::Proposals => vec![row("p-1", "prompt-rule", "pending", "3 events")],
                LearningView::Manifests => vec![row("m-1", "agent-policy", "active", "v2")],
                LearningView::Incidents => {
                    vec![row("i-1", "false-completion", "open", "claim mismatch")]
                }
            }
        }
    }

    #[test]
    fn proposals_screen_loads_proposals() {
        let mut screen = LearningScreen::proposals();
        screen.load_from(&Store);
        assert_eq!(screen.selected().unwrap().id, "p-1");
    }

    #[test]
    fn manifests_screen_loads_manifests() {
        let mut screen = LearningScreen::manifests();
        screen.load_from(&Store);
        assert_eq!(screen.selected().unwrap().state, "active");
    }

    #[test]
    fn incidents_filter_by_evidence() {
        let mut screen = LearningScreen::incidents();
        screen.load_from(&Store);
        screen.set_query("mismatch");
        assert_eq!(screen.len(), 1);
    }

    fn row(id: &str, kind: &str, state: &str, evidence: &str) -> LearningRow {
        LearningRow {
            id: id.into(),
            kind: kind.into(),
            state: state.into(),
            evidence: evidence.into(),
        }
    }
}
