//! TASK-#213 SLASH-REFRESH — `/refresh` slash-command handler.
//!
//! Triggers a re-scan of the locally-loaded `AgentRegistry` so the user
//! picks up newly-added agent definitions on disk
//! (`.archon/agents/custom/`, `.archon/plugins/*/agents/`,
//! `~/.archon/agents/custom/`, `~/.archon/plugins/*/agents/`) without
//! restarting the session.
//!
//! Reads `ctx.agent_registry` (DIRECT field added in #211) and
//! `ctx.working_dir`, acquires the registry's `RwLock::write()` guard
//! synchronously (`std::sync::RwLock`, not `tokio::Mutex`), captures
//! the before/after agent counts, calls `AgentRegistry::reload(
//! &working_dir)`, drops the guard, and emits a single
//! `TuiEvent::TextDelta` summarising the delta.
//!
//! # Spec/reality reconciliation
//!
//! The mission ticket said "rescan agents/skills/plugins". Reality on
//! this branch:
//!
//!   1. Agents (`AgentRegistry`): refresh is fully supported. The
//!      registry is wrapped in `Arc<RwLock<AgentRegistry>>` and
//!      `reload(&Path)` is sync. Plugin AGENTS (`.archon/plugins/*/
//!      agents/`) are part of the same scan, so they refresh in lockstep
//!      with custom agents.
//!   2. Skills (`SkillRegistry`): the session stores
//!      `Arc<SkillRegistry>` (no `Mutex`/`RwLock` wrapper at
//!      `src/slash_context.rs:40`), so the registry is immutable after
//!      session bootstrap. Re-scanning skills would require either:
//!        (a) wrapping `SlashCommandContext::skill_registry` in
//!            `Arc<Mutex<SkillRegistry>>` and threading the Mutex
//!            through every consumer, or
//!        (b) replacing the Arc inside `SlashCommandContext` itself
//!            (which would need `&mut SlashCommandContext` reachable
//!            from `apply_effect`).
//!      Both options are cross-cutting subsystem refactors — well over
//!      the wrapper-scope ceiling for this ticket. Deferred to a
//!      follow-up.
//!   3. Plugins (WASM `WasmPluginHost`): there is no session-shared
//!      plugin manager. The `archon-plugin` crate exposes
//!      `WasmPluginHost::load_plugin` but no `reload_plugins` API. The
//!      mission spec lists ticket #217 SLASH-RELOAD-PLUGINS as the
//!      dedicated hot-reload path; #213 defers WASM-plugin refresh to
//!      that ticket.
//!
//! Resolution per the mission "Spec-reality drift → reconcile, adapt
//! scope, document in commit body" rule:
//!
//!   - Ship `/refresh` as the AGENTS-only refresh today (which also
//!     covers plugin AGENT files, since they live under the same
//!     scan). The rendered TextDelta explicitly flags the skills and
//!     WASM-plugin deferrals so users see the deferral, not a missing
//!     feature. This is the most useful slice we can ship without
//!     subsystem-level plumbing.
//!   - Distinct from `/reload` (config-only — re-reads the TOML
//!     config + diffs changed keys; see `src/command/reload.rs`).

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// `/refresh` handler — re-scan the agent registry from disk.
pub(crate) struct RefreshHandler;

impl CommandHandler for RefreshHandler {
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        let registry_arc = ctx.agent_registry.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "RefreshHandler invoked without agent_registry populated \
                 — build_command_context bug"
            )
        })?;

        let working_dir = ctx.working_dir.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "RefreshHandler invoked without working_dir populated \
                 — build_command_context bug"
            )
        })?;

        // Acquire the write lock + capture before-count + reload +
        // capture after-count + collect any error report — all under
        // the same critical section so concurrent readers see a
        // consistent before-or-after view, never an in-flight scan.
        let (before, after, error_count) = {
            let mut guard = match registry_arc.write() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            let before = guard.len();
            guard.reload(working_dir);
            let after = guard.len();
            let error_count = guard.load_errors().len();
            (before, after, error_count)
        };

        let delta_label = format_delta(before, after);
        let mut msg = String::with_capacity(384);
        msg.push('\n');
        msg.push_str("Refreshed agent registry.\n");
        msg.push_str(&format!("  Agents:   {after} loaded ({delta_label})\n"));
        if error_count > 0 {
            msg.push_str(&format!(
                "  Errors:   {error_count} agent file(s) failed to load \
                 (check `archon agent list --include-invalid` for details)\n"
            ));
        }
        msg.push_str("  Skills:   refresh deferred (registry is immutable in this session;\n");
        msg.push_str("            session restart required — follow-up ticket tracked).\n");
        msg.push_str("  Plugins:  WASM plugin hot-reload deferred to /reload-plugins (#217).\n");
        msg.push_str(
            "            Plugin AGENTS were re-scanned in lockstep with the agents above.\n",
        );

        ctx.emit(TuiEvent::TextDelta(msg));
        Ok(())
    }

    fn description(&self) -> &str {
        "Re-scan agent registry from disk (skills + WASM plugins deferred)"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }
}

