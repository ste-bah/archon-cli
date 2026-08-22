//! Model picker screen.
//! Layer 1 module — no imports from screens/ or app/.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::ListItem;

use crate::theme::Theme;
use crate::virtual_list::VirtualList;

/// Provider identifier (placeholder).
pub type ProviderId = String;

/// Model identifier (placeholder).
pub type ModelId = String;

/// A single (provider, model) entry in the picker.
#[derive(Debug, Clone)]
pub struct ProviderEntry {
    pub provider_id: ProviderId,
    pub model_id: ModelId,
    pub label: String,
}

/// Model picker state with virtualized scrolling and fuzzy filter.
#[derive(Debug)]
pub struct ModelPicker {
    providers: Vec<ProviderEntry>,
    list: VirtualList<ProviderEntry>,
    query: String,
}

impl ModelPicker {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
            list: VirtualList::new(Vec::new(), 10),
            query: String::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    crate::virtual_list::delegate_virtual_list!(list, ProviderEntry);

    pub fn query(&self) -> &str {
        &self.query
    }

    /// Set the query string and filter the list.
    pub fn set_query(&mut self, q: &str) {
        self.query = q.to_string();
        self.rebuild_filtered();
    }

    /// Set the full provider list and reset filter.
    pub fn set_providers(&mut self, providers: Vec<ProviderEntry>) {
        self.providers = providers;
        self.query.clear();
        self.rebuild_filtered();
    }

    fn rebuild_filtered(&mut self) {
        let filtered: Vec<ProviderEntry> = if self.query.is_empty() {
            self.providers.clone()
        } else {
            let q = self.query.to_lowercase();
            self.providers
                .iter()
                .filter(|p| p.label.to_lowercase().contains(&q))
                .cloned()
                .collect()
        };
        self.list.set_items(filtered);
    }

    /// Draw the picker into a centred rect inside `area`.
    ///
    /// Renders through [`crate::overlay`] so it is opaque, themed, and shows
    /// its selection — none of which it did before #192, which is why it
    /// looked like a list you could not move around in.
    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let title = if self.query.is_empty() {
            " Models — type to filter · Up/Down select · Enter choose · Esc close ".to_string()
        } else {
            format!(
                " Models — filter: {} · Enter choose · Esc close ",
                self.query
            )
        };

        if self.list.is_empty() {
            let body = if self.providers.is_empty() {
                "No models available."
            } else {
                "No model matches that filter."
            };
            crate::overlay::message(f, area, &title, body, theme);
            return;
        }

        let (region, block) =
            crate::overlay::open(f, area, self.list.len() as u16 + 2, &title, theme);

        let items: Vec<ListItem> = self
            .list
            .items()
            .iter()
            .map(|p| {
                // The label is what the user recognises; the ids are what the
                // config takes. Showing both means the picker can be used to
                // find a name and to confirm one.
                let line = if p.label.is_empty() {
                    format!("{}/{}", p.provider_id, p.model_id)
                } else {
                    format!("{}  ({}/{})", p.label, p.provider_id, p.model_id)
                };
                ListItem::new(line).style(crate::overlay::body_style(theme))
            })
            .collect();

        crate::overlay::render_list(f, region, block, items, self.list.selected_index(), theme);
    }
}

impl Default for ModelPicker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod render_tests {
    //! Assertions about the drawn frame.
    //!
    //! This screen had the same defects as `task_overlay` — no `Clear`, no
    //! selection rendering, `_theme` ignored — and the same reason nobody
    //! noticed: every test below asserts on the model. These look at a buffer.

    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn picker() -> ModelPicker {
        let mut picker = ModelPicker::new();
        picker.set_providers(vec![
            ProviderEntry {
                provider_id: "anthropic".into(),
                model_id: "claude-opus-5".into(),
                label: "opus".into(),
            },
            ProviderEntry {
                provider_id: "anthropic".into(),
                model_id: "claude-sonnet-5".into(),
                label: "sonnet".into(),
            },
        ]);
        picker
    }

