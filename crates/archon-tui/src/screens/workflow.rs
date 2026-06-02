//! Dynamic workflow inspection screen.

use ratatui::Frame;
use ratatui::layout::Rect;

use super::evidence_browser::{EvidenceBrowser, EvidenceRow, title_with_query};
use crate::theme::Theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowView {
    Runs,
    RunDetail { run_id: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowRow {
    pub id: String,
    pub label: String,
    pub status: String,
    pub detail: String,
}

pub trait WorkflowStore: Send + Sync {
    fn load_rows(&self, view: &WorkflowView) -> Vec<WorkflowRow>;
}

#[derive(Debug)]
pub struct WorkflowScreen {
    view: WorkflowView,
    browser: EvidenceBrowser<WorkflowRow>,
}

impl WorkflowScreen {
    pub fn runs() -> Self {
        Self::new(WorkflowView::Runs)
    }

    pub fn run_detail(run_id: impl Into<String>) -> Self {
        Self::new(WorkflowView::RunDetail {
            run_id: run_id.into(),
        })
    }

    pub fn new(view: WorkflowView) -> Self {
        Self {
            view,
            browser: EvidenceBrowser::new(12),
        }
    }

    pub fn load_from<S: WorkflowStore>(&mut self, store: &S) {
        self.set_rows(store.load_rows(&self.view));
    }

    pub fn set_rows(&mut self, rows: Vec<WorkflowRow>) {
        self.browser.set_rows(rows);
    }

    pub fn set_query(&mut self, query: &str) {
        self.browser.set_query(query);
    }

    pub fn len(&self) -> usize {
        self.browser.len()
    }

    pub fn selected(&self) -> Option<&WorkflowRow> {
        self.browser.selected()
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        self.browser.render(frame, area, theme, self.title());
    }

    fn title(&self) -> String {
        let base = match &self.view {
            WorkflowView::Runs => "Dynamic Workflows".to_string(),
            WorkflowView::RunDetail { run_id } => format!("Workflow {run_id}"),
        };
        title_with_query(&base, self.browser.query())
    }
}

impl Default for WorkflowScreen {
    fn default() -> Self {
        Self::runs()
    }
}

impl EvidenceRow for WorkflowRow {
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

    struct Store;

    impl WorkflowStore for Store {
        fn load_rows(&self, view: &WorkflowView) -> Vec<WorkflowRow> {
            match view {
                WorkflowView::Runs => vec![row("wf-1", "Repo audit", "completed", "4/4 stages")],
                WorkflowView::RunDetail { run_id } => {
                    vec![row(run_id, "review", "failed", "failure remains visible")]
                }
            }
        }
    }

    #[test]
    fn runs_load_source_rows() {
        let mut screen = WorkflowScreen::runs();
        screen.load_from(&Store);
        assert_eq!(screen.selected().unwrap().id, "wf-1");
    }

    #[test]
    fn failed_stage_stays_visible_in_detail() {
        let mut screen = WorkflowScreen::run_detail("wf-2");
        screen.load_from(&Store);
        assert_eq!(screen.selected().unwrap().status, "failed");
    }

    fn row(id: &str, label: &str, status: &str, detail: &str) -> WorkflowRow {
        WorkflowRow {
            id: id.into(),
            label: label.into(),
            status: status.into(),
            detail: detail.into(),
        }
    }
}