/// Format a before/after count delta as `"+N"`, `"-N"`, or
/// `"unchanged"`. Pulled out for unit-test surface.
fn format_delta(before: usize, after: usize) -> String {
    use std::cmp::Ordering;
    match after.cmp(&before) {
        Ordering::Greater => format!("+{}", after - before),
        Ordering::Less => format!("-{}", before - after),
        Ordering::Equal => "unchanged".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_support::*;

    #[test]
    fn execute_without_registry_returns_err() {
        let handler = RefreshHandler;
        let (mut ctx, _rx) = make_bug_ctx();
        let result = handler.execute(&mut ctx, &[]);
        assert!(result.is_err());
        let msg = format!("{:#}", result.unwrap_err()).to_lowercase();
        assert!(
            msg.contains("agent_registry"),
            "error must reference agent_registry; got: {}",
            msg
        );
    }

    #[test]
    fn execute_without_working_dir_returns_err() {
        // Registry present but working_dir absent → Err with descriptive
        // message. Build a ctx with the agent_registry only.
        let handler = RefreshHandler;
        let registry = std::sync::Arc::new(std::sync::RwLock::new(
            archon_core::agents::AgentRegistry::empty(),
        ));
        let (mut ctx, _rx) = CtxBuilder::new()
            .with_agent_registry(registry)
            .build();
        let result = handler.execute(&mut ctx, &[]);
        assert!(result.is_err());
        let msg = format!("{:#}", result.unwrap_err()).to_lowercase();
        assert!(msg.contains("working_dir"));
    }

    #[test]
    fn execute_with_empty_registry_emits_summary() {
        let handler = RefreshHandler;
        let registry = std::sync::Arc::new(std::sync::RwLock::new(
            archon_core::agents::AgentRegistry::empty(),
        ));
        let tmp = std::env::temp_dir().join("archon_refresh_test_empty");
        let _ = std::fs::create_dir_all(&tmp);
        let (mut ctx, mut rx) = CtxBuilder::new()
            .with_agent_registry(registry)
            .with_working_dir(tmp)
            .build();
        handler.execute(&mut ctx, &[]).unwrap();

        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        let body = match &events[0] {
            TuiEvent::TextDelta(s) => s.clone(),
            other => panic!("expected TextDelta, got {:?}", other),
        };
        assert!(body.contains("Refreshed agent registry"));
        assert!(body.contains("Agents:"));
        // Empty registry + reload from a directory with no agent files
        // should still emit a count, even if it's 0 or includes
        // built-ins.
        assert!(body.contains("loaded"));
    }

    #[test]
    fn execute_emits_skills_and_plugins_deferral_lines() {
        let handler = RefreshHandler;
        let registry = std::sync::Arc::new(std::sync::RwLock::new(
            archon_core::agents::AgentRegistry::empty(),
        ));
        let tmp = std::env::temp_dir().join("archon_refresh_test_deferral");
        let _ = std::fs::create_dir_all(&tmp);
        let (mut ctx, mut rx) = CtxBuilder::new()
            .with_agent_registry(registry)
            .with_working_dir(tmp)
            .build();
        handler.execute(&mut ctx, &[]).unwrap();
        let body = match drain_tui_events(&mut rx).into_iter().next().unwrap() {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        };
        // Both deferral notices must be present so users see WHY skills
        // and WASM plugins did not refresh.
        assert!(body.contains("Skills:"));
        assert!(
            body.contains("session restart required"),
            "skills deferral missing"
        );
        assert!(body.contains("Plugins:"));
        assert!(
            body.contains("/reload-plugins"),
            "WASM plugin deferral pointer missing"
        );
    }

    #[test]
    fn format_delta_branches() {
        assert_eq!(format_delta(5, 5), "unchanged");
        assert_eq!(format_delta(5, 7), "+2");
        assert_eq!(format_delta(5, 3), "-2");
        // Underflow guard: before > after returns negative-prefixed
        // string, never panics.
        assert_eq!(format_delta(100, 0), "-100");
        // Symmetric upper bound.
        assert_eq!(format_delta(0, 100), "+100");
    }

    #[test]
    fn description_and_aliases() {
        let h = RefreshHandler;
        assert!(!h.description().is_empty());
        assert_eq!(h.aliases(), &[] as &[&'static str]);
    }

    #[test]
    #[ignore = "Gate 5 live smoke — exercises Registry dispatch via default_registry(), run via --ignored"]
    fn refresh_dispatches_via_registry() {
        // Gate-5 smoke: Registry::get("refresh") must return Some;
        // executing against an empty AgentRegistry + tempdir
        // working_dir must emit a single TextDelta that includes
        // "Refreshed agent registry" + the skills + plugins deferral
        // lines.
        use crate::command::registry::default_registry;

        let registry = default_registry();
        let handler = registry
            .get("refresh")
            .expect("refresh must be registered in default_registry()");

        let agents = std::sync::Arc::new(std::sync::RwLock::new(
            archon_core::agents::AgentRegistry::empty(),
        ));
        let tmp = std::env::temp_dir().join("archon_refresh_smoke");
        let _ = std::fs::create_dir_all(&tmp);
        let (mut ctx, mut rx) = CtxBuilder::new()
            .with_agent_registry(agents)
            .with_working_dir(tmp)
            .build();
        handler.execute(&mut ctx, &[]).unwrap();

        let body = match drain_tui_events(&mut rx).into_iter().next().unwrap() {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        };
        assert!(body.contains("Refreshed agent registry"));
        assert!(body.contains("Skills:"));
        assert!(body.contains("/reload-plugins"));
    }
}
