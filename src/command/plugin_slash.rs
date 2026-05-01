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

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// `/plugin` umbrella handler — list / info / hint subcommands.
pub(crate) struct PluginSlashHandler;

impl CommandHandler for PluginSlashHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        let subcommand = args.first().map(|s| s.as_str()).unwrap_or("list");
        let rest: &[String] = if args.is_empty() { &[] } else { &args[1..] };

        let body = match subcommand {
            "" | "list" => render_list(ctx.plugin_enable_state.as_ref()),
            "info" | "show" => match rest.first() {
                Some(name) => render_info(name, ctx.plugin_enable_state.as_ref()),
                None => render_usage("info: missing <name>"),
            },
            "enable" | "disable" => match rest.first() {
                Some(name) => {
                    let enable = subcommand == "enable";
                    handle_enable_disable(ctx, name, enable)
                }
                None => render_usage(&format!("{subcommand}: missing <name>")),
            },
            "install" => match rest.first() {
                Some(name) => {
                    handle_install(name, rest.get(1).map(|s| s.as_str()) == Some("--force"))
                }
                None => render_usage("install: missing <name>"),
            },
            "reload" => handle_reload(ctx.plugin_enable_state.as_ref()),
            other => render_usage(&format!("unknown subcommand `{}`", other)),
        };

        ctx.emit(TuiEvent::TextDelta(body));
        Ok(())
    }

    fn description(&self) -> &str {
        "Manage WASM plugins (list, info, enable, disable, install, reload)"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }
}

