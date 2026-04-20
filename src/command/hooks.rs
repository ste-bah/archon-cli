//! TASK-AGS-812: /hooks slash-command handler (Option C, gap-fix,
//! DIRECT pattern).
//!
//! NEW primary command — `/hooks` did NOT exist in shipped slash.rs or
//! registry.rs pre-AGS-812. This is the FIRST Q4=A gap-fix ticket to
//! land a real primary (prior gap-fix AGS-805 /cancel only added a
//! registry stub with aliases routing to `Ok(())`). /hooks is the real
//! thing: a DIRECT-pattern handler that enumerates the shipped
//! `HookRegistry` load path and emits the list to the TUI.
//!
//! # Why DIRECT (no snapshot, no effect slot)?
//!
//! `HookRegistry::load_all(project_root, home_dir)` at
//! `crates/archon-core/src/hooks/registry.rs:323` is SYNC (`fn load_all
//! -> HookRegistry`, no `.await` on any source). The new
//! `HookRegistry::summaries()` accessor added in this ticket is also
//! sync (plain `HashMap` iteration with a `Vec` allocation). So the
//! handler body has no async surface:
//!
//! - NO `HooksSnapshot` type (nothing to pre-compute inside an async
//!   guard, unlike `/status` / `/model` / `/cost` / `/mcp`).
//! - NO new `CommandContext` field (builder-side drives nothing for
//!   /hooks — AGS-822 Rule 5 respected: first ticket that ACTUALLY
//!   needs a context field is the first ticket that adds it, and
//!   /hooks does not).
//! - NO `CommandEffect` variant (list subcommand is read-only; write
//!   subcommands — enable / disable / reload — are SCOPE-HELD with a
//!   placeholder TextDelta emit, no actual state mutation).
//!
//! The sole side effect is `ctx.tui_tx.try_send(TuiEvent::TextDelta(..))`
//! — which is sync and legal inside `CommandHandler::execute`. Matches
//! AGS-806 /tasks and AGS-810 /resume DIRECT-pattern precedent.
//!
//! # SCOPE-HELD deferrals
//!
//! Spec `TASK-AGS-812.md` describes a richer surface than shipped
//! infrastructure can support without additional wiring. Per orchestrator
//! decisions (Q1=A sync, Q2=A manual register, Q4=A Option-A minimum
//! viable), the following are SCOPE-HELD and not implemented here:
//!
//! 1. `enable <id>` / `disable <id>` — shipped `HookConfig`
//!    (crates/archon-core/src/hooks/types.rs:70-100) has NO `enabled`
//!    field, so there is nothing to toggle. Subcommand emits a
//!    placeholder TextDelta directing operators at
//!    `~/.archon/settings.json`.
//! 2. `reload` — would require holding `HookRegistry` inside
//!    `SlashCommandContext` and replacing it at runtime. Shipped
//!    context does not expose a hook registry reference. Placeholder
//!    TextDelta.
//! 3. `[✓]` / `[ ]` enabled-marker rendering — depends on the missing
//!    `enabled` field above.
//! 4. `config.save()` persistence — depends on subcommand implementation
//!    above.
//!
//! Each SCOPE-HELD branch emits a uniform
//! `"Hook {sub} command not yet implemented — edit ~/.archon/settings.json directly"`
//! message so operators are informed rather than silently dropped.
//!
//! # Aliases
//!
//! Spec TASK-AGS-812 lists no aliases. Shipped registry has no prior
//! /hooks entry. Match registry row: empty alias slice.

use archon_core::hooks::{HookRegistry, HookSummary};
use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// Zero-sized handler registered as the primary `/hooks` command.
///
/// No aliases. Subcommands dispatched inside `execute`:
/// * `list` (default) — enumerate registered hooks via the new
///   `HookRegistry::summaries()` accessor.
/// * `enable` / `disable` / `reload` — SCOPE-HELD placeholder branch.
/// * any other token — unknown-subcommand hint branch.
pub(crate) struct HooksHandler;

impl CommandHandler for HooksHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        args: &[String],
    ) -> anyhow::Result<()> {
        // `args.first()` is the first positional token after the command
        // name. Empty args (bare `/hooks`) and explicit `list` both map
        // to the list branch. Every other token goes through the
        // three-way match below.
        let sub = args
            .first()
            .map(|s| s.as_str())
            .unwrap_or("list")
            .trim();

        match sub {
            "list" | "" => {
                self.emit_list(ctx);
            }
            "enable" | "disable" | "reload" => {
                let msg = format!(
                    "Hook {sub} command not yet implemented — edit \
                     ~/.archon/settings.json directly"
                );
                let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(msg));
            }
            other => {
                let msg = format!(
                    "Unknown /hooks subcommand: {other}. Valid: list, \
                     enable, disable, reload"
                );
                let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(msg));
            }
        }
        Ok(())
    }

    fn description(&self) -> &'static str {
        "List or manage hook registrations"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }
}

