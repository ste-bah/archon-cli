//! TASK-#217 SLASH-RELOAD-PLUGINS — `/reload-plugins` slash-command
//! handler.
//!
//! Re-scans the plugin directories
//! (`~/.local/share/archon/plugins/`, plus any
//! `ARCHON_PLUGIN_SEED_DIR` entries) by calling the shared
//! `load_plugins_from_default_dirs()` helper from
//! `src/command/plugin.rs`, and emits a single `TuiEvent::TextDelta`
//! summarising the result (counts of enabled / disabled / errored
//! plugins).
//!
//! # Spec/reality reconciliation
//!
//! The mission ticket said "hot-reload without restart". Reality:
//!
//!   1. `WasmPluginHost` (host.rs:98) exposes only `new()`,
//!      `load_plugin()`, and `dispatch_tool()`. There is no
//!      `reload_plugin` / `unload_plugin` / `hot_swap` API. Once a
//!      WASM module is instantiated inside the host's private
//!      `runtime: Option<WasmRuntime>`, it cannot be replaced.
//!   2. There is no session-shared plugin host. Plugins are only
//!      ever loaded by the CLI (`archon plugin list`) for inspection
//!      via a fresh `PluginLoader::load_all()` call; they are never
//!      instantiated as a running WASM host inside the slash-dispatch
//!      surface. (Plugin AGENTS — `.archon/plugins/*/agents/*.md` —
//!      ARE loaded by the agent registry; `/refresh` re-scans those.
//!      Plugin TOOLS / WASM artifacts are not.)
//!   3. A true hot-swap of a running WASM module would require
//!      either (a) extending `WasmPluginHost` with `reload_plugin`
//!      that drops the runtime and re-instantiates from disk
//!      (subsystem refactor), or (b) wiring up a session-shared
//!      `Arc<RwLock<WasmPluginHost>>` so the slash handler can
//!      actually swap modules in a running host (cross-cutting —
//!      requires plumbing through SlashCommandContext + a session-
//!      bootstrap step).
//!
//! Resolution per the mission "Plugin persistence note: ... scope-
//! reduce to RUNTIME-ONLY state + file a follow-up" rule:
//!
//!   - Ship `/reload-plugins` as a DISK RE-SCAN command. It calls
//!     the shared loader helper (re-reads every manifest, re-checks
//!     every `.wasm` artifact, re-runs cache validation) and reports
//!     the new counts. This is the same data the user sees when they
//!     run `/plugin list` immediately after dropping a new plugin
//!     directory — but presented as a single-step "did the scanner
//!     pick up my new plugin" check.
//!   - True hot-swap of running WASM modules is deferred to a
//!     follow-up that resolves either path (a) or (b) above. The
//!     module rustdoc + the rendered output both flag this so users
//!     see the deferral, not a missing feature.

use archon_tui::app::TuiEvent;

use crate::command::plugin::load_plugins_from_default_dirs;
use crate::command::registry::{CommandContext, CommandHandler};

/// `/reload-plugins` handler — re-scan plugin directories from disk.
pub(crate) struct ReloadPluginsHandler;

impl CommandHandler for ReloadPluginsHandler {
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        let result = load_plugins_from_default_dirs();
        let body = render_summary(
            result.enabled.len(),
            result.disabled.len(),
            result.errors.len(),
            &result.errors,
        );
        ctx.emit(TuiEvent::TextDelta(body));
        Ok(())
    }

    fn description(&self) -> &str {
        "Re-scan plugin directories from disk (true hot-swap deferred)"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }
}