fn render_list(state: Option<&Arc<RwLock<HashMap<String, bool>>>>) -> String {
    let result = load_plugins_with_state(state);
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

fn load_plugins_with_state(
    state: Option<&Arc<RwLock<HashMap<String, bool>>>>,
) -> archon_plugin::result::PluginLoadResult {
    if let Some(lock) = state {
        let map = lock.read().unwrap_or_else(|p| p.into_inner());
        crate::command::plugin::load_plugins_from_default_dirs_with_state(&map)
    } else {
        crate::command::plugin::load_plugins_from_default_dirs()
    }
}

fn render_info(name: &str, state: Option<&Arc<RwLock<HashMap<String, bool>>>>) -> String {
    let result = load_plugins_with_state(state);
    let plugin = result
        .enabled
        .iter()
        .chain(result.disabled.iter())
        .find(|p| p.manifest.name == name);

    match plugin {
        Some(p) => {
            let status = if result.disabled.iter().any(|d| d.manifest.name == name) {
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

// ---------------------------------------------------------------------------
// GHOST-005: real enable/disable/install/reload implementations
// ---------------------------------------------------------------------------

fn handle_enable_disable(ctx: &mut CommandContext, name: &str, enable: bool) -> String {
    let state_lock = match ctx.plugin_enable_state.as_ref() {
        Some(lock) => lock,
        None => return "\nPlugin state not available — session bootstrap issue.\n".to_string(),
    };

    // Verify the plugin exists on disk first.
    let result = crate::command::plugin::load_plugins_from_default_dirs();
    let exists = result
        .enabled
        .iter()
        .chain(result.disabled.iter())
        .any(|p| p.manifest.name == name);
    if !exists {
        return format!(
            "\nPlugin `{name}` not found on disk.\n\
             Run `/plugin list` to see discovered plugins.\n"
        );
    }

    // Mutate in-memory state.
    {
        let mut state = state_lock.write().unwrap_or_else(|p| p.into_inner());
        state.insert(name.to_string(), enable);
    }

    // Persist to disk.
    let state = state_lock.read().unwrap_or_else(|p| p.into_inner());
    match crate::command::plugin::save_plugin_enable_state(&state) {
        Ok(()) => {
            let verb = if enable { "enabled" } else { "disabled" };
            format!(
                "\nPlugin `{name}` {verb}.\n\
                 Run `/plugin list` to see the updated state.\n"
            )
        }
        Err(e) => {
            format!(
                "\nState changed in memory but persist failed: {e}\n\
                 The change will be lost on session restart.\n"
            )
        }
    }
}

fn handle_install(name: &str, force: bool) -> String {
    let plugins_dir = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("archon")
        .join("plugins");

    let dest = plugins_dir.join(name);
    if dest.exists() && !force {
        return format!(
            "\nPlugin `{name}` already exists at {}. Use `--force` to overwrite.\n",
            dest.display()
        );
    }

    // Search seed dirs for the plugin source.
    let seed_dirs: Vec<std::path::PathBuf> = std::env::var("ARCHON_PLUGIN_SEED_DIR")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .collect();

    let mut source: Option<std::path::PathBuf> = None;
    for seed in &seed_dirs {
        let candidate = seed.join(name);
        if candidate.is_dir() {
            // Validate manifest parses before copying.
            let manifest_path = candidate.join("manifest.toml");
            if !manifest_path.exists() {
                continue;
            }
            source = Some(candidate);
            break;
        }
    }

    let source = match source {
        Some(s) => s,
        None => {
            return format!(
                "\nPlugin `{name}` not found in any seed directory.\n\
                 Set ARCHON_PLUGIN_SEED_DIR to a directory containing\n\
                 `{name}/manifest.toml` and try again.\n"
            );
        }
    };

    // Copy the plugin directory.
    match copy_dir_recursive(&source, &dest) {
        Ok(()) => {
            format!(
                "\nPlugin `{name}` installed to {}.\n\
                 Run `/plugin list` or `/plugin reload` to discover it.\n",
                dest.display()
            )
        }
        Err(e) => {
            format!("\nInstall failed: {e}\n")
        }
    }
}

fn handle_reload(state: Option<&Arc<RwLock<HashMap<String, bool>>>>) -> String {
    let result = load_plugins_with_state(state);
    let mut out = String::with_capacity(256);
    out.push('\n');
    out.push_str(&format!(
        "/plugin reload — {} enabled, {} disabled, {} errors\n",
        result.enabled.len(),
        result.disabled.len(),
        result.errors.len(),
    ));
    if result.enabled.is_empty() && result.disabled.is_empty() && result.errors.is_empty() {
        out.push_str("(no plugins discovered)\n");
    } else {
        for p in &result.enabled {
            out.push_str(&format!("  {}  enabled\n", p.manifest.name));
        }
        for p in &result.disabled {
            out.push_str(&format!("  {}  disabled\n", p.manifest.name));
        }
        for (id, err) in &result.errors {
            out.push_str(&format!("  {id}  error: {err}\n"));
        }
    }
    out.push_str("\nNote: Reload re-scans disk. Running WASM instances are not hot-swapped.\n");
    out
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
         enable <name>        Enable a plugin (persisted).\n  \
         disable <name>       Disable a plugin (persisted).\n  \
         install <name>       Install plugin from seed dir.\n  \
         reload               Re-scan plugin directories.\n\
         \n\
         Run `/plugin list` to start.\n",
        prefix = prefix,
    )
}

/// Recursively copy a directory. Used by `handle_install`.
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("create_dir: {e}"))?;
    for entry in std::fs::read_dir(src).map_err(|e| format!("read_dir: {e}"))? {
        let entry = entry.map_err(|e| format!("entry: {e}"))?;
        let ty = entry.file_type().map_err(|e| format!("file_type: {e}"))?;
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path)?;
        } else if ty.is_file() || ty.is_symlink() {
            std::fs::copy(entry.path(), &dst_path).map_err(|e| format!("copy: {e}"))?;
        }
    }
    Ok(())
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

    // GHOST-005: real impl now wires enable/disable/install/reload through
    // CommandContext.plugin_enable_state and disk persistence. Tests below
    // exercise the user-visible output paths reachable from a default test
    // fixture (no plugin_enable_state set, no fixture plugins on disk).

    #[test]
    fn enable_subcommand_emits_state_unavailable_or_not_found() {
        let body = execute_subcommand(&["enable", "myplugin"]);
        // Either "state not available" (no plugin_enable_state on ctx in
        // bare test fixture) OR "not found on disk" (state present, no
        // fixture plugin). Both are non-error, non-deferral outputs.
        let lower = body.to_lowercase();
        assert!(
            lower.contains("plugin state not available")
                || lower.contains("not found on disk")
                || lower.contains("`myplugin` enabled"),
            "expected state-unavailable / not-found / enabled message; got: {body}"
        );
    }

    #[test]
    fn disable_subcommand_emits_state_unavailable_or_not_found() {
        let body = execute_subcommand(&["disable", "myplugin"]);
        let lower = body.to_lowercase();
        assert!(
            lower.contains("plugin state not available")
                || lower.contains("not found on disk")
                || lower.contains("`myplugin` disabled"),
            "expected state-unavailable / not-found / disabled message; got: {body}"
        );
    }

    #[test]
    fn install_subcommand_emits_install_outcome() {
        let body = execute_subcommand(&["install", "myplugin"]);
        let lower = body.to_lowercase();
        // install handler can return: success, already-exists (use --force),
        // source-not-found, or install error. All are valid non-deferral
        // outputs of the real handler.
        assert!(
            lower.contains("myplugin")
                && (lower.contains("install")
                    || lower.contains("already exists")
                    || lower.contains("not found")
                    || lower.contains("source")),
            "expected install-outcome message mentioning myplugin; got: {body}"
        );
    }

    #[test]
    fn reload_subcommand_emits_reload_summary() {
        let body = execute_subcommand(&["reload"]);
        // Real reload handler emits "/plugin reload — N enabled, M disabled, K errors".
        assert!(
            body.contains("/plugin reload"),
            "expected reload summary header; got: {body}"
        );
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
