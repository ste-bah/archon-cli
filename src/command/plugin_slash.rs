//! TASK-#216 SLASH-PLUGIN — `/plugin` umbrella slash-command handler.
//!
//! Subcommands:
//!   - `/plugin` (no args)         → list (default — alias for `list`)
//!   - `/plugin list`              → render the plugin table
//!     (enabled / disabled / errors) by re-scanning disk via the
//!     shared `load_plugins_from_default_dirs()` helper from
//!     `src/command/plugin.rs`. Mirrors the CLI surface.
//!   - `/plugin info <name>`       → render manifest details for one
//!     plugin (also fresh-scan).
//!   - `/plugin enable <name>`     → emit hint TextDelta —
//!     **runtime-only enable/disable + persistence are deferred**;
//!     see scope reconciliation below.
//!   - `/plugin disable <name>`    → emit hint TextDelta — same.
//!   - `/plugin install <name>`    → emit hint TextDelta pointing the
//!     user at the manual install path (copy WASM + manifest into
//!     `~/.local/share/archon/plugins/<name>/`).
//!   - `/plugin reload`            → hint pointing at the dedicated
//!     `/reload-plugins` primary (TASK-#217).
//!   - `/plugin <other>`           → usage TextDelta.
//!
//! # Spec/reality reconciliation
//!
//! The mission ticket said:
//!
//!   "multi-subcommand plugin manager. The current CLI has only
//!    `archon plugin {list|info}` — these add the in-session versions
//!    + extend with enable/disable/install/hot-reload."
//!
//! Reality on this branch:
//!
//!   1. `archon-plugin` exposes `PluginLoader::with_enabled_state(
//!      HashMap<String, bool>)` (loader.rs:59) for opt-in
//!      bucketing into `PluginLoadResult::{enabled, disabled}` vecs.
//!      But there is **no persistence layer** — no JSON file, no
//!      TOML field, no API for write-back. Mutating enable/disable
//!      state from a slash command would only affect the in-process
//!      bucket on the next `load_all()` call — and even that needs a
//!      session-shared `Arc<RwLock<HashMap<String, bool>>>` field on
//!      `SlashCommandContext` to persist the choice across slash
//!      dispatches.
//!   2. `WasmPluginHost` (host.rs:98) exposes `load_plugin()` and
//!      `dispatch_tool()` but **no `reload_plugin` / `unload` /
//!      `hot_swap` API**. The runtime is private; once instantiated
//!      it cannot be replaced without constructing a fresh host.
//!   3. There is no session-shared plugin manager. The CLI builds a
//!      fresh `PluginLoader` per invocation and discards the result
//!      after rendering. Plugins are never instantiated as a running
//!      WASM host within the slash-dispatch surface — only their
//!      AGENT-side manifests (`.archon/plugins/*/agents/*`) are
//!      loaded, via the agent registry (#211).
//!   4. There is no archon-side `install` command — plugins are
//!      installed by hand-copying directories.
//!
//! Resolution per the mission "Plugin persistence note: if
//! enabling/disabling plugins requires a persistence layer that
//! doesn't exist in `archon-plugin::manifest`, scope-reduce to
//! RUNTIME-ONLY state + file a follow-up for persisted enable/
//! disable" rule:
//!
//!   - Ship `/plugin` with the LIST + INFO subcommands fully
//!     functional (re-scan + render — same data the CLI shows but
//!     via TextDelta inside the TUI).
//!   - Ship enable/disable/install/reload as HINT subcommands that
//!     emit a clear TextDelta describing the deferred-persistence /
//!     deferred-host-API reality and pointing the user at the
//!     canonical workaround. No silent failure; no claim of features
//!     that don't exist.
//!   - Defer the actual mutate-state subcommands to a follow-up
//!     ticket that adds either:
//!       (a) a session-shared `plugin_enable_state: Arc<RwLock<
//!           HashMap<String, bool>>>` field on `SlashCommandContext`
//!           plus a JSON state file at `~/.local/state/archon/
//!           plugin-state.json`, OR
//!       (b) a richer `PluginManager` type in archon-plugin with
//!           in-place enable/disable + reload + persistence APIs.
//!     Both are cross-cutting subsystem work beyond wrapper scope.

use archon_tui::app::TuiEvent;

use crate::command::plugin::load_plugins_from_default_dirs;
use crate::command::registry::{CommandContext, CommandHandler};