/// Render the result summary. Pulled out so unit tests can exercise
/// the rendering branches without exposing on-disk state.
fn render_summary(
    enabled: usize,
    disabled: usize,
    errors: usize,
    error_details: &[(String, archon_plugin::PluginError)],
) -> String {
    let total = enabled + disabled + errors;
    let mut out = String::with_capacity(512);
    out.push('\n');
    out.push_str("Re-scanned plugin directories.\n");
    out.push_str(&format!("  Total found: {total}\n"));
    out.push_str(&format!("  Enabled:     {enabled}\n"));
    out.push_str(&format!("  Disabled:    {disabled}\n"));
    out.push_str(&format!("  Errors:      {errors}\n"));

    if errors > 0 {
        out.push_str("\nError details:\n");
        for (id, err) in error_details {
            out.push_str(&format!("  - {id}: {err}\n"));
        }
    }

    out.push_str(
        "\nNote: this command re-scans the plugin DIRECTORIES (manifest +\n\
         .wasm artifacts on disk). True in-process hot-swap of a running\n\
         WASM module is deferred — there is no session-shared\n\
         WasmPluginHost in this branch and no `reload_plugin` API on the\n\
         host. Use `/plugin list` to see the current scan results, or\n\
         restart the session to instantiate any newly-discovered\n\
         plugins.\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_support::*;

    fn run_handler() -> String {
        let handler = ReloadPluginsHandler;
        let (mut ctx, mut rx) = make_bug_ctx();
        handler.execute(&mut ctx, &[]).unwrap();
        let evs = drain_tui_events(&mut rx);
        assert_eq!(evs.len(), 1);
        match evs.into_iter().next().unwrap() {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn execute_emits_summary_line() {
        // Cannot predict on-disk plugin count, so just verify the
        // canonical summary header + count labels are present.
        let body = run_handler();
        assert!(body.contains("Re-scanned plugin directories."));
        assert!(body.contains("Total found:"));
        assert!(body.contains("Enabled:"));
        assert!(body.contains("Disabled:"));
        assert!(body.contains("Errors:"));
    }

    #[test]
    fn execute_emits_hot_swap_deferral_note() {
        let body = run_handler();
        // The body MUST surface the hot-swap deferral so users see why
        // a "reload" doesn't actually swap a running WASM module.
        assert!(body.contains("hot-swap"));
        assert!(body.contains("deferred"));
        assert!(body.contains("/plugin list"));
    }

    #[test]
    fn render_summary_zero_plugins() {
        let body = render_summary(0, 0, 0, &[]);
        assert!(body.contains("Total found: 0"));
        assert!(body.contains("Enabled:     0"));
        assert!(body.contains("Disabled:    0"));
        assert!(body.contains("Errors:      0"));
        // No "Error details:" section when errors == 0.
        assert!(!body.contains("Error details:"));
    }

    #[test]
    fn render_summary_with_counts_and_errors() {
        // 2 enabled + 1 disabled + 1 errored = 4 total. Error details
        // section should render the error id + Display-formatted
        // message. Use the real `LoadFailed` variant from
        // archon_plugin::PluginError (single-string Display target;
        // matches the `#[error("plugin load failed: {0}")]` attr).
        let errors = vec![(
            "broken-plugin".to_string(),
            archon_plugin::PluginError::LoadFailed("missing required field: name".to_string()),
        )];
        let body = render_summary(2, 1, 1, &errors);
        assert!(body.contains("Total found: 4"));
        assert!(body.contains("Enabled:     2"));
        assert!(body.contains("Disabled:    1"));
        assert!(body.contains("Errors:      1"));
        assert!(body.contains("Error details:"));
        assert!(body.contains("broken-plugin"));
        assert!(body.contains("missing required field: name"));
    }

    #[test]
    fn render_summary_no_error_details_when_zero_errors() {
        let body = render_summary(5, 2, 0, &[]);
        assert!(body.contains("Total found: 7"));
        assert!(!body.contains("Error details:"));
    }

    #[test]
    fn description_and_aliases() {
        let h = ReloadPluginsHandler;
        assert!(!h.description().is_empty());
        assert_eq!(h.aliases(), &[] as &[&'static str]);
    }

    #[test]
    #[ignore = "Gate 5 live smoke — exercises Registry dispatch via default_registry(), run via --ignored"]
    fn reload_plugins_dispatches_via_registry() {
        // Gate-5 smoke: Registry::get("reload-plugins") must return
        // Some; executing must emit a single TextDelta containing the
        // canonical summary header + hot-swap deferral note.
        use crate::command::registry::default_registry;

        let registry = default_registry();
        let handler = registry
            .get("reload-plugins")
            .expect("reload-plugins must be registered in default_registry()");
        let (mut ctx, mut rx) = make_bug_ctx();
        handler.execute(&mut ctx, &[]).unwrap();
        let body = match drain_tui_events(&mut rx).into_iter().next().unwrap() {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        };
        assert!(body.contains("Re-scanned plugin directories"));
        assert!(body.contains("hot-swap"));
        assert!(body.contains("/plugin list"));
    }
}
