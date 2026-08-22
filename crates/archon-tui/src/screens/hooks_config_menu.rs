//! The `/hooks` overlay (#192).
//!
//! Layer 1 module — no imports from screens/ or app/.
//!
//! It carried a `HookStore` trait with no implementor and a `toggle_selected`
//! that flipped a bool in a local `Vec`. Enabling a hook writes to
//! `.archon/hooks.local.toml` through the registry; flipping a copy of the
//! list did none of that, so the screen would have shown a hook switching off
//! while it carried on firing. Both are gone — Enter injects
//! `/hooks enable <id>` or `/hooks disable <id>`, which is the path that
//! actually persists.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::widgets::Row;

use crate::theme::Theme;
use crate::virtual_list::VirtualList;

/// One registered hook as the overlay shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRow {
    /// Stable id — what `/hooks enable` and `/hooks disable` accept.
    pub id: String,
    /// The event it fires on, e.g. `PreToolUse`.
    pub event: String,
    /// The command it runs.
    ///
    /// This is what tells two hooks on the same event apart. Observed live:
    /// three project hooks all read `PostToolUse` with matcher `*` and
    /// differed only in the tool named at the end of their command, so a list
    /// showing the matcher showed three identical rows.
    pub command: String,
    /// Where it was loaded from: user, project, local, policy.
    pub source: String,
    pub enabled: bool,
}

impl HookRow {
    /// The command Enter should put in the prompt for this row.
    ///
    /// Always the opposite of the current state: there are two, and making
    /// someone type which one they meant after selecting the row is ceremony.
    pub fn command(&self) -> String {
        let verb = if self.enabled { "disable" } else { "enable" };
        format!("/hooks {verb} {}", self.id)
    }
}

/// Hooks overlay over the registered hooks.
#[derive(Debug)]
pub struct HooksMenu {
    list: VirtualList<HookRow>,
}

impl HooksMenu {
    pub fn new() -> Self {
        Self {
            list: VirtualList::new(Vec::new(), 10),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    crate::virtual_list::delegate_virtual_list!(list, HookRow);

    pub fn set_hooks(&mut self, hooks: Vec<HookRow>) {
        self.list.set_items(hooks);
    }

    /// Draw the hooks list into a centred rect inside `area`.
    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        const TITLE: &str = " Hooks — Up/Down select · Enter enable/disable · Esc close ";

        if self.list.is_empty() {
            crate::overlay::message(f, area, TITLE, "No hooks are registered.", theme);
            return;
        }

        // rows + header + two border lines.
        let (region, block) =
            crate::overlay::open(f, area, self.list.len() as u16 + 3, TITLE, theme);

        let widths = [
            Constraint::Length(3),
            Constraint::Length(10),
            Constraint::Length(14),
            Constraint::Min(20),
            Constraint::Length(9),
        ];

        let rows: Vec<Row> = self
            .list
            .items()
            .iter()
            .map(|hook| {
                Row::new([
                    if hook.enabled { "[x]" } else { "[ ]" }.to_string(),
                    hook.id.clone(),
                    hook.event.clone(),
                    hook.command.clone(),
                    hook.source.clone(),
                ])
                .style(crate::overlay::body_style(theme))
            })
            .collect();

        crate::overlay::render_table(
            f,
            region,
            block,
            Row::new(["On", "ID", "Event", "Command", "Source"]),
            rows,
            &widths,
            self.list.selected_index(),
            theme,
        );
    }
}

impl Default for HooksMenu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(id: &str, enabled: bool) -> HookRow {
        HookRow {
            id: id.into(),
            event: "PreToolUse".into(),
            command: "bash scripts/self-check-file.sh Edit".into(),
            source: "project".into(),
            enabled,
        }
    }

    #[test]
    fn a_new_menu_is_empty() {
        assert!(HooksMenu::new().is_empty());
    }

