//! TASK-TUI-627 /skills slash-command handler.
//!
//! `/skills` opens a scrollable overlay listing every registered skill
//! (from `archon_core::skills::SkillRegistry`). User picks one and the
//! selected skill's name is injected into the prompt buffer.
//!
//! # Architecture (overlay command)
//!
//! Mirrors TUI-620 `/rewind` exactly:
//!
//!   - New `SkillEntry` DTO in `archon-tui::events` (+ re-export via app).
//!   - New `TuiEvent::ShowSkillsMenu(Vec<SkillEntry>)` variant (dual,
//!     events.rs + app.rs, mirroring `ShowMessageSelector` precedent).
//!   - New `SkillsMenu` screen at `crates/archon-tui/src/screens/skills_menu.rs`
//!     with `selected_index` + `select_next`/`select_prev` nav methods
//!     + 5 tests.
//!   - `App::skills_menu: Option<SkillsMenu>` field.
//!   - Event-loop arm sets `app.skills_menu = Some(SkillsMenu::new(...))`.
//!   - `SkillLister` trait seam — `RealSkillLister` builds a fresh
//!     `SkillRegistry` via `register_builtins()` each call; `MockSkillLister`
//!     for tests (`#[cfg(test)]`).
//!
//! # Reconciliation with TASK-TUI-627.md spec
//!
//! Spec references `crates/archon-tui/src/slash/skills.rs` +
//! `SlashCommand` + `SlashOutcome::OpenOverlay(Box::new(SkillsMenuOverlay))`.
//! Actual: bin-crate `src/command/skills.rs` + `CommandHandler` +
//! dedicated `TuiEvent::ShowSkillsMenu` variant (same adaptation as
//! TUI-620 /rewind).
//!
//! Spec asserts "Enter on a skill injects `/skill-name` into the prompt
//! buffer." That interaction lives in `event_loop/input.rs` routing —
//! deferred to TUI-627-followup (same scope reduction as TUI-620).

use archon_tui::app::{SkillEntry, TuiEvent};

use crate::command::registry::{CommandContext, CommandHandler};

/// Seam — tests inject `MockSkillLister`, production uses `RealSkillLister`.
pub(crate) trait SkillLister: Send + Sync {
    fn list(&self) -> Vec<SkillEntry>;
}

pub(crate) struct RealSkillLister;

impl SkillLister for RealSkillLister {
    fn list(&self) -> Vec<SkillEntry> {
        // Build a fresh registry via register_builtins() each call.
        // Cheap: SkillRegistry::new() + built-in skill construction is
        // allocation-only, no I/O.
        let registry = archon_core::skills::builtin::register_builtins();
        registry
            .list_all()
            .into_iter()
            .map(|(name, description)| SkillEntry {
                name: name.to_string(),
                description: description.to_string(),
            })
            .collect()
    }
}

pub(crate) struct SkillsHandler {
    lister: std::sync::Arc<dyn SkillLister>,
}

impl SkillsHandler {
    pub(crate) fn new() -> Self {
        Self {
            lister: std::sync::Arc::new(RealSkillLister),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_lister(lister: std::sync::Arc<dyn SkillLister>) -> Self {
        Self { lister }
    }
}

impl CommandHandler for SkillsHandler {
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        let skills = self.lister.list();
        if skills.is_empty() {
            return Err(anyhow::anyhow!("no skills available"));
        }
        ctx.emit(TuiEvent::ShowSkillsMenu(skills));
        Ok(())
    }

    fn description(&self) -> &str {
        "Browse and invoke available skills"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_support::*;
    use std::sync::Arc;

    struct MockSkillLister {
        entries: Vec<SkillEntry>,
    }
    impl SkillLister for MockSkillLister {
        fn list(&self) -> Vec<SkillEntry> {
            self.entries.clone()
        }
    }

    fn fixture_entries(n: usize) -> Vec<SkillEntry> {
        (0..n)
            .map(|i| SkillEntry {
                name: format!("skill-{}", i),
                description: format!("desc-{}", i),
            })
            .collect()
    }

    #[test]
    fn no_skills_returns_err() {
        let lister = Arc::new(MockSkillLister { entries: vec![] });
        let handler = SkillsHandler::with_lister(lister);
        let (mut ctx, _rx) = make_bug_ctx();
        let result = handler.execute(&mut ctx, &[]);
        assert!(result.is_err());
        let msg = format!("{:#}", result.unwrap_err()).to_lowercase();
        assert!(
            msg.contains("no skills") || msg.contains("empty"),
            "expected 'no skills' or 'empty'; got: {}",
            msg
        );
    }

    #[test]
    fn with_skills_emits_show_skills_menu() {
        let lister = Arc::new(MockSkillLister {
            entries: fixture_entries(3),
        });
        let handler = SkillsHandler::with_lister(lister);
        let (mut ctx, mut rx) = make_bug_ctx();
        handler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        match &events[0] {
            TuiEvent::ShowSkillsMenu(skills) => {
                assert_eq!(skills.len(), 3, "expected 3 skills, got {}", skills.len());
            }
            other => panic!("expected ShowSkillsMenu, got {:?}", other),
        }
    }

    #[test]
    fn skills_entries_carry_name_and_description() {
        let lister = Arc::new(MockSkillLister {
            entries: fixture_entries(2),
        });
        let handler = SkillsHandler::with_lister(lister);
        let (mut ctx, mut rx) = make_bug_ctx();
        handler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        match &events[0] {
            TuiEvent::ShowSkillsMenu(skills) => {
                assert_eq!(skills[0].name, "skill-0");
                assert_eq!(skills[0].description, "desc-0");
            }
            other => panic!("expected ShowSkillsMenu, got {:?}", other),
        }
    }

    #[test]
    #[ignore = "Gate 5 live smoke — exercises Registry dispatch via default_registry(), run via --ignored"]
    fn skills_dispatches_via_registry() {
        // Gate 5 smoke: Registry::get("skills") must return Some(handler).
        // Dispatches with RealSkillLister (the registered production impl)
        // and accepts BOTH outcomes — non-flaky across test environments:
        //   (a) Ok + ShowSkillsMenu with non-empty skills vec (builtins present).
        //   (b) Err "no skills available" (no builtins in this env).
        // Either path proves dispatch wiring + SkillLister seam run correctly.
        use crate::command::registry::default_registry;

        let registry = default_registry();
        let handler = registry
            .get("skills")
            .expect("skills must be registered in default_registry()");

        let (mut ctx, mut rx) = make_bug_ctx();
        let result = handler.execute(&mut ctx, &[]);

        match result {
            Ok(()) => {
                let events = drain_tui_events(&mut rx);
                assert_eq!(
                    events.len(),
                    1,
                    "expected exactly one TextDelta on Ok path; got: {:?}",
                    events
                );
                match &events[0] {
                    TuiEvent::ShowSkillsMenu(skills) => {
                        assert!(
                            !skills.is_empty(),
                            "Ok path must emit non-empty skills vec; got empty"
                        );
                    }
                    other => panic!("expected ShowSkillsMenu on Ok path, got: {:?}", other),
                }
            }
            Err(e) => {
                let msg = format!("{:#}", e).to_lowercase();
                assert!(
                    msg.contains("no skills") || msg.contains("empty"),
                    "Err path must mention 'no skills' or 'empty'; got: {}",
                    msg
                );
                let events = drain_tui_events(&mut rx);
                assert!(
                    events.is_empty(),
                    "Err path must not emit any events; got: {:?}",
                    events
                );
            }
        }
    }
}
