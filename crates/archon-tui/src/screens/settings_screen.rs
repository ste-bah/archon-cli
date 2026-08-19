//! The `/config` settings overlay (#192).
//!
//! Layer 1 module — no imports from screens/ or app/.
//!
//! It carried a `SettingsStore` trait nobody implemented and a
//! `toggle_selected` that flipped a bool in a local `Vec`. Nothing downstream
//! read either, so the screen could show a setting turning on while the
//! process it belonged to never heard about it. Both are gone: Enter injects
//! `/config <key> <value>`, which is the one path that validates a value and
//! puts it into the running configuration.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Row, Table, TableState};

use crate::theme::Theme;
use crate::virtual_list::VirtualList;

/// One configuration key as the overlay shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingField {
    pub key: String,
    pub value: String,
    /// A boolean key, so Enter can offer the opposite value.
    pub is_bool: bool,
    /// `/config` will refuse to set this, so the overlay says so up front.
    pub read_only: bool,
}

impl SettingField {
    /// The command Enter should put in the prompt for this row.
    ///
    /// A boolean is offered flipped, because there is only one other value it
    /// could take and making the user type it is ceremony. Everything else
    /// arrives with the current value, ready to be edited — the argument is
    /// still validated by `/config`, so a bad edit is refused there rather
    /// than here.
    pub fn command(&self) -> String {
        if self.is_bool {
            let flipped = !matches!(self.value.as_str(), "true");
            format!("/config {} {flipped}", self.key)
        } else {
            format!("/config {} {}", self.key, self.value)
        }
    }
}

/// Settings overlay over the runtime configuration keys.
#[derive(Debug)]
pub struct SettingsScreen {
    list: VirtualList<SettingField>,
}

impl SettingsScreen {
    pub fn new() -> Self {
        Self {
            list: VirtualList::new(Vec::new(), 10),
        }
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

    pub fn selected(&self) -> Option<&SettingField> {
        self.list.selected()
    }

    pub fn set_fields(&mut self, fields: Vec<SettingField>) {
        self.list.set_items(fields);
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

    /// Draw the settings list into a centred rect inside `area`.
    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        const TITLE: &str = " Settings — Up/Down select · Enter edit · Esc close ";

        if self.list.is_empty() {
            crate::overlay::message(f, area, TITLE, "No configuration keys.", theme);
            return;
        }

        // rows + header + two border lines.
        let (region, block) =
            crate::overlay::open(f, area, self.list.len() as u16 + 3, TITLE, theme);

        let widths = [
            ratatui::layout::Constraint::Percentage(50),
            ratatui::layout::Constraint::Percentage(30),
            ratatui::layout::Constraint::Percentage(20),
        ];

        let rows: Vec<Row> = self
            .list
            .items()
            .iter()
            .map(|field| {
                // Read-only is marked in the list rather than discovered by
                // pressing Enter and being told no.
                let note = if field.read_only {
                    "read-only"
                } else if field.is_bool {
                    "on/off"
                } else {
                    ""
                };
                Row::new([field.key.clone(), field.value.clone(), note.to_string()])
                    .style(crate::overlay::body_style(theme))
            })
            .collect();

        let table = Table::new(rows, &widths)
            .header(Row::new(["Key", "Value", ""]).style(crate::overlay::header_style(theme)))
            .block(block)
            .highlight_symbol(crate::overlay::HIGHLIGHT_SYMBOL)
            .row_highlight_style(crate::overlay::selection_style(theme));

        let mut state = TableState::default().with_selected(Some(self.list.selected_index()));
        f.render_stateful_widget(table, region, &mut state);
    }
}

impl Default for SettingsScreen {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toggle(key: &str, value: &str) -> SettingField {
        SettingField {
            key: key.into(),
            value: value.into(),
            is_bool: true,
            read_only: false,
        }
    }

    fn text(key: &str, value: &str) -> SettingField {
        SettingField {
            key: key.into(),
            value: value.into(),
            is_bool: false,
            read_only: false,
        }
    }

    #[test]
    fn a_new_screen_is_empty() {
        assert!(SettingsScreen::new().is_empty());
    }

