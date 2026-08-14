//! `/plan` slash-command handler — typed Plan Mode lifecycle and plan-file surface.
//!
//! The synchronous handler only reads its owned [`PlanSnapshot`] and emits an
//! owned [`CommandEffect`]. `apply_effect` records the shared lifecycle state
//! before it changes the lock-protected permission mode.

use archon_permissions::mode::PermissionMode;
use archon_tui::app::TuiEvent;

use crate::command::plan_file;
use crate::command::registry::{CommandContext, CommandEffect, CommandHandler};

#[derive(Clone, Debug)]
pub(crate) struct PlanSnapshot {
    pub(crate) current_mode: PermissionMode,
}

/// `/plan` handler — displays the plan file and changes mode through effects.
pub(crate) struct PlanHandler;

impl PlanHandler {
    fn resolve_plan_path(ctx: &CommandContext) -> std::path::PathBuf {
        let cwd_owned;
        let base: &std::path::Path = match ctx.working_dir.as_ref() {
            Some(path) => path.as_path(),
            None => {
                cwd_owned =
                    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                return plan_file::plan_path(&cwd_owned);
            }
        };
        plan_file::plan_path(base)
    }

    fn enter_or_show(ctx: &mut CommandContext) -> anyhow::Result<()> {
        let current_mode = ctx
            .plan_snapshot
            .as_ref()
            .map(|snapshot| snapshot.current_mode)
            .unwrap_or_default();
        if current_mode == PermissionMode::Plan {
            ctx.emit(TuiEvent::TextDelta(
                "Already in plan mode — /plan off to exit, or tell the agent to proceed."
                    .to_string(),
            ));
            return Ok(());
        }

        let path = Self::resolve_plan_path(ctx);
        let plan_body = match plan_file::read_plan_file(&path) {
            Ok(Some(content)) if !content.trim().is_empty() => {
                format!("\nCurrent plan ({}):\n\n{}\n", path.display(), content)
            }
            Ok(_) => format!(
                "\nNo plan written yet at {} — tool calls blocked while in \
                 Plan Mode will be appended here for review.\n",
                path.display()
            ),
            Err(error) => {
                ctx.emit(TuiEvent::Error(format!(
                    "Failed to read plan file {}: {}",
                    path.display(),
                    error
                )));
                String::new()
            }
        };
        ctx.emit(TuiEvent::TextDelta(format!(
            "{plan_body}\nPlan mode enabled. Use /plan off to exit.\n"
        )));
        ctx.pending_effect = Some(CommandEffect::EnterPlanMode {
            previous_mode: current_mode,
        });
        Ok(())
    }

    fn open_document(ctx: &mut CommandContext) -> anyhow::Result<()> {
        let path = Self::resolve_plan_path(ctx);
        match plan_file::open_plan_in_editor(&path) {
            Ok(()) => ctx.emit(TuiEvent::TextDelta(format!(
                "\nOpened plan in $EDITOR: {}\n",
                path.display()
            ))),
            Err(error) => ctx.emit(TuiEvent::Error(format!(
                "Failed to open plan file {}: {}",
                path.display(),
                error
            ))),
        }
        Ok(())
    }

    fn exit_plan(ctx: &mut CommandContext) -> anyhow::Result<()> {
        ctx.emit(TuiEvent::TextDelta("Plan mode disabled.".to_string()));
        ctx.pending_effect = Some(CommandEffect::SetPermissionMode("default".to_string()));
        Ok(())
    }

    fn emit_valid_forms_error(ctx: &mut CommandContext) -> anyhow::Result<()> {
        ctx.emit(TuiEvent::Error(
            "Valid /plan forms: show, open, off, exit, done.".to_string(),
        ));
        Ok(())
    }
}