impl HooksHandler {
    /// Render the list output by loading the registry from the current
    /// `.archon/` hierarchy and emitting one line per `HookSummary`.
    ///
    /// The shipped `HookRegistry::load_all(project_root, home_dir)`
    /// reads (in order) `.archon/settings.json`, `~/.archon/hooks.toml`,
    /// `<project>/.archon/hooks.toml`, `<project>/.archon/hooks.local.toml`,
    /// and `~/.archon/policy/hooks.toml`, with `.claude/` fallback for
    /// backward compatibility and `(event, hook_type, command)`
    /// deduplication. This handler inherits that resolution.
    ///
    /// `project_root` comes from `std::env::current_dir()`. `home_dir`
    /// comes from `dirs::home_dir()` — same crate `doctor`, `plugin`,
    /// and `slash.rs` already use at several sites.
    fn emit_list(&self, ctx: &mut CommandContext) {
        let project_root = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."));
        let home_dir =
            dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));

        let registry = HookRegistry::load_all(&project_root, &home_dir);
        let summaries = registry.summaries();

        let text = render_list(&summaries);
        let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(text));
    }
}

/// Pure text renderer, factored out for direct unit testing without
/// touching filesystem or TUI channel.
fn render_list(summaries: &[HookSummary]) -> String {
    let mut lines: Vec<String> = Vec::with_capacity(summaries.len() + 2);
    lines.push(format!("Registered hooks ({}):", summaries.len()));
    if summaries.is_empty() {
        lines.push("(no hooks registered)".to_string());
    } else {
        for s in summaries {
            let matcher_label = s.matcher.as_deref().unwrap_or("*");
            let source_label = s.source.as_deref().unwrap_or("(none)");
            lines.push(format!(
                "  {:?} {} -> {} [source: {}]",
                s.event, matcher_label, s.command, source_label
            ));
        }
    }
    lines.join("\n")
}

