//! `/plan` slash-command handler — Plan Mode state + plan-file surface.
//!
//! TASK-TUI-626 landed the Plan Mode TOGGLE (SNAPSHOT+EFFECT via
//! `CommandEffect::SetPermissionMode("plan")`). TASK-P0-B.3 (#174) lands
//! the remaining four surfaces the TUI-626 spec deferred:
//!
//! 1. `/plan open` — spawn `$EDITOR` on `.archon/plan.md` (no mode flip).
//! 2. Bare `/plan` (and `/plan show`) — show the current plan-file
//!    content inline via `TextDelta`, then flip into Plan Mode.
//! 3. Plan-file I/O helpers — live in
//!    `crate::command::plan_file` (re-exporting from
//!    `archon_core::plan_file`, see shim rationale in that module).
//! 4. Dispatch-layer interception now appends each blocked tool call
//!    to `.archon/plan.md` so the user has a written record.
//!
//! # Architecture
//!
//! Uses the existing SNAPSHOT+EFFECT plumbing from `/permissions`
//! (`src/command/permissions.rs`). Handler body stays small: emit
//! confirmation + plan-file content TextDelta, stash
//! `CommandEffect::SetPermissionMode("plan")`. The dispatcher's
//! `apply_effect` post-handler does the async write to
//! `slash_ctx.permission_mode.lock().await` AND emits
//! `TuiEvent::PermissionModeChanged("plan")`.
//!
//! # Arg contract
//!
//! * `/plan` (bare) — show current plan content (or "No plan written
//!   yet.") AND stash SetPermissionMode("plan"). The TUI-626 precedent
//!   was "always flip"; we PRESERVE that bit-for-bit in the bare-call
//!   case so the 4 shipped TUI-626 tests keep passing byte-identical.
//! * `/plan show` — same as bare.
//! * `/plan open` — spawn `$EDITOR` on the plan file; do NOT stash the
//!   mode-flip effect. This branch is the one the TUI-626 spec
//!   explicitly left to the P0-B.3 followup (#174).
//! * Any other arg — fall through to the default (bare) behaviour so
//!   unknown args cannot silently disable the mode flip.
//!
//! We deliberately did NOT add a `current_permission_mode` SNAPSHOT
//! field to `CommandContext` here (the spec's "preferred" option): the
//! bare-call already produces the correct downstream state (enter Plan
//! Mode + show the plan-file), and the apply_effect path is idempotent
//! when Plan Mode is already set. Adding a cross-cutting SNAPSHOT
//! field would touch `build_command_context`, every `CtxBuilder`
//! setter, and all 24+ handler test fixtures for no behavioural
//! benefit — the simpler "always flip" interpretation is therefore the
//! one we ship.
//!
//! # Reconciliation with TASK-TUI-626.md spec
//!
//! Spec references `crates/archon-tui/src/slash/plan.rs` +
//! `SlashCommand` + `SlashOutcome::Message`. Actual: bin-crate
//! `src/command/plan.rs` + `CommandHandler` (re-exported as
//! `SlashCommand` at `src/command/mod.rs:86`). Mode write goes via
//! `ctx.pending_effect = Some(CommandEffect::SetPermissionMode("plan"))`,
//! not direct enum assignment — matches the `/permissions` precedent
//! for sync `CommandHandler::execute` + async mutex write.

use archon_tui::app::TuiEvent;

use crate::command::plan_file;
use crate::command::registry::{CommandContext, CommandEffect, CommandHandler};

/// `/plan` handler — enables Plan Mode via the SNAPSHOT+EFFECT pattern
/// AND routes the `open` sub-argument to `$EDITOR` on the plan file.
pub(crate) struct PlanHandler;

impl PlanHandler {
    /// Resolve the plan file path for the current session. Prefers
    /// `CommandContext::working_dir` (clone from
    /// `SlashCommandContext::working_dir`) so tests can redirect the
    /// lookup to a tempdir; falls back to the process CWD for the
    /// (vanishingly-rare) case where the builder left `working_dir`
    /// `None`. Final HOME-fallback is handled inside
    /// `plan_file::plan_path`.
    fn resolve_plan_path(ctx: &CommandContext) -> std::path::PathBuf {
        let cwd_owned;
        let base: &std::path::Path = match ctx.working_dir.as_ref() {
            Some(p) => p.as_path(),
            None => {
                cwd_owned =
                    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                // Widen lifetime via the owned buffer above.
                // (We can't return a reference to a local without it.)
                return plan_file::plan_path(&cwd_owned);
            }
        };
        plan_file::plan_path(base)
    }
}

