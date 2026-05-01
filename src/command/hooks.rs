//! `/hooks` slash-command handler — list, enable, disable, reload.
//!
//! GHOST-004 replaces the AGS-812 scope-held placeholders with real
//! dispatch through the session-shared `Arc<HookRegistry>` on `CommandContext`.
//!
//! Subcommands:
//! * `list` (default) — enumerate registered hooks with id and `[✓]`/`[ ]` markers
//! * `enable <id>` — enable a hook by id, persists to `.archon/hooks.local.toml`
//! * `disable <id>` — disable a hook by id, persists to `.archon/hooks.local.toml`
//! * `reload` — re-read all hook sources from disk

use archon_core::hooks::HookSummary;
use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

pub(crate) struct HooksHandler;

impl CommandHandler for HooksHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        let sub = args.first().map(|s| s.as_str()).unwrap_or("list").trim();

        match sub {
            "list" | "" => {
                self.emit_list(ctx);
            }
            "enable" | "disable" => {
                let enabled = sub == "enable";
                let id = match args.get(1) {
                    Some(id) if !id.is_empty() => id.as_str(),
                    _ => {
                        ctx.emit(TuiEvent::TextDelta(format!(
                            "Usage: /hooks {sub} <hook-id>\n"
                        )));
                        return Ok(());
                    }
                };
                self.set_enabled(ctx, id, enabled);
            }
            "reload" => {
                self.do_reload(ctx);
            }
            other => {
                let msg = format!(
                    "Unknown /hooks subcommand: {other}. Only 'list' is \
                     currently functional (enable/disable/reload are not yet implemented)"
                );
                ctx.emit(TuiEvent::TextDelta(msg));
            }
        }
        Ok(())
    }

    fn description(&self) -> &'static str {
        "List hook registrations (enable/disable/reload not yet implemented)"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }
}

impl HooksHandler {
    fn emit_list(&self, ctx: &mut CommandContext) {
        let summaries = match ctx.hook_registry.as_ref() {
            Some(reg) => reg.summaries(),
            None => {
                ctx.emit(TuiEvent::TextDelta(
                    "\nHook registry not available (session boot wiring gap).\n".to_string(),
                ));
                return;
            }
        };

        let text = render_list(&summaries);
        ctx.emit(TuiEvent::TextDelta(text));
    }

    fn set_enabled(&self, ctx: &mut CommandContext, id: &str, enabled: bool) {
        let reg = match ctx.hook_registry.as_ref() {
            Some(r) => r,
            None => {
                ctx.emit(TuiEvent::TextDelta(
                    "\nHook registry not available (session boot wiring gap).\n".to_string(),
                ));
                return;
            }
        };

        match reg.set_enabled(id, enabled) {
            Ok(()) => {
                let state = if enabled { "enabled" } else { "disabled" };
                ctx.emit(TuiEvent::TextDelta(format!("\nHook {id} {state}.\n")));
            }
            Err(e) => {
                ctx.emit(TuiEvent::TextDelta(format!(
                    "\nFailed to {action} hook {id}: {e}\n",
                    action = if enabled { "enable" } else { "disable" }
                )));
            }
        }
    }

    fn do_reload(&self, ctx: &mut CommandContext) {
        let reg = match ctx.hook_registry.as_ref() {
            Some(r) => r,
            None => {
                ctx.emit(TuiEvent::TextDelta(
                    "\nHook registry not available (session boot wiring gap).\n".to_string(),
                ));
                return;
            }
        };

        match reg.reload() {
            Ok(()) => {
                let count = reg.hook_count();
                ctx.emit(TuiEvent::TextDelta(format!(
                    "\nHook registry reloaded ({count} hooks).\n"
                )));
            }
            Err(e) => {
                ctx.emit(TuiEvent::TextDelta(format!(
                    "\nFailed to reload hook registry: {e}\n"
                )));
            }
        }
    }
}

