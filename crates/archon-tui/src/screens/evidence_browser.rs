//! Shared evidence-engine list browser.
//! Layer 1 module - no imports from sibling screens or app.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, List, ListItem};

use crate::theme::Theme;
use crate::virtual_list::VirtualList;

pub trait EvidenceRow: Clone {
    fn id(&self) -> &str;
    fn title(&self) -> &str;
    fn status(&self) -> &str;
    fn detail(&self) -> &str;
}

#[derive(Debug)]
pub struct EvidenceBrowser<T: EvidenceRow> {
    rows: Vec<T>,
    list: VirtualList<T>,
    query: String,
}

impl<T: EvidenceRow> EvidenceBrowser<T> {
    pub fn new(viewport_height: usize) -> Self {
        Self {
            rows: Vec::new(),
            list: VirtualList::new(Vec::new(), viewport_height),
            query: String::new(),
        }
    }

    pub fn set_rows(&mut self, rows: Vec<T>) {
        self.rows = rows;
        self.query.clear();
        self.rebuild_filtered();
    }

    pub fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
        self.rebuild_filtered();
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn selected(&self) -> Option<&T> {
        self.list.selected()
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn move_down(&mut self) {
        self.list.move_down();
    }

    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme, title: String) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .title_style(theme.header)
            .border_style(Style::default().fg(theme.border));
        let items: Vec<ListItem> = self
            .list
            .visible_items()
            .iter()
            .map(|row| {
                ListItem::new(format!(
                    "{}  [{}]  {}",
                    row.title(),
                    row.status(),
                    row.detail()
                ))
                .style(Style::default().fg(theme.fg))
            })
            .collect();
        f.render_widget(List::new(items).block(block), area);
    }

    fn rebuild_filtered(&mut self) {
        let filtered = filter_rows(&self.rows, &self.query);
        self.list.set_items(filtered);
    }
}

pub fn title_with_query(base: impl Into<String>, query: &str) -> String {
    let base = base.into();
    if query.is_empty() {
        base
    } else {
        format!("{base} — {query}")
    }
}

fn filter_rows<T: EvidenceRow>(rows: &[T], query: &str) -> Vec<T> {
    if query.is_empty() {
        return rows.to_vec();
    }
    let q = query.to_lowercase();
    rows.iter()
        .filter(|row| {
            row.id().to_lowercase().contains(&q)
                || row.title().to_lowercase().contains(&q)
                || row.status().to_lowercase().contains(&q)
                || row.detail().to_lowercase().contains(&q)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestRow {
        id: String,
        title: String,
        status: String,
        detail: String,
    }

    impl EvidenceRow for TestRow {
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
            &self.detail
        }
    }

    #[test]
    fn set_rows_selects_first_row() {
        let mut browser = EvidenceBrowser::new(3);
        browser.set_rows(vec![row("r1", "Run", "ok", "detail")]);
        assert_eq!(browser.len(), 1);
        assert_eq!(browser.selected().unwrap().id, "r1");
    }

    #[test]
    fn query_filters_across_all_row_fields() {
        let mut browser = EvidenceBrowser::new(3);
        browser.set_rows(vec![
            row("r1", "Run", "ok", "ordinary"),
            row("r2", "Evidence", "verified", "citation chain"),
        ]);
        browser.set_query("citation");
        assert_eq!(browser.len(), 1);
        assert_eq!(browser.selected().unwrap().id, "r2");
    }

    #[test]
    fn set_rows_clears_previous_query() {
        let mut browser = EvidenceBrowser::new(3);
        browser.set_rows(vec![row("r1", "Run", "ok", "ordinary")]);
        browser.set_query("missing");
        assert_eq!(browser.len(), 0);
        browser.set_rows(vec![row("r2", "Learning", "open", "proposal")]);
        assert_eq!(browser.query(), "");
        assert_eq!(browser.len(), 1);
    }

    fn row(id: &str, title: &str, status: &str, detail: &str) -> TestRow {
        TestRow {
            id: id.into(),
            title: title.into(),
            status: status.into(),
            detail: detail.into(),
        }
    }
}
