//! The `/permissions` rules overlay (#192).
//!
//! Layer 1 module — no imports from screens/ or app/.
//!
//! Read-only, and deliberately so. The restored screen had a `cycle_selected`
//! that walked a row Allow → Deny → Prompt inside its own `Vec`. There is no
//! runtime setter for these rules anywhere in the tree: they are read once from
//! `[permissions]` at session start into the `RuleSet` the checker evaluates,
//! and nothing can change them without editing the config file and restarting.
//! Cycling a copy would have shown a tool being denied while it carried on
//! being allowed, so the cycle is gone.
//!
//! What is left is still worth having. `/permissions` reports the mode, and the
//! mode is only half the answer — these rules are evaluated first and override
//! it, and until now there was nowhere in the TUI that showed them at all.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::widgets::Row;

use crate::theme::Theme;
use crate::virtual_list::VirtualList;

/// What a rule does when it matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleEffect {
    /// Evaluated first; wins over everything.
    Deny,
    Allow,
    Ask,
}

impl RuleEffect {
    fn label(self) -> &'static str {
        match self {
            Self::Deny => "DENY",
            Self::Allow => "ALLOW",
            Self::Ask => "ASK",
        }
    }
}

/// One configured rule as the overlay shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPermission {
    pub effect: RuleEffect,
    /// The tool the rule applies to, e.g. `Bash`.
    pub tool: String,
    /// The argument pattern, e.g. `git:*`.
    pub pattern: String,
}

/// Read-only browser over the configured permission rules.
#[derive(Debug)]
pub struct PermissionsBrowser {
    list: VirtualList<ToolPermission>,
    /// The permission mode in force, shown alongside the rules because the
    /// rules only make sense against it.
    mode: String,
}

impl PermissionsBrowser {
    pub fn new(mode: String) -> Self {
        Self {
            list: VirtualList::new(Vec::new(), 10),
            mode,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    crate::virtual_list::delegate_virtual_list!(list, ToolPermission);

    pub fn set_permissions(&mut self, rules: Vec<ToolPermission>) {
        self.list.set_items(rules);
    }

    /// Draw the rule list into a centred rect inside `area`.
    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let title = format!(" Permissions — mode: {} · Esc close ", self.mode);

        if self.list.is_empty() {
            // Not a defect and not an empty box: no rules means the mode alone
            // decides, which is worth saying out loud.
            crate::overlay::message(
                f,
                area,
                &title,
                "No rules configured — the mode alone decides. Add them under [permissions] in config.toml.",
                theme,
            );
            return;
        }

        // rows + header + two border lines + the footer line below.
        let (region, block) =
            crate::overlay::open(f, area, self.list.len() as u16 + 4, &title, theme);

        let widths = [
            Constraint::Length(6),
            Constraint::Length(18),
            Constraint::Min(20),
        ];

        let rows: Vec<Row> = self
            .list
            .items()
            .iter()
            .map(|rule| {
                Row::new([
                    rule.effect.label().to_string(),
                    rule.tool.clone(),
                    rule.pattern.clone(),
                ])
                .style(crate::overlay::body_style(theme))
            })
            .collect();

        crate::overlay::render_table(
            f,
            region,
            block,
            Row::new(["Effect", "Tool", "Pattern"]),
            rows,
            &widths,
            self.list.selected_index(),
            theme,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(effect: RuleEffect, tool: &str, pattern: &str) -> ToolPermission {
        ToolPermission {
            effect,
            tool: tool.into(),
            pattern: pattern.into(),
        }
    }

    #[test]
    fn a_new_browser_is_empty() {
        assert!(PermissionsBrowser::new("default".into()).is_empty());
    }

    #[test]
    fn effects_have_the_labels_the_config_uses() {
        assert_eq!(RuleEffect::Deny.label(), "DENY");
        assert_eq!(RuleEffect::Allow.label(), "ALLOW");
        assert_eq!(RuleEffect::Ask.label(), "ASK");
    }

    #[test]
    fn the_cursor_wraps() {
        let mut browser = PermissionsBrowser::new("default".into());
        browser.set_permissions(vec![
            rule(RuleEffect::Deny, "Bash", "rm:*"),
            rule(RuleEffect::Allow, "Read", "*"),
        ]);
        browser.move_down();
        assert_eq!(browser.selected_index(), 1);
        browser.move_down();
        assert_eq!(browser.selected_index(), 0);
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn browser() -> PermissionsBrowser {
        let mut browser = PermissionsBrowser::new("acceptEdits".into());
        browser.set_permissions(vec![
            ToolPermission {
                effect: RuleEffect::Deny,
                tool: "Bash".into(),
                pattern: "rm:*".into(),
            },
            ToolPermission {
                effect: RuleEffect::Allow,
                tool: "Read".into(),
                pattern: "*".into(),
            },
        ]);
        browser
    }

    fn draw(browser: &PermissionsBrowser) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(96, 24)).expect("terminal");
        terminal
            .draw(|frame| browser.render(frame, frame.area(), &crate::theme::dark_theme()))
            .expect("draw permissions browser");
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
    fn rules_and_the_mode_are_drawn() {
        let rendered = text(&draw(&browser()));
        assert!(rendered.contains("DENY"), "{rendered}");
        assert!(rendered.contains("rm:*"), "{rendered}");
        assert!(
            rendered.contains("acceptEdits"),
            "the mode the rules qualify is missing: {rendered}"
        );
        assert!(rendered.contains("Pattern"), "header missing: {rendered}");
    }

    #[test]
    fn the_selected_row_is_highlighted_and_moves_with_the_keys() {
        let mut browser = browser();
        let first = draw(&browser);
        let deny = style_of(&first, "rm:*").expect("first row");
        let allow = style_of(&first, "Read").expect("second row");
        assert_ne!(deny, allow, "selection is invisible");

        browser.move_down();
        assert_ne!(
            deny,
            style_of(&draw(&browser), "rm:*").expect("still drawn"),
            "moving the selection changed nothing on screen"
        );
    }

    /// No rules is a meaningful answer, not a blank box.
    #[test]
    fn no_rules_says_the_mode_alone_decides() {
        let rendered = text(&draw(&PermissionsBrowser::new("default".into())));
        assert!(rendered.contains("mode alone decides"), "{rendered}");
        assert!(
            rendered.contains("[permissions]"),
            "no remedy offered: {rendered}"
        );
    }

    #[test]
    fn the_overlay_does_not_cover_the_whole_frame() {
        let terminal = draw(&browser());
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        let bottom: String = (0..area.width)
            .map(|x| buffer[(x, area.height - 1)].symbol().to_string())
            .collect();
        assert!(bottom.trim().is_empty(), "painted the status-bar row");
    }
}