/// `/plugin` umbrella handler — list / info / hint subcommands.
pub(crate) struct PluginSlashHandler;

impl CommandHandler for PluginSlashHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        let subcommand = args.first().map(|s| s.as_str()).unwrap_or("list");
        let rest: &[String] = if args.is_empty() { &[] } else { &args[1..] };

        let body = match subcommand {
            "" | "list" => render_list(),
            "info" | "show" => match rest.first() {
                Some(name) => render_info(name),
                None => render_usage("info: missing <name>"),
            },
            "enable" | "disable" => match rest.first() {
                Some(name) => render_enable_disable_hint(subcommand, name),
                None => render_usage(&format!("{subcommand}: missing <name>")),
            },
            "install" => match rest.first() {
                Some(name) => render_install_hint(name),
                None => render_usage("install: missing <name>"),
            },
            "reload" => render_reload_hint(),
            other => render_usage(&format!("unknown subcommand `{}`", other)),
        };

        ctx.emit(TuiEvent::TextDelta(body));
        Ok(())
    }

    fn description(&self) -> &str {
        "Manage WASM plugins (list, info — enable/disable/install/reload deferred)"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }
}

fn render_list() -> String {
    let result = load_plugins_from_default_dirs();
    let mut out = String::with_capacity(2048);
    out.push('\n');
    out.push_str(&format!(
        "/plugin list — {} enabled, {} disabled, {} errors\n",
        result.enabled.len(),
        result.disabled.len(),
        result.errors.len(),
    ));
    out.push_str(&format!(
        "  {:<28}  {:<12}  {}\n",
        "name", "version", "status"
    ));
    out.push_str(&format!(
        "  {}  {}  {}\n",
        "-".repeat(28),
        "-".repeat(12),
        "-".repeat(8),
    ));
    if result.enabled.is_empty() && result.disabled.is_empty() && result.errors.is_empty() {
        out.push_str("  (no plugins discovered)\n");
        out.push_str(
            "\nDrop a plugin under ~/.local/share/archon/plugins/<name>/\n\
             with a manifest.toml and a .wasm artifact, then run\n\
             /reload-plugins to re-scan.\n",
        );
        return out;
    }
    for p in &result.enabled {
        out.push_str(&format!(
            "  {:<28}  {:<12}  enabled\n",
            truncate_chars(&p.manifest.name, 28),
            truncate_chars(&p.manifest.version, 12),
        ));
    }
    for p in &result.disabled {
        out.push_str(&format!(
            "  {:<28}  {:<12}  disabled\n",
            truncate_chars(&p.manifest.name, 28),
            truncate_chars(&p.manifest.version, 12),
        ));
    }
    for (id, err) in &result.errors {
        let err_str = format!("error: {}", err);
        out.push_str(&format!(
            "  {:<28}  {:<12}  {}\n",
            truncate_chars(id, 28),
            "?",
            truncate_chars(&err_str, 32),
        ));
    }
    out.push_str("\nUse `/plugin info <name>` for details.\n");
    out
}

fn render_info(name: &str) -> String {
    let result = load_plugins_from_default_dirs();
    let plugin = result
        .enabled
        .iter()
        .chain(result.disabled.iter())
        .find(|p| p.manifest.name == name);

    match plugin {
        Some(p) => {
            let status = if result
                .disabled
                .iter()
                .any(|d| d.manifest.name == name)
            {
                "disabled"
            } else {
                "enabled"
            };
            let mut out = String::with_capacity(512);
            out.push('\n');
            out.push_str(&format!("Plugin: {}\n", p.manifest.name));
            out.push_str(&format!("  Version:      {}\n", p.manifest.version));
            out.push_str(&format!("  Status:       {}\n", status));
            if let Some(desc) = &p.manifest.description {
                out.push_str(&format!("  Description:  {}\n", desc));
            }
            if let Some(author) = &p.manifest.author {
                out.push_str(&format!("  Author:       {}\n", author));
            }
            if let Some(license) = &p.manifest.license {
                out.push_str(&format!("  License:      {}\n", license));
            }
            if !p.manifest.capabilities.is_empty() {
                out.push_str(&format!(
                    "  Capabilities: {}\n",
                    p.manifest.capabilities.join(", ")
                ));
            }
            if !p.manifest.dependencies.is_empty() {
                out.push_str(&format!(
                    "  Dependencies: {}\n",
                    p.manifest.dependencies.join(", ")
                ));
            }
            out.push_str(&format!("  Data dir:     {}\n", p.data_dir.display()));
            out
        }
        None => {
            // Check the errors bucket so the user sees a useful diagnostic
            // instead of a generic "not found".
            if let Some((_, err)) = result.errors.iter().find(|(id, _)| id == name) {
                format!(
                    "\nPlugin `{name}` failed to load: {err}\n\
                     Run `/plugin list` to see all discovered plugins.\n"
                )
            } else {
                format!(
                    "\nPlugin `{name}` not found. Run `/plugin list` to see all\n\
                     discovered plugins.\n"
                )
            }
        }
    }
}