// ---------------------------------------------------------------------------
// TASK-AGS-812: tests for /hooks slash-command gap-fix
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use archon_core::hooks::{HookEvent, HookSummary};
    use archon_tui::app::TuiEvent;
    use tokio::sync::mpsc;

    /// Build a `CommandContext` with a freshly-created channel.
    /// /hooks is a DIRECT-pattern handler — no snapshot, no effect slot
    /// — so every optional field stays `None`. Mirrors the make_ctx
    /// fixtures in resume.rs / task.rs / cost.rs / model.rs / status.rs.
    fn make_ctx() -> (CommandContext, mpsc::Receiver<TuiEvent>) {
        let (tx, rx) = mpsc::channel::<TuiEvent>(16);
        (
            CommandContext {
                tui_tx: tx,
                status_snapshot: None,
                model_snapshot: None,
                cost_snapshot: None,
                mcp_snapshot: None,
                // TASK-AGS-814: /hooks tests never exercise /context paths — None.
                context_snapshot: None,
                // TASK-AGS-815: /hooks tests never exercise /fork paths — None.
                session_id: None,
                // TASK-AGS-817: /hooks tests never exercise /memory paths — None.
                memory: None,
                // TASK-AGS-POST-6-BODIES-B01-FAST: /hooks tests never exercise /fast paths — None.
                fast_mode_shared: None,
                // TASK-AGS-POST-6-BODIES-B02-THINKING: /hooks tests never exercise /thinking paths — None.
                show_thinking: None,
                // TASK-AGS-POST-6-BODIES-B04-DIFF: /hooks tests never exercise /diff paths — None.
                working_dir: None,
                // TASK-AGS-POST-6-BODIES-B06-HELP: /hooks tests never exercise /help paths — None.
                skill_registry: None,
                // TASK-AGS-POST-6-BODIES-B08-DENIALS: /hooks tests never exercise /denials paths — None.
                denial_snapshot: None,
                effort_snapshot: None,
                pending_effect: None,
                pending_effort_set: None,
            },
            rx,
        )
    }

    #[test]
    fn hooks_handler_description_matches() {
        let h = HooksHandler;
        let desc = h.description().to_lowercase();
        assert!(
            desc.contains("hook"),
            "HooksHandler description should reference 'hook', got: {}",
            h.description()
        );
    }

    #[test]
    fn hooks_handler_has_no_aliases() {
        let h = HooksHandler;
        assert_eq!(
            h.aliases(),
            &[] as &[&'static str],
            "HooksHandler must have an empty alias slice per AGS-812 \
             (spec lists no aliases; shipped registry had no prior \
             /hooks entry)"
        );
    }

    /// `list` (and the bare no-arg form) must emit a header-bearing
    /// `TextDelta` even when the disk registry is empty or absent.
    /// Whatever the operator's `.archon/` state, the output MUST include
    /// the canonical prefix `"Registered hooks ("` so the rest of the
    /// TUI and any downstream scraping can pivot off a stable marker.
    #[test]
    fn hooks_handler_list_emits_registered_hooks_header() {
        let (mut ctx, mut rx) = make_ctx();
        let h = HooksHandler;
        let res = h.execute(&mut ctx, &["list".to_string()]);
        assert!(
            res.is_ok(),
            "HooksHandler::execute(list) must return Ok(()), got: {res:?}"
        );

        let mut saw_header = false;
        while let Ok(ev) = rx.try_recv() {
            if let TuiEvent::TextDelta(text) = ev {
                if text.contains("Registered hooks (") {
                    saw_header = true;
                }
            }
        }
        assert!(
            saw_header,
            "HooksHandler::execute(list) must emit a TextDelta starting \
             with 'Registered hooks (' regardless of disk registry state"
        );
    }

    /// Unknown subcommand must emit a friendly one-line hint rather than
    /// silently dropping, panicking, or returning Err(..). The hint
    /// text must enumerate the four valid subcommand tokens so the
    /// operator can pivot without opening the source.
    #[test]
    fn hooks_handler_unknown_subcommand_emits_hint() {
        let (mut ctx, mut rx) = make_ctx();
        let h = HooksHandler;
        let res = h.execute(&mut ctx, &["bogus-sub".to_string()]);
        assert!(
            res.is_ok(),
            "HooksHandler::execute(bogus-sub) must return Ok(()), got: {res:?}"
        );

        let mut saw_hint = false;
        while let Ok(ev) = rx.try_recv() {
            if let TuiEvent::TextDelta(text) = ev {
                if text.contains("Unknown /hooks subcommand")
                    && text.contains("list")
                    && text.contains("enable")
                    && text.contains("disable")
                    && text.contains("reload")
                {
                    saw_hint = true;
                }
            }
        }
        assert!(
            saw_hint,
            "HooksHandler::execute(unknown) must emit a hint TextDelta \
             naming all four valid subcommands"
        );
    }

    /// `enable` / `disable` / `reload` are SCOPE-HELD for AGS-812 —
    /// they emit a placeholder TextDelta pointing at
    /// `~/.archon/settings.json` rather than mutating state. Pin the
    /// placeholder text so a later ticket that upgrades the subcommand
    /// path has to consciously change this test (preventing silent
    /// regressions of the advisory UX).
    #[test]
    fn hooks_handler_enable_disable_reload_emit_placeholder() {
        for sub in ["enable", "disable", "reload"] {
            let (mut ctx, mut rx) = make_ctx();
            let h = HooksHandler;
            let res = h.execute(&mut ctx, &[sub.to_string()]);
            assert!(
                res.is_ok(),
                "HooksHandler::execute({sub}) must return Ok(()), \
                 got: {res:?}"
            );

            let mut saw_placeholder = false;
            while let Ok(ev) = rx.try_recv() {
                if let TuiEvent::TextDelta(text) = ev {
                    if text.contains(&format!("Hook {sub} command not yet implemented"))
                        && text.contains("~/.archon/settings.json")
                    {
                        saw_placeholder = true;
                    }
                }
            }
            assert!(
                saw_placeholder,
                "HooksHandler::execute({sub}) must emit the canonical \
                 SCOPE-HELD placeholder TextDelta"
            );
        }
    }

    /// Pure `render_list` must produce a `(N):` header matching the
    /// summary count and a `(no hooks registered)` line for the empty
    /// case. Guards the renderer's contract independent of the I/O
    /// path exercised by `hooks_handler_list_emits_registered_hooks_header`.
    #[test]
    fn render_list_formats_header_and_handles_empty_and_populated() {
        // Empty.
        let out_empty = render_list(&[]);
        assert!(
            out_empty.starts_with("Registered hooks (0):"),
            "empty render_list must start with 'Registered hooks (0):', \
             got: {out_empty}"
        );
        assert!(
            out_empty.contains("(no hooks registered)"),
            "empty render_list must emit '(no hooks registered)', got: {out_empty}"
        );

        // Populated.
        let summaries = vec![
            HookSummary {
                event: HookEvent::PreToolUse,
                matcher: Some("Bash".to_string()),
                command: "guard-secrets".to_string(),
                source: Some("project".to_string()),
            },
            HookSummary {
                event: HookEvent::SessionStart,
                matcher: None,
                command: "welcome.sh".to_string(),
                source: None,
            },
        ];
        let out_populated = render_list(&summaries);
        assert!(
            out_populated.starts_with("Registered hooks (2):"),
            "populated render_list must start with 'Registered hooks (2):', \
             got: {out_populated}"
        );
        assert!(
            out_populated.contains("PreToolUse Bash -> guard-secrets [source: project]"),
            "populated render_list must emit the first hook verbatim, got: {out_populated}"
        );
        assert!(
            out_populated.contains("SessionStart * -> welcome.sh [source: (none)]"),
            "populated render_list must use '*' for missing matcher and \
             '(none)' for missing source, got: {out_populated}"
        );
    }
}
