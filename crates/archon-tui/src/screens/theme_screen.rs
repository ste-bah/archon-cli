//! Theme screen.
//! Layer 1 module — no imports from screens/ or app/.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{List, ListItem, ListState};

use crate::theme::Theme;
use crate::virtual_list::VirtualList;

/// A theme entry for the picker.
#[derive(Debug, Clone)]
pub struct ThemeEntry {
    pub name: String,
    pub is_active: bool,
}

/// Theme screen with virtualized list of themes.
#[derive(Debug)]
pub struct ThemeScreen {
    themes: Vec<ThemeEntry>,
    list: VirtualList<ThemeEntry>,
}

impl ThemeScreen {
    pub fn new() -> Self {
        Self {
            themes: Vec::new(),
            list: VirtualList::new(Vec::new(), 10),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.themes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn selected_index(&self) -> usize {
        self.list.selected_index()
    }

    pub fn selected(&self) -> Option<&ThemeEntry> {
        self.list.selected()
    }

    /// Set themes list.
    pub fn set_themes(&mut self, themes: Vec<ThemeEntry>) {
        self.themes = themes;
        self.list.set_items(self.themes.clone());
    }

    /// Select theme (marks active, unmarks others).
    pub fn select_theme(&mut self, name: &str) {
        for t in &mut self.themes {
            t.is_active = t.name == name;
        }
        self.list.set_items(self.themes.clone());
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

    /// Render theme screen into area.
    /// Draw the theme list into a centred rect inside `area`.
    ///
    /// Renders through [`crate::overlay`], so it is opaque, themed and shows
    /// its selection. The `theme` argument is the one currently applied — this
    /// screen used to underscore and ignore it, which meant the theme picker
    /// was the one overlay that did not respect the theme.
    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        const TITLE: &str = " Theme — Up/Down select · Enter apply · Esc close ";

        if self.list.is_empty() {
            crate::overlay::message(f, area, TITLE, "No themes available.", theme);
            return;
        }

        let (region, block) =
            crate::overlay::open(f, area, self.list.len() as u16 + 2, TITLE, theme);

        let items: Vec<ListItem> = self
            .list
            .items()
            .iter()
            .map(|t| {
                // The active theme is marked, because "which one am I on" is
                // the question you open this to answer.
                let marker = if t.is_active { "●" } else { " " };
                ListItem::new(format!(" {marker} {}", t.name))
                    .style(crate::overlay::body_style(theme))
            })
            .collect();

        let list = List::new(items)
            .block(block)
            .highlight_style(crate::overlay::selection_style(theme));

        let mut state = ListState::default().with_selected(Some(self.list.selected_index()));
        f.render_stateful_widget(list, region, &mut state);
    }
}

impl Default for ThemeScreen {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod render_tests {
    //! Buffer assertions. The model tests below all passed while this screen
    //! rendered without `Clear`, without a selection, and ignoring the theme.

    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn screen() -> ThemeScreen {
        let mut screen = ThemeScreen::new();
        screen.set_themes(vec![
            ThemeEntry {
                name: "intj".into(),
                is_active: true,
            },
            ThemeEntry {
                name: "ocean".into(),
                is_active: false,
            },
        ]);
        screen
    }

    fn draw(screen: &ThemeScreen) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(96, 24)).expect("terminal");
        terminal
            .draw(|frame| screen.render(frame, frame.area(), &crate::theme::dark_theme()))
            .expect("draw theme screen");
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
    fn theme_names_and_the_active_marker_are_drawn() {
        let rendered = text(&draw(&screen()));
        assert!(rendered.contains("intj"), "{rendered}");
        assert!(rendered.contains("ocean"), "{rendered}");
        assert!(
            rendered.contains('●'),
            "the applied theme is unmarked, which is the question this screen answers: {rendered}"
        );
        assert!(rendered.contains("Esc close"), "keys not shown: {rendered}");
    }

    #[test]
    fn the_selected_row_is_highlighted_and_moves_with_the_keys() {
        let mut screen = screen();
        let first = draw(&screen);
        let intj = style_of(&first, "intj").expect("first row");
        let ocean = style_of(&first, "ocean").expect("second row");
        assert_ne!(intj, ocean, "selection is invisible");

        screen.move_down();
        assert_ne!(
            intj,
            style_of(&draw(&screen), "intj").expect("still drawn"),
            "moving the selection changed nothing on screen"
        );
    }

    #[test]
    fn no_themes_is_stated_rather_than_drawn_as_an_empty_box() {
        assert!(text(&draw(&ThemeScreen::new())).contains("No themes available."));
    }

    #[test]
    fn the_picker_does_not_cover_the_whole_frame() {
        let terminal = draw(&screen());
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        let bottom: String = (0..area.width)
            .map(|x| buffer[(x, area.height - 1)].symbol().to_string())
            .collect();
        assert!(bottom.trim().is_empty(), "painted the status-bar row");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_screen_empty() {
        let screen = ThemeScreen::new();
        assert!(screen.is_empty());
    }

    #[test]
    fn set_themes_updates_list() {
        let mut screen = ThemeScreen::new();
        screen.set_themes(vec![
            ThemeEntry {
                name: "dark".into(),
                is_active: true,
            },
            ThemeEntry {
                name: "light".into(),
                is_active: false,
            },
        ]);
        assert_eq!(screen.len(), 2);
    }

    #[test]
    fn select_theme_marks_active() {
        let mut screen = ThemeScreen::new();
        screen.set_themes(vec![
            ThemeEntry {
                name: "dark".into(),
                is_active: true,
            },
            ThemeEntry {
                name: "light".into(),
                is_active: false,
            },
            ThemeEntry {
                name: "nord".into(),
                is_active: false,
            },
        ]);
        screen.select_theme("light");
        let light = screen.themes.iter().find(|t| t.name == "light").unwrap();
        assert!(light.is_active);
        let dark = screen.themes.iter().find(|t| t.name == "dark").unwrap();
        assert!(!dark.is_active);
    }

    #[test]
    fn cursor_wraps() {
        let mut screen = ThemeScreen::new();
        screen.set_themes(vec![
            ThemeEntry {
                name: "a".into(),
                is_active: false,
            },
            ThemeEntry {
                name: "b".into(),
                is_active: false,
            },
        ]);
        screen.move_down();
        assert_eq!(screen.selected_index(), 1);
        screen.move_down();
        assert_eq!(screen.selected_index(), 0); // wrap
    }
}
