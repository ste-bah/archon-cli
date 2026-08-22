//! The `/permissions presets` selector (#200 Phase 3).
//!
//! Layer 1 module — no imports from screens/ or app/.
//!
//! Setting a coherent permission posture used to mean editing five
//! interdependent fields across two config sections and knowing which of them
//! the chosen backend actually reads. This overlay lists the named
//! combinations with the one-line reason each exists, and marks the one in
//! force — including `custom`, when the config matches none of them.
//!
//! Enter injects `/permissions preset <name>` into the prompt rather than
//! applying anything itself, the same contract the `/model` and `/theme`
//! pickers keep. There is exactly one handler that validates a preset name and
//! persists it; an overlay that wrote the knobs directly would be a second
//! path with its own bugs, and worse, it would be a preset layer that acts
//! instead of one that records intent.
//!
//! Individual knobs stay settable. `/permissions <mode>` still sets a mode on
//! its own, and hand-editing config.toml still works — such a config simply
//! reads back here as `custom`.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{List, ListItem, ListState};

use crate::theme::Theme;
use crate::virtual_list::VirtualList;

/// One selectable preset row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetEntry {
    /// Preset name, as typed at `/permissions preset <name>`.
    pub name: String,
    /// One line saying what the combination buys.
    pub description: String,
    /// `permissions.mode` this preset writes.
    pub permission_mode: String,
    /// `sandbox.backend` this preset writes.
    pub sandbox_backend: String,
}

/// Preset selector overlay.
#[derive(Debug)]
pub struct PermissionPresetPicker {
    list: VirtualList<PresetEntry>,
    /// The preset the current config corresponds to, or `custom`. Derived by
    /// the caller from live values — never stored anywhere.
    active: String,
}

impl PermissionPresetPicker {
    pub fn new(active: String) -> Self {
        Self {
            list: VirtualList::new(Vec::new(), 10),
            active,
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

    pub fn selected(&self) -> Option<&PresetEntry> {
        self.list.selected()
    }

    pub fn active(&self) -> &str {
        &self.active
    }

    /// Load the rows and stand the cursor on the active preset.
    ///
    /// Landing on the first row instead would make "which am I on" the one
    /// question the overlay does not answer at a glance. When the active
    /// posture is `custom` no row matches, and the cursor stays at the top.
    pub fn set_presets(&mut self, presets: Vec<PresetEntry>) {
        let active_index = presets.iter().position(|p| p.name == self.active);
        self.list.set_items(presets);
        if let Some(index) = active_index {
            self.list.select_index(index);
        }
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

    /// Draw the preset list into a centred rect inside `area`.
    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let title = format!(
            " Permission presets — in force: {} · Enter select · Esc close ",
            self.active
        );

        if self.list.is_empty() {
            crate::overlay::message(f, area, &title, "No presets available.", theme);
            return;
        }

        // Two lines per preset: the tuple it writes, then why you would want
        // it. The tuple is the part that is otherwise invisible.
        let (region, block) =
            crate::overlay::open(f, area, self.list.len() as u16 * 2 + 2, &title, theme);

        let items: Vec<ListItem> = self
            .list
            .items()
            .iter()
            .map(|preset| {
                let marker = if preset.name == self.active {
                    "●"
                } else {
                    " "
                };
                ListItem::new(format!(
                    "{marker} {name}  [mode {mode} · sandbox {backend}]\n    {description}",
                    name = preset.name,
                    mode = preset.permission_mode,
                    backend = preset.sandbox_backend,
                    description = preset.description,
                ))
                .style(crate::overlay::body_style(theme))
            })
            .collect();

        let list = List::new(items)
            .block(block)
            .highlight_symbol(crate::overlay::HIGHLIGHT_SYMBOL)
            .highlight_style(crate::overlay::selection_style(theme));

        let mut state = ListState::default().with_selected(Some(self.list.selected_index()));
        f.render_stateful_widget(list, region, &mut state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn entries() -> Vec<PresetEntry> {
        vec![
            PresetEntry {
                name: "read-only".into(),
                description: "Explore and plan. No writes, no execution.".into(),
                permission_mode: "plan".into(),
                sandbox_backend: "disabled".into(),
            },
            PresetEntry {
                name: "sandboxed".into(),
                description: "Auto-approve everything, but only inside a container.".into(),
                permission_mode: "bubble".into(),
                sandbox_backend: "docker".into(),
            },
        ]
    }

    fn buffer_text(picker: &PermissionPresetPicker) -> String {
        let mut terminal = Terminal::new(TestBackend::new(96, 24)).expect("terminal");
        terminal
            .draw(|frame| picker.render(frame, frame.area(), &crate::theme::dark_theme()))
            .expect("draw");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn cursor_lands_on_the_active_preset() {
        let mut picker = PermissionPresetPicker::new("sandboxed".into());
        picker.set_presets(entries());

        assert_eq!(picker.selected_index(), 1);
        assert_eq!(
            picker.selected().map(|p| p.name.as_str()),
            Some("sandboxed")
        );
    }

    #[test]
    fn a_custom_posture_selects_no_row_and_still_opens() {
        let mut picker = PermissionPresetPicker::new("custom".into());
        picker.set_presets(entries());

        assert_eq!(picker.selected_index(), 0);
        assert_eq!(picker.active(), "custom");
        assert!(buffer_text(&picker).contains("custom"));
    }

    #[test]
    fn rows_show_the_tuple_and_the_reason() {
        let mut picker = PermissionPresetPicker::new("read-only".into());
        picker.set_presets(entries());

        let text = buffer_text(&picker);
        assert!(text.contains("read-only"), "{text}");
        assert!(text.contains("mode plan"), "{text}");
        assert!(text.contains("sandbox docker"), "{text}");
        assert!(text.contains("No writes, no execution."), "{text}");
    }

    #[test]
    fn an_empty_table_says_so_rather_than_drawing_a_blank_box() {
        let picker = PermissionPresetPicker::new("custom".into());

        assert!(picker.is_empty());
        assert!(buffer_text(&picker).contains("No presets available."));
    }

    #[test]
    fn movement_wraps_the_way_every_other_overlay_does() {
        // `VirtualList` wraps at both ends; asserting a clamp here would pin a
        // behaviour this overlay does not have and the others do not either.
        let mut picker = PermissionPresetPicker::new("read-only".into());
        picker.set_presets(entries());

        picker.move_up();
        assert_eq!(picker.selected_index(), picker.len() - 1);
        picker.move_down();
        assert_eq!(picker.selected_index(), 0);
        picker.move_down();
        assert_eq!(picker.selected_index(), 1);
        assert!(picker.selected().is_some());
    }
}