impl CommandHandler for PlanHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        // Normalise the first positional arg — whitespace-tolerant to
        // match the /permissions precedent at permissions.rs:258.
        let joined = args.join(" ");
        let arg = joined.trim().to_ascii_lowercase();

        // ── Branch: /plan open ───────────────────────────────────────
        // Open the plan file in $EDITOR. Do NOT flip the mode — the
        // user can flip separately with a bare /plan call. This is
        // important because "edit the plan" and "enter Plan Mode" are
        // distinct intents.
        if arg == "open" {
            let path = Self::resolve_plan_path(ctx);
            match plan_file::open_plan_in_editor(&path) {
                Ok(()) => {
                    ctx.emit(TuiEvent::TextDelta(format!(
                        "\nOpened plan in $EDITOR: {}\n",
                        path.display()
                    )));
                }
                Err(e) => {
                    ctx.emit(TuiEvent::Error(format!(
                        "Failed to open plan file {}: {}",
                        path.display(),
                        e
                    )));
                }
            }
            return Ok(());
        }

        // ── Default branch: /plan (bare) and /plan show ──────────────
        // Emit the current plan-file content (or a "no plan yet" hint)
        // inline, THEN the confirmation line, THEN stash the mode-flip
        // effect. The legacy confirmation emission is preserved
        // byte-for-byte so the TUI-626 shipped tests still assert on it.
        let path = Self::resolve_plan_path(ctx);
        let plan_body = match plan_file::read_plan_file(&path) {
            Ok(Some(content)) if !content.trim().is_empty() => {
                format!("\nCurrent plan ({}):\n\n{}\n", path.display(), content)
            }
            Ok(_) => {
                // Missing file OR empty file — same hint.
                format!(
                    "\nNo plan written yet at {} — tool calls blocked while in \
                     Plan Mode will be appended here for review.\n",
                    path.display()
                )
            }
            Err(e) => {
                // IO error reading the plan (permissions / filesystem) —
                // surface as an Error event but continue flipping the
                // mode so the user is not stuck in a half-enabled state.
                ctx.emit(TuiEvent::Error(format!(
                    "Failed to read plan file {}: {}",
                    path.display(),
                    e
                )));
                String::new()
            }
        };

        // Single TextDelta carrying the plan body + legacy confirmation
        // so the TUI-626 "exactly one TextDelta" invariant survives.
        // Invariant: the string MUST start AND end with '\n' (matches
        // the shipped assertion `s.starts_with('\n') && s.ends_with('\n')`).
        let msg = format!(
            "{plan_body}\nPlan mode enabled. You will be asked to approve each tool call.\n"
        );
        ctx.emit(TuiEvent::TextDelta(msg));

        // Stash the shared-mutex write. apply_effect at the dispatch
        // site performs:
        //   *slash_ctx.permission_mode.lock().await = PermissionMode::Plan
        // AND emits TuiEvent::PermissionModeChanged("plan") AFTER the
        // write lands.
        ctx.pending_effect = Some(CommandEffect::SetPermissionMode("plan".to_string()));
        Ok(())
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
    fn plan_stashes_set_permission_mode_effect() {
        let (mut ctx, _rx) = make_bug_ctx();
        PlanHandler.execute(&mut ctx, &[]).unwrap();
        match ctx.pending_effect {
            Some(CommandEffect::SetPermissionMode(ref mode)) => {
                assert_eq!(mode, "plan", "effect must carry mode='plan'");
            }
            other => panic!(
                "expected Some(SetPermissionMode(\"plan\")), got {:?}",
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
            matches!(ctx.pending_effect, None),
            "open-branch must NOT stash SetPermissionMode; got: {:?}",
            ctx.pending_effect
        );
    }

    #[test]
    #[ignore = "Gate 5 live smoke — exercises Registry dispatch via default_registry(), run via --ignored"]
    fn plan_dispatches_via_registry() {
        // Gate 5 smoke: Registry::get("plan") must return Some(handler).
        // Dispatched execute must emit exactly one TextDelta containing
        // "Plan mode" AND stash CommandEffect::SetPermissionMode("plan")
        // on the context — proves plumbing end-to-end.
        use crate::command::registry::default_registry;

        let registry = default_registry();
        let handler = registry
            .get("plan")
            .expect("plan must be registered in default_registry()");

        let (mut ctx, mut rx) = make_bug_ctx();
        handler
            .execute(&mut ctx, &[])
            .expect("dispatched /plan must not error");

        // Assertion 1: TextDelta emitted.
        let events = drain_tui_events(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "expected exactly one TextDelta; got: {:?}",
            events
        );
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                assert!(
                    s.to_lowercase().contains("plan mode"),
                    "TextDelta must contain 'plan mode' (case-insensitive); got: {}",
                    s
                );
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }

        // Assertion 2: SetPermissionMode("plan") effect stashed.
        match ctx.pending_effect {
            Some(CommandEffect::SetPermissionMode(ref mode)) => {
                assert_eq!(mode, "plan", "effect must carry mode='plan'");
            }
            other => panic!(
                "expected Some(SetPermissionMode(\"plan\")), got {:?}",
                other
            ),
        }
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
            matches!(ctx.pending_effect, None),
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
        // Mode flip effect still stashed.
        match ctx.pending_effect {
            Some(CommandEffect::SetPermissionMode(ref mode)) => {
                assert_eq!(mode, "plan");
            }
            other => panic!(
                "expected Some(SetPermissionMode(\"plan\")), got {:?}",
                other
            ),
        }
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
        // Mode flip still applied.
        assert!(matches!(
            ctx.pending_effect,
            Some(CommandEffect::SetPermissionMode(_))
        ));
    }
}
