//! Document intelligence browser screens.
//! Layer 1 module - no imports from sibling screens or app.

use ratatui::Frame;
use ratatui::layout::Rect;

use super::evidence_browser::{EvidenceBrowser, EvidenceRow, title_with_query};
use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocsView {
    Documents,
    Evidence,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DocsRow {
    pub id: String,
    pub title: String,
    pub status: String,
    pub summary: String,
}

pub trait DocsStore: Send + Sync {
    fn load_rows(&self, view: DocsView) -> Vec<DocsRow>;
}

#[derive(Debug)]
pub struct DocsScreen {
    view: DocsView,
    browser: EvidenceBrowser<DocsRow>,
}

impl DocsScreen {
    pub fn documents() -> Self {
        Self::new(DocsView::Documents)
    }

    pub fn evidence() -> Self {
        Self::new(DocsView::Evidence)
    }

    pub fn new(view: DocsView) -> Self {
        Self {
            view,
            browser: EvidenceBrowser::new(12),
        }
    }

    pub fn load_from<S: DocsStore>(&mut self, store: &S) {
        self.set_rows(store.load_rows(self.view));
    }

    pub fn set_rows(&mut self, rows: Vec<DocsRow>) {
        self.browser.set_rows(rows);
    }

    pub fn set_query(&mut self, query: &str) {
        self.browser.set_query(query);
    }

    pub fn len(&self) -> usize {
        self.browser.len()
    }

    pub fn selected(&self) -> Option<&DocsRow> {
        self.browser.selected()
    }

    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        self.browser.render(f, area, theme, self.title());
    }

    fn title(&self) -> String {
        let base = match self.view {
            DocsView::Documents => "Documents",
            DocsView::Evidence => "Evidence",
        };
        title_with_query(base, self.browser.query())
    }
}

impl EvidenceRow for DocsRow {
    fn id(&self) -> &str {
        &self.id
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn status(&self) -> &str {
        &self.status
    }

    fn detail(&self) -> &str {
        &self.summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Store;

    impl DocsStore for Store {
        fn load_rows(&self, view: DocsView) -> Vec<DocsRow> {
            match view {
                DocsView::Documents => vec![row("doc-1", "Policy Pack", "indexed", "12 chunks")],
                DocsView::Evidence => {
                    vec![row(
                        "claim-1",
                        "Claim evidence",
                        "verified",
                        "citation chain",
                    )]
                }
            }
        }
    }

    #[test]
    fn documents_screen_loads_documents() {
        let mut screen = DocsScreen::documents();
        screen.load_from(&Store);
        assert_eq!(screen.selected().unwrap().id, "doc-1");
    }

    #[test]
    fn evidence_screen_loads_evidence_rows() {
        let mut screen = DocsScreen::evidence();
        screen.load_from(&Store);
        assert_eq!(screen.selected().unwrap().status, "verified");
    }

    #[test]
    fn query_filters_across_summary() {
        let mut screen = DocsScreen::documents();
        screen.load_from(&Store);
        screen.set_query("12 chunks");
        assert_eq!(screen.len(), 1);
    }

    fn row(id: &str, title: &str, status: &str, summary: &str) -> DocsRow {
        DocsRow {
            id: id.into(),
            title: title.into(),
            status: status.into(),
            summary: summary.into(),
        }
    }
}