fn render_enable_disable_hint(verb: &str, name: &str) -> String {
    format!(
        "\n\
         /plugin {verb} `{name}` — DEFERRED\n\
         ─────────────────────────────────\n\
         Runtime-only enable/disable from the slash-command surface is\n\
         deferred. archon-plugin's `PluginLoader::with_enabled_state(\n\
         HashMap<String, bool>)` API exists, but there is no persistence\n\
         layer (no JSON state file, no TOML field) and no session-shared\n\
         `plugin_enable_state` field on `SlashCommandContext` — see\n\
         `src/command/plugin_slash.rs` module rustdoc for the full\n\
         reconciliation.\n\
         \n\
         To {verb} `{name}` today:\n  \
         1. Edit the plugin's manifest at\n     \
            ~/.local/share/archon/plugins/{name}/manifest.toml — but the\n     \
            shipped manifest schema does not yet have an `enabled` field;\n     \
            file a follow-up if you need it persisted.\n  \
         2. Or move/rename the plugin directory out of\n     \
            ~/.local/share/archon/plugins/ to disable it manually, then\n     \
            run /reload-plugins.\n  \
         3. Restart the session for any change to take effect across\n     \
            the WASM host.\n\
         \n\
         The persisted-state follow-up will reduce this to a one-line\n\
         `/plugin {verb} {name}` + auto-write.\n",
        verb = verb,
        name = name
    )
}

fn render_install_hint(name: &str) -> String {
    format!(
        "\n\
         /plugin install `{name}` — DEFERRED\n\
         ───────────────────────────────────\n\
         There is no archon-side install command. To install `{name}`:\n\
         \n  \
         1. Obtain the plugin (a directory containing manifest.toml + a\n     \
            .wasm artifact + any data files).\n  \
         2. Copy it into ~/.local/share/archon/plugins/{name}/ (or set\n     \
            ARCHON_PLUGIN_SEED_DIR=<dir> to a directory containing a\n     \
            `{name}/` subdirectory and re-launch the session).\n  \
         3. Run `/reload-plugins` (or `/plugin list`) to verify the\n     \
            scanner picked it up.\n\
         \n\
         A future ticket may add network/registry-driven install — for\n\
         now this is the manual surface.\n",
        name = name
    )
}

fn render_reload_hint() -> String {
    "\n\
     /plugin reload is a pointer to the dedicated `/reload-plugins`\n\
     primary (TASK-#217). Use:\n  \
     \n  \
     /reload-plugins\n\
     \n\
     The dedicated command re-scans the plugin directories and reports\n\
     a count delta of enabled / disabled / errored plugins on disk.\n"
        .to_string()
}

fn render_usage(prefix: &str) -> String {
    format!(
        "\n\
         /plugin — manage WASM plugins.\n\
         Note: {prefix}\n\
         \n\
         Subcommands:\n  \
         list                 List all discovered plugins (default).\n  \
         info <name>          Show plugin manifest details.\n  \
         enable <name>        Hint — runtime-only enable is deferred.\n  \
         disable <name>       Hint — runtime-only disable is deferred.\n  \
         install <name>       Hint — manual install only.\n  \
         reload               Hint — pointer to /reload-plugins.\n\
         \n\
         Run `/plugin list` to start.\n",
        prefix = prefix,
    )
}