    /// The point of the rewrite: the change goes through `/hooks`, which
    /// writes `.archon/hooks.local.toml`, not through a local bool.
    #[test]
    fn enter_offers_the_opposite_state() {
        assert_eq!(hook("h1", true).command(), "/hooks disable h1");
        assert_eq!(hook("h1", false).command(), "/hooks enable h1");
    }

    #[test]
    fn the_cursor_wraps() {
        let mut menu = HooksMenu::new();
        menu.set_hooks(vec![hook("a", true), hook("b", false)]);
        menu.move_down();
        assert_eq!(menu.selected_index(), 1);
        menu.move_down();
        assert_eq!(menu.selected_index(), 0);
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn menu() -> HooksMenu {
        let mut menu = HooksMenu::new();
        menu.set_hooks(vec![
            HookRow {
                id: "abc123".into(),
                event: "PreToolUse".into(),
                command: "bash scripts/self-check-file.sh Edit".into(),
                source: "project".into(),
                enabled: true,
            },
            HookRow {
                id: "def456".into(),
                event: "PostToolUse".into(),
                command: "bash scripts/self-check-file.sh Write".into(),
                source: "user".into(),
                enabled: false,
            },
        ]);
        menu
    }

    fn draw(menu: &HooksMenu) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(96, 24)).expect("terminal");
        terminal
            .draw(|frame| menu.render(frame, frame.area(), &crate::theme::dark_theme()))
            .expect("draw hooks menu");
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
    fn ids_events_and_the_enabled_marker_are_drawn() {
        let rendered = text(&draw(&menu()));
        assert!(rendered.contains("abc123"), "{rendered}");
        assert!(rendered.contains("PostToolUse"), "{rendered}");
        assert!(
            rendered.contains("[x]"),
            "enabled marker missing: {rendered}"
        );
        assert!(
            rendered.contains("[ ]"),
            "disabled marker missing: {rendered}"
        );
        assert!(rendered.contains("Source"), "header missing: {rendered}");
    }

    /// Caught on a real terminal: this repository's three project hooks are all
    /// `PostToolUse` with matcher `*`, so a list built from the matcher drew
    /// three rows differing only by an opaque id. What a hook runs has to be on
    /// the row.
    #[test]
    fn hooks_that_differ_only_in_what_they_run_are_distinguishable() {
        let mut menu = HooksMenu::new();
        menu.set_hooks(vec![
            HookRow {
                id: "h1".into(),
                event: "PostToolUse".into(),
                command: "self-check Edit".into(),
                source: "project".into(),
                enabled: true,
            },
            HookRow {
                id: "h2".into(),
                event: "PostToolUse".into(),
                command: "self-check Write".into(),
                source: "project".into(),
                enabled: true,
            },
        ]);

        let rendered = text(&draw(&menu));
        assert!(rendered.contains("self-check Edit"), "{rendered}");
        assert!(rendered.contains("self-check Write"), "{rendered}");
    }

    /// Which hook a keypress will act on has to be visible.
    #[test]
    fn the_selected_row_is_highlighted_and_moves_with_the_keys() {
        let mut menu = menu();
        let first = draw(&menu);
        let one = style_of(&first, "abc123").expect("first row");
        let two = style_of(&first, "def456").expect("second row");
        assert_ne!(one, two, "selection is invisible");

        menu.move_down();
        assert_ne!(
            one,
            style_of(&draw(&menu), "abc123").expect("still drawn"),
            "moving the selection changed nothing on screen"
        );
    }

    #[test]
    fn no_hooks_is_stated_in_words() {
        assert!(text(&draw(&HooksMenu::new())).contains("No hooks are registered."));
    }

    #[test]
    fn the_overlay_does_not_cover_the_whole_frame() {
        let terminal = draw(&menu());
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        let bottom: String = (0..area.width)
            .map(|x| buffer[(x, area.height - 1)].symbol().to_string())
            .collect();
        assert!(bottom.trim().is_empty(), "painted the status-bar row");
    }
}