/// Pure text renderer. Each line shows: enabled marker, id, event, matcher, command, source.
fn render_list(summaries: &[HookSummary]) -> String {
    let mut lines: Vec<String> = Vec::with_capacity(summaries.len() + 2);
    lines.push(format!("Registered hooks ({}):", summaries.len()));
    if summaries.is_empty() {
        lines.push("(no hooks registered)".to_string());
    } else {
        for s in summaries {
            let marker = if s.enabled { "[✓]" } else { "[ ]" };
            let matcher_label = s.matcher.as_deref().unwrap_or("*");
            let source_label = s.source.as_deref().unwrap_or("(none)");
            lines.push(format!(
                "  {} {} {:?} {} -> {} [source: {}]",
                marker, s.id, s.event, matcher_label, s.command, source_label
            ));
        }
    }
    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use archon_core::hooks::{HookEvent, HookSummary};
    use archon_tui::app::TuiEvent;
    use tokio::sync::mpsc;

    fn make_ctx() -> (CommandContext, mpsc::UnboundedReceiver<TuiEvent>) {
        crate::command::test_support::CtxBuilder::new().build()
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
        assert_eq!(h.aliases(), &[] as &[&'static str]);
    }

    #[test]
    fn hooks_handler_list_emits_registered_hooks_header() {
        let (mut ctx, mut rx) = make_ctx();
        // Without a hook_registry, list should emit the "not available" message.
        let h = HooksHandler;
        let res = h.execute(&mut ctx, &["list".to_string()]);
        assert!(res.is_ok());

        let mut saw_output = false;
        while let Ok(ev) = rx.try_recv() {
            if let TuiEvent::TextDelta(text) = ev
                && (text.contains("Registered hooks (") || text.contains("not available"))
            {
                saw_output = true;
            }
        }
        assert!(saw_output, "list must emit output even without a registry");
    }

    #[test]
    fn hooks_handler_unknown_subcommand_emits_hint() {
        let (mut ctx, mut rx) = make_ctx();
        let h = HooksHandler;
        let res = h.execute(&mut ctx, &["bogus-sub".to_string()]);
        assert!(res.is_ok());

        let mut saw_hint = false;
        while let Ok(ev) = rx.try_recv() {
            if let TuiEvent::TextDelta(text) = ev
                && text.contains("Unknown /hooks subcommand")
                && text.contains("list")
                && text.contains("enable")
                && text.contains("disable")
                && text.contains("reload")
            {
                saw_hint = true;
            }
        }
        assert!(saw_hint);
    }

    #[test]
    fn hooks_handler_enable_without_id_shows_usage() {
        let (mut ctx, mut rx) = make_ctx();
        let h = HooksHandler;
        let res = h.execute(&mut ctx, &["enable".to_string()]);
        assert!(res.is_ok());

        let mut saw_usage = false;
        while let Ok(ev) = rx.try_recv() {
            if let TuiEvent::TextDelta(text) = ev
                && text.contains("Usage: /hooks enable")
            {
                saw_usage = true;
            }
        }
        assert!(saw_usage);
    }

    #[test]
    fn hooks_handler_disable_without_id_shows_usage() {
        let (mut ctx, mut rx) = make_ctx();
        let h = HooksHandler;
        let res = h.execute(&mut ctx, &["disable".to_string()]);
        assert!(res.is_ok());

        let mut saw_usage = false;
        while let Ok(ev) = rx.try_recv() {
            if let TuiEvent::TextDelta(text) = ev
                && text.contains("Usage: /hooks disable")
            {
                saw_usage = true;
            }
        }
        assert!(saw_usage);
    }

    #[test]
    fn hooks_handler_enable_disable_reload_without_registry_shows_error() {
        for sub in ["enable", "disable", "reload"] {
            let (mut ctx, mut rx) = make_ctx();
            let h = HooksHandler;
            // Build args with an id for enable/disable so we exercise the registry path.
            let args: Vec<String> = if sub == "reload" {
                vec![sub.to_string()]
            } else {
                vec![sub.to_string(), "h00000000".to_string()]
            };
            let res = h.execute(&mut ctx, &args);
            assert!(res.is_ok(), "{sub} must return Ok(())");

            let mut saw_error = false;
            while let Ok(ev) = rx.try_recv() {
                if let TuiEvent::TextDelta(text) = ev
                    && text.contains("not available")
                {
                    saw_error = true;
                }
            }
            assert!(
                saw_error,
                "{sub} without registry must emit 'not available' error"
            );
        }
    }

    #[test]
    fn render_list_formats_header_and_handles_empty_and_populated() {
        // Empty.
        let out_empty = render_list(&[]);
        assert!(out_empty.starts_with("Registered hooks (0):"));
        assert!(out_empty.contains("(no hooks registered)"));

        // Populated.
        let summaries = vec![
            HookSummary {
                id: "h4f2a1b9".to_string(),
                event: HookEvent::PreToolUse,
                matcher: Some("Bash".to_string()),
                command: "guard-secrets".to_string(),
                source: Some("project".to_string()),
                enabled: true,
            },
            HookSummary {
                id: "h8c3d5e7".to_string(),
                event: HookEvent::SessionStart,
                matcher: None,
                command: "welcome.sh".to_string(),
                source: None,
                enabled: false,
            },
        ];
        let out_populated = render_list(&summaries);
        assert!(out_populated.starts_with("Registered hooks (2):"));
        assert!(
            out_populated.contains("[✓] h4f2a1b9"),
            "enabled hook must show [✓] marker and id"
        );
        assert!(
            out_populated.contains("[ ] h8c3d5e7"),
            "disabled hook must show [ ] marker and id"
        );
        assert!(out_populated.contains("PreToolUse Bash -> guard-secrets [source: project]"));
        assert!(out_populated.contains("SessionStart * -> welcome.sh [source: (none)]"));
    }

    #[test]
    fn render_list_shows_enabled_markers() {
        let summaries = vec![
            HookSummary {
                id: "h11111111".to_string(),
                event: HookEvent::PreToolUse,
                matcher: None,
                command: "enabled-hook".to_string(),
                source: None,
                enabled: true,
            },
            HookSummary {
                id: "h22222222".to_string(),
                event: HookEvent::PostToolUse,
                matcher: None,
                command: "disabled-hook".to_string(),
                source: None,
                enabled: false,
            },
        ];
        let out = render_list(&summaries);
        assert!(out.contains("[✓] h11111111"));
        assert!(out.contains("[ ] h22222222"));
    }
}