    #[test]
    fn set_fields_updates_the_list() {
        let mut screen = SettingsScreen::new();
        screen.set_fields(vec![toggle("tools.cargo.incremental", "false")]);
        assert_eq!(screen.len(), 1);
    }

    /// The point of the rewrite: a boolean is changed by running `/config`,
    /// not by flipping a copy the configuration never sees.
    #[test]
    fn a_boolean_offers_the_opposite_value() {
        assert_eq!(
            toggle("tools.cargo.incremental", "false").command(),
            "/config tools.cargo.incremental true"
        );
        assert_eq!(
            toggle("tools.cargo.incremental", "true").command(),
            "/config tools.cargo.incremental false"
        );
    }

    /// Anything unparseable is not silently treated as `true`.
    #[test]
    fn a_boolean_with_a_junk_value_offers_true() {
        assert_eq!(
            toggle("tools.cargo.incremental", "").command(),
            "/config tools.cargo.incremental true"
        );
    }

    #[test]
    fn a_value_key_arrives_ready_to_edit() {
        assert_eq!(
            text("api.default_effort", "high").command(),
            "/config api.default_effort high"
        );
    }

    #[test]
    fn the_cursor_wraps() {
        let mut screen = SettingsScreen::new();
        screen.set_fields(vec![text("a", "1"), text("b", "2")]);
        screen.move_down();
        assert_eq!(screen.selected_index(), 1);
        screen.move_down();
        assert_eq!(screen.selected_index(), 0);
    }
}

#[cfg(test)]
mod render_tests {
    //! Buffer assertions. Every model test above passed while this screen
    //! rendered with no `Clear`, no selection and the theme underscored.

    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn screen() -> SettingsScreen {
        let mut screen = SettingsScreen::new();
        screen.set_fields(vec![
            SettingField {
                key: "api.default_effort".into(),
                value: "high".into(),
                is_bool: false,
                read_only: false,
            },
            SettingField {
                key: "tools.cargo.incremental".into(),
                value: "false".into(),
                is_bool: true,
                read_only: false,
            },
        ]);
        screen
    }

    fn draw(screen: &SettingsScreen) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(96, 24)).expect("terminal");
        terminal
            .draw(|frame| screen.render(frame, frame.area(), &crate::theme::dark_theme()))
            .expect("draw settings screen");
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
    fn keys_values_and_the_header_are_drawn() {
        let rendered = text(&draw(&screen()));
        assert!(rendered.contains("api.default_effort"), "{rendered}");
        assert!(rendered.contains("high"), "{rendered}");
        assert!(rendered.contains("Key"), "header missing: {rendered}");
        assert!(rendered.contains("Esc close"), "keys not shown: {rendered}");
    }

    #[test]
    fn the_selected_row_is_highlighted_and_moves_with_the_keys() {
        let mut screen = screen();
        let first = draw(&screen);
        let effort = style_of(&first, "api.default_effort").expect("first row");
        let cargo = style_of(&first, "tools.cargo.incremental").expect("second row");
        assert_ne!(effort, cargo, "selection is invisible");

        screen.move_down();
        assert_ne!(
            effort,
            style_of(&draw(&screen), "api.default_effort").expect("still drawn"),
            "moving the selection changed nothing on screen"
        );
    }

    /// A key that cannot be set has to say so in the list, not after Enter.
    #[test]
    fn a_read_only_key_is_marked() {
        let mut screen = SettingsScreen::new();
        screen.set_fields(vec![SettingField {
            key: "build.profile".into(),
            value: "dev".into(),
            is_bool: false,
            read_only: true,
        }]);
        assert!(text(&draw(&screen)).contains("read-only"));
    }

    #[test]
    fn an_empty_configuration_is_stated_in_words() {
        assert!(text(&draw(&SettingsScreen::new())).contains("No configuration keys."));
    }

    #[test]
    fn the_overlay_does_not_cover_the_whole_frame() {
        let terminal = draw(&screen());
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        let bottom: String = (0..area.width)
            .map(|x| buffer[(x, area.height - 1)].symbol().to_string())
            .collect();
        assert!(bottom.trim().is_empty(), "painted the status-bar row");
    }
}