/// Truncate to at most `max` Unicode characters, appending `…` when
/// shortened. Char-aware — never panics on multi-byte input.
fn truncate_chars(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    let take = max.saturating_sub(1);
    let mut out: String = s.chars().take(take).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_support::*;

    fn execute_subcommand(args: &[&str]) -> String {
        let handler = PluginSlashHandler;
        let (mut ctx, mut rx) = make_bug_ctx();
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        handler.execute(&mut ctx, &owned).unwrap();
        let evs = drain_tui_events(&mut rx);
        assert_eq!(evs.len(), 1);
        match evs.into_iter().next().unwrap() {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn no_args_renders_list_header() {
        // We can't predict whether plugins exist on the test host's
        // ~/.local/share/archon/plugins/, so just assert the LIST
        // header / no-plugins fallback / total counts line is present.
        let body = execute_subcommand(&[]);
        assert!(body.contains("/plugin list"));
    }

    #[test]
    fn list_keyword_path_matches_default() {
        let body = execute_subcommand(&["list"]);
        assert!(body.contains("/plugin list"));
    }

    #[test]
    fn info_unknown_plugin_returns_friendly_message() {
        let body = execute_subcommand(&["info", "no-such-plugin-fixture"]);
        assert!(body.contains("`no-such-plugin-fixture`"));
        assert!(
            body.contains("not found") || body.contains("failed to load"),
            "expected not-found or failed-to-load message; got:\n{}",
            body
        );
    }

    #[test]
    fn info_without_name_emits_usage() {
        let body = execute_subcommand(&["info"]);
        assert!(body.contains("missing <name>"));
        assert!(body.contains("Subcommands:"));
    }

    #[test]
    fn enable_subcommand_emits_deferral_hint() {
        let body = execute_subcommand(&["enable", "myplugin"]);
        assert!(body.contains("/plugin enable `myplugin`"));
        assert!(body.contains("DEFERRED"));
        assert!(body.contains("manifest.toml"));
        assert!(body.contains("/reload-plugins"));
    }

    #[test]
    fn disable_subcommand_emits_deferral_hint() {
        let body = execute_subcommand(&["disable", "myplugin"]);
        assert!(body.contains("/plugin disable `myplugin`"));
        assert!(body.contains("DEFERRED"));
    }

    #[test]
    fn install_subcommand_emits_manual_install_hint() {
        let body = execute_subcommand(&["install", "myplugin"]);
        assert!(body.contains("/plugin install `myplugin`"));
        assert!(body.contains("manifest.toml"));
        assert!(body.contains(".wasm"));
        assert!(body.contains("/reload-plugins"));
    }

    #[test]
    fn reload_subcommand_points_to_reload_plugins() {
        let body = execute_subcommand(&["reload"]);
        assert!(body.contains("/reload-plugins"));
        assert!(body.contains("dedicated"));
    }

    #[test]
    fn unknown_subcommand_emits_usage() {
        let body = execute_subcommand(&["frobnicate"]);
        assert!(body.contains("unknown subcommand `frobnicate`"));
        assert!(body.contains("Subcommands:"));
    }

    #[test]
    fn enable_without_name_emits_usage() {
        let body = execute_subcommand(&["enable"]);
        assert!(body.contains("enable: missing <name>"));
    }

    #[test]
    fn install_without_name_emits_usage() {
        let body = execute_subcommand(&["install"]);
        assert!(body.contains("install: missing <name>"));
    }

    #[test]
    fn truncate_chars_unicode_safe() {
        let truncated = truncate_chars("αβγδεζηθικ", 5);
        assert_eq!(truncated.chars().count(), 5);
        assert!(truncated.ends_with('…'));
    }

    #[test]
    fn description_and_aliases() {
        let h = PluginSlashHandler;
        assert!(!h.description().is_empty());
        assert_eq!(h.aliases(), &[] as &[&'static str]);
    }

    #[test]
    #[ignore = "Gate 5 live smoke — exercises Registry dispatch via default_registry(), run via --ignored"]
    fn plugin_dispatches_via_registry() {
        // Gate-5 smoke: Registry::get("plugin") must return Some;
        // executing with arg "info" without a name emits the usage
        // TextDelta. With "list" emits the list header.
        use crate::command::registry::default_registry;

        let registry = default_registry();
        let handler = registry
            .get("plugin")
            .expect("plugin must be registered in default_registry()");

        let (mut ctx, mut rx) = make_bug_ctx();
        handler.execute(&mut ctx, &[String::from("list")]).unwrap();
        let body = match drain_tui_events(&mut rx).into_iter().next().unwrap() {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        };
        assert!(body.contains("/plugin list"));
    }
}