impl CommandHandler for PlanHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        let arg = args.join(" ").trim().to_ascii_lowercase();
        match arg.as_str() {
            "" | "show" => Self::enter_or_show(ctx),
            "open" => Self::open_document(ctx),
            "off" | "exit" | "done" => Self::exit_plan(ctx),
            _ => Self::emit_valid_forms_error(ctx),
        }
    }

    fn description(&self) -> &str {
        "Enable Plan Mode (approve each tool call individually)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_support::*;

    #[test]
    fn plan_emits_confirmation_textdelta() {
        let (mut ctx, mut rx) = make_bug_ctx();
        PlanHandler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                assert!(
                    s.to_lowercase().contains("plan mode"),
                    "TextDelta must mention 'Plan mode'; got: {}",
                    s
                );
                assert!(
                    s.starts_with('\n') && s.ends_with('\n'),
                    "TextDelta must carry leading+trailing \\n wrap; got: {:?}",
                    s
                );
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn plan_stashes_enter_plan_mode_effect() {
        let (mut ctx, _rx) = make_bug_ctx();
        PlanHandler.execute(&mut ctx, &[]).unwrap();
        match ctx.pending_effect {
            Some(CommandEffect::EnterPlanMode { previous_mode }) => {
                assert_eq!(
                    previous_mode,
                    archon_permissions::mode::PermissionMode::Default
                );
            }
            other => panic!(
                "expected Some(EnterPlanMode {{ previous_mode: Default }}), got {:?}",
                other
            ),
        }
    }

    #[test]
    fn plan_ignores_trailing_args() {
        // TASK-P0-B.3 (#174): trailing "open" now has a new meaning
        // (spawn $EDITOR, do NOT flip mode). The existing test name is
        // preserved to keep git-blame continuity; the body asserts the
        // new "open" branch (text contains "Opened plan" OR a
        // Failed-to-open error, zero effect stashed).
        //
        // NOTE: EDITOR defaults to `vi` / `notepad` which would be
        // interactive; wrap the whole test with EDITOR=true (the
        // no-op success binary) via an env override so CI stays
        // non-interactive. Use a tempdir for working_dir so the test
        // is hermetic (no touching the real worktree's .archon/).
        unsafe {
            std::env::set_var("EDITOR", "true");
        }
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".archon")).unwrap();
        let (mut ctx, mut rx) = CtxBuilder::new()
            .with_working_dir(tmp.path().to_path_buf())
            .build();
        PlanHandler
            .execute(&mut ctx, &[String::from("open")])
            .unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "open-branch must emit exactly one event; got: {:?}",
            events
        );
        // After P0-B.3 the /plan open branch does NOT stash the mode
        // effect — opening the plan file and entering Plan Mode are
        // distinct intents.
        assert!(
            ctx.pending_effect.is_none(),
            "open-branch must NOT stash SetPermissionMode; got: {:?}",
            ctx.pending_effect
        );
    }

    #[test]
    fn plan_off_stashes_default_and_confirms_exit() {
        let (mut ctx, mut rx) = make_bug_ctx();
        PlanHandler
            .execute(&mut ctx, &[String::from("off")])
            .unwrap();

        assert!(matches!(
            ctx.pending_effect.as_ref(),
            Some(CommandEffect::SetPermissionMode(mode)) if mode == "default"
        ));
        let events = drain_tui_events(&mut rx);
        assert!(matches!(
            events.as_slice(),
            [TuiEvent::TextDelta(message)] if message.contains("Plan mode disabled")
        ));
    }

    #[test]
    fn plan_exit_and_done_alias_off() {
        for alias in ["exit", "done"] {
            let (mut ctx, _rx) = make_bug_ctx();
            PlanHandler.execute(&mut ctx, &[alias.to_string()]).unwrap();
            assert!(matches!(
                ctx.pending_effect.as_ref(),
                Some(CommandEffect::SetPermissionMode(mode)) if mode == "default"
            ));
        }
    }

    #[test]
    fn plan_bogus_errors_and_does_not_change_mode() {
        let (mut ctx, mut rx) = make_bug_ctx();
        PlanHandler
            .execute(&mut ctx, &[String::from("bogus")])
            .unwrap();

        assert!(ctx.pending_effect.is_none());
        let events = drain_tui_events(&mut rx);
        assert!(matches!(
            events.as_slice(),
            [TuiEvent::Error(message)] if ["show", "open", "off", "exit", "done"]
                .iter()
                .all(|form| message.contains(form))
        ));
    }

    #[test]
    fn plan_while_active_reports_already_in_plan_mode() {
        let (mut ctx, mut rx) = CtxBuilder::new()
            .with_plan_snapshot(crate::command::plan::PlanSnapshot {
                current_mode: archon_permissions::mode::PermissionMode::Plan,
            })
            .build();
        PlanHandler.execute(&mut ctx, &[]).unwrap();

        assert!(ctx.pending_effect.is_none());
        let events = drain_tui_events(&mut rx);
        assert!(matches!(
            events.as_slice(),
            [TuiEvent::TextDelta(message)]
                if message == "Already in plan mode — /plan off to exit, or tell the agent to proceed."
        ));
    }

    #[test]
    fn plan_is_registered_and_stashes_entry_effect() {
        use crate::command::registry::default_registry;

        let registry = default_registry();
        let handler = registry
            .get("plan")
            .expect("plan must be registered in default_registry()");

        let (mut ctx, mut rx) = make_bug_ctx();
        handler
            .execute(&mut ctx, &[])
            .expect("dispatched /plan must not error");

        let events = drain_tui_events(&mut rx);
        assert!(
            matches!(events.as_slice(), [TuiEvent::TextDelta(message)] if message.to_lowercase().contains("plan mode"))
        );
        assert!(matches!(
            ctx.pending_effect,
            Some(CommandEffect::EnterPlanMode { .. })
        ));
    }

    // ─────────────────────────────────────────────────────────────────
    // TASK-P0-B.3 (#174) new tests
    // ─────────────────────────────────────────────────────────────────

    /// `/plan open` spawns `$EDITOR` and reports success. We set
    /// EDITOR=`true` (the no-op success binary) so the test stays
    /// non-interactive and CI-safe.
    #[test]
    fn plan_open_spawns_editor_and_reports_path() {
        unsafe {
            std::env::set_var("EDITOR", "true");
        }
        // Point the plan path at a fresh tempdir so the test does not
        // touch the user's real `.archon/plan.md`.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".archon")).unwrap();

        let (mut ctx, mut rx) = CtxBuilder::new()
            .with_working_dir(tmp.path().to_path_buf())
            .build();

        PlanHandler
            .execute(&mut ctx, &[String::from("open")])
            .unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                assert!(
                    s.contains("Opened plan"),
                    "expected 'Opened plan' text; got: {}",
                    s
                );
                assert!(
                    s.contains("plan.md"),
                    "expected plan.md path in output; got: {}",
                    s
                );
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }
        // `/plan open` must NOT flip mode.
        assert!(
            ctx.pending_effect.is_none(),
            "open-branch must NOT stash SetPermissionMode; got: {:?}",
            ctx.pending_effect
        );
        // And it must have created the file (so the editor always
        // opens into a real file, not a blank buffer).
        assert!(tmp.path().join(".archon").join("plan.md").exists());
    }

    /// Bare `/plan` with an existing plan file MUST echo the plan
    /// content back to the user AND still flip Plan Mode on.
    #[test]
    fn plan_reads_existing_plan_file_and_flips_mode() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".archon")).unwrap();
        let path = tmp.path().join(".archon").join("plan.md");
        std::fs::write(&path, "# My plan\n\n- step 1\n- step 2\n").unwrap();

        let (mut ctx, mut rx) = CtxBuilder::new()
            .with_working_dir(tmp.path().to_path_buf())
            .build();

        PlanHandler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "expected a single TextDelta; got {:?}",
            events
        );
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                assert!(
                    s.contains("- step 1"),
                    "plan file content must appear in TextDelta; got: {}",
                    s
                );
                assert!(
                    s.to_lowercase().contains("plan mode"),
                    "legacy confirmation line must still appear; got: {}",
                    s
                );
                assert!(
                    s.starts_with('\n') && s.ends_with('\n'),
                    "TextDelta must carry leading+trailing \\n wrap; got: {:?}",
                    s
                );
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }
        assert!(matches!(
            ctx.pending_effect,
            Some(CommandEffect::EnterPlanMode {
                previous_mode: PermissionMode::Default
            })
        ));
    }

    /// Bare `/plan` with NO plan file emits a "no plan yet" hint AND
    /// still flips Plan Mode on.
    #[test]
    fn plan_reports_no_plan_yet_when_file_absent() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".archon")).unwrap();
        // Deliberately DO NOT create plan.md.

        let (mut ctx, mut rx) = CtxBuilder::new()
            .with_working_dir(tmp.path().to_path_buf())
            .build();

        PlanHandler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                assert!(
                    s.contains("No plan written yet"),
                    "expected 'No plan written yet' hint; got: {}",
                    s
                );
                assert!(
                    s.to_lowercase().contains("plan mode"),
                    "legacy confirmation must still appear; got: {}",
                    s
                );
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }
        assert!(matches!(
            ctx.pending_effect,
            Some(CommandEffect::EnterPlanMode {
                previous_mode: PermissionMode::Default
            })
        ));
    }
}