    fn draw(picker: &ModelPicker) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(96, 24)).expect("terminal");
        terminal
            .draw(|frame| picker.render(frame, frame.area(), &crate::theme::dark_theme()))
            .expect("draw model picker");
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
    fn labels_and_model_ids_are_both_drawn() {
        let rendered = text(&draw(&picker()));
        assert!(rendered.contains("opus"), "{rendered}");
        assert!(rendered.contains("claude-opus-5"), "{rendered}");
        assert!(rendered.contains("Esc close"), "keys not shown: {rendered}");
    }

    #[test]
    fn the_selected_row_is_highlighted_and_moves_with_the_keys() {
        let mut picker = picker();
        let first_draw = draw(&picker);
        let opus = style_of(&first_draw, "opus").expect("first row drawn");
        let sonnet = style_of(&first_draw, "sonnet").expect("second row drawn");
        assert_ne!(
            opus, sonnet,
            "selected and unselected rows render identically"
        );

        picker.move_down();
        let after = draw(&picker);
        assert_ne!(
            opus,
            style_of(&after, "opus").expect("still drawn"),
            "moving the selection changed nothing on screen"
        );
    }

    /// Colour alone was not perceptible on a real terminal, so the arrow keys
    /// looked dead even though the selection was moving. The marker is the
    /// part that cannot fail to render, so it is the part worth asserting.
    #[test]
    fn the_selection_marker_is_drawn_and_moves_with_the_keys() {
        let mut picker = picker();

        let line_with_marker = |terminal: &Terminal<TestBackend>| -> String {
            let buffer = terminal.backend().buffer();
            let area = buffer.area;
            for y in 0..area.height {
                let line: String = (0..area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect();
                if line.contains(crate::overlay::HIGHLIGHT_SYMBOL.trim()) {
                    return line;
                }
            }
            String::new()
        };

        let first = line_with_marker(&draw(&picker));
        assert!(
            first.contains("opus"),
            "the marker is not on the first row: {first:?}"
        );

        picker.move_down();
        let second = line_with_marker(&draw(&picker));
        assert!(
            second.contains("sonnet"),
            "the marker did not move to the second row: {second:?}"
        );
    }

    #[test]
    fn a_filter_that_matches_nothing_says_so() {
        let mut picker = picker();
        picker.set_query("zzzz");
        assert!(text(&draw(&picker)).contains("No model matches that filter."));
    }

    #[test]
    fn no_models_at_all_is_distinguished_from_a_bad_filter() {
        assert!(text(&draw(&ModelPicker::new())).contains("No models available."));
    }

    #[test]
    fn the_picker_does_not_cover_the_whole_frame() {
        let terminal = draw(&picker());
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        let bottom: String = (0..area.width)
            .map(|x| buffer[(x, area.height - 1)].symbol().to_string())
            .collect();
        assert!(
            bottom.trim().is_empty(),
            "picker painted the last row, where the status bar lives: {bottom:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(provider: &str, model: &str) -> ProviderEntry {
        ProviderEntry {
            provider_id: provider.to_string(),
            model_id: model.to_string(),
            label: format!("{}/{}", provider, model),
        }
    }

    #[test]
    fn new_picker_empty() {
        let picker = ModelPicker::new();
        assert!(picker.is_empty());
        assert_eq!(picker.query(), "");
    }

    #[test]
    fn set_providers_updates_list() {
        let mut picker = ModelPicker::new();
        picker.set_providers(vec![
            entry("anthropic", "claude-opus"),
            entry("openai", "gpt-5"),
        ]);
        assert_eq!(picker.len(), 2);
    }

    #[test]
    fn set_query_filters_list() {
        let mut picker = ModelPicker::new();
        picker.set_providers(vec![
            entry("anthropic", "claude-opus"),
            entry("anthropic", "claude-sonnet"),
            entry("openai", "gpt-5"),
        ]);
        picker.set_query("sonnet");
        assert_eq!(picker.len(), 1); // only claude-sonnet matches
        picker.set_query("claude");
        assert_eq!(picker.len(), 2); // both anthropic entries match
    }

    #[test]
    fn cursor_wraps() {
        let mut picker = ModelPicker::new();
        picker.set_providers(vec![entry("a", "b"), entry("c", "d")]);
        picker.move_down();
        assert_eq!(picker.selected_index(), 1);
        picker.move_down();
        assert_eq!(picker.selected_index(), 0); // wrap
    }

    #[test]
    fn empty_query_shows_all() {
        let mut picker = ModelPicker::new();
        picker.set_providers(vec![entry("x", "y")]);
        picker.set_query("");
        assert_eq!(picker.len(), 1);
    }
}
