//! Game-theory evidence browser screens.
//! Layer 1 module - no imports from sibling screens or app.

use ratatui::Frame;
use ratatui::layout::Rect;

use super::evidence_browser::{EvidenceBrowser, EvidenceRow, title_with_query};
use crate::theme::Theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameTheoryView {
    Main,
    RunDetail { run_id: String },
    Specimens,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GameTheoryRow {
    pub id: String,
    pub label: String,
    pub status: String,
    pub detail: String,
}

pub trait GameTheoryStore: Send + Sync {
    fn load_rows(&self, view: &GameTheoryView) -> Vec<GameTheoryRow>;
}

#[derive(Debug)]
pub struct GameTheoryScreen {
    view: GameTheoryView,
    browser: EvidenceBrowser<GameTheoryRow>,
}

impl GameTheoryScreen {
    pub fn new(view: GameTheoryView) -> Self {
        Self {
            view,
            browser: EvidenceBrowser::new(12),
        }
    }

    pub fn main() -> Self {
        Self::new(GameTheoryView::Main)
    }

    pub fn run_detail(run_id: impl Into<String>) -> Self {
        Self::new(GameTheoryView::RunDetail {
            run_id: run_id.into(),
        })
    }

    pub fn specimens() -> Self {
        Self::new(GameTheoryView::Specimens)
    }

    pub fn view(&self) -> &GameTheoryView {
        &self.view
    }

    pub fn load_from<S: GameTheoryStore>(&mut self, store: &S) {
        self.set_rows(store.load_rows(&self.view));
    }

    pub fn set_rows(&mut self, rows: Vec<GameTheoryRow>) {
        self.browser.set_rows(rows);
    }

    pub fn set_query(&mut self, query: &str) {
        self.browser.set_query(query);
    }

    pub fn len(&self) -> usize {
        self.browser.len()
    }

    pub fn selected(&self) -> Option<&GameTheoryRow> {
        self.browser.selected()
    }

    pub fn move_down(&mut self) {
        self.browser.move_down();
    }

    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        self.browser.render(f, area, theme, self.title());
    }

    fn title(&self) -> String {
        let base = match &self.view {
            GameTheoryView::Main => "Game-Theory Runs".to_string(),
            GameTheoryView::RunDetail { run_id } => format!("Game-Theory Run {run_id}"),
            GameTheoryView::Specimens => "Game-Theory Specimens".to_string(),
        };
        title_with_query(base, self.browser.query())
    }
}

impl Default for GameTheoryScreen {
    fn default() -> Self {
        Self::main()
    }
}

impl EvidenceRow for GameTheoryRow {
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

    impl GameTheoryStore for Store {
        fn load_rows(&self, view: &GameTheoryView) -> Vec<GameTheoryRow> {
            match view {
                GameTheoryView::Main => vec![row("gt-1", "Run one", "completed", "cost $0.01")],
                GameTheoryView::RunDetail { run_id } => {
                    vec![row(run_id, "nash-equilibrium-finder", "completed", "ok")]
                }
                GameTheoryView::Specimens => vec![row(
                    "specimen-prisoners-dilemma",
                    "Prisoner's dilemma",
                    "loaded",
                    "non-cooperative",
                )],
            }
        }
    }

    #[test]
    fn main_screen_loads_runs() {
        let mut screen = GameTheoryScreen::main();
        screen.load_from(&Store);
        assert_eq!(screen.len(), 1);
        assert_eq!(screen.selected().unwrap().id, "gt-1");
    }

    #[test]
    fn run_detail_uses_run_id() {
        let mut screen = GameTheoryScreen::run_detail("gt-42");
        screen.load_from(&Store);
        assert_eq!(screen.selected().unwrap().id, "gt-42");
    }

    #[test]
    fn specimens_filter_by_detail() {
        let mut screen = GameTheoryScreen::specimens();
        screen.load_from(&Store);
        screen.set_query("non-cooperative");
        assert_eq!(screen.len(), 1);
    }

    fn row(id: &str, label: &str, status: &str, detail: &str) -> GameTheoryRow {
        GameTheoryRow {
            id: id.into(),
            label: label.into(),
            status: status.into(),
            detail: detail.into(),
        }
    }
}
