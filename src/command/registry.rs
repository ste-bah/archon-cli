//! Slash command registry.
//!
//! TASK-AGS-622: typed command table. Replaces the implicit mapping
//! embedded in `handle_slash_command`'s monolithic `match` block with
//! an explicit `HashMap<&'static str, Arc<dyn CommandHandler>>`.
//!
//! This module establishes the structural shape only. Handler bodies
//! are intentional no-op stubs returning `Ok(())`; TASK-AGS-624 (or a
//! Phase 8 follow-up) migrates the real per-command logic out of
//! `main.rs::handle_slash_command`. Keeping the shape here lets Phase 8
//! register new commands by adding entries instead of editing `main.rs`.
//!
//! Declared `pub(crate)` from `src/command/mod.rs` so visibility is
//! scoped to the bin crate (the `archon-cli` library target does not
//! see this module).

use std::collections::HashMap;
use std::sync::Arc;

use archon_tui::app::TuiEvent;

// TASK-AGS-806: real /tasks handler lives in `crate::command::task` so the
// body-migrate keeps registry.rs declarative (just struct registrations).
// Imported here so `b.insert_primary("tasks", Arc::new(TasksHandler))`
// resolves to the real impl, not the prior `declare_handler!` stub.
use crate::command::task::TasksHandler;
// TASK-AGS-807: real /status handler lives in `crate::command::status`.
// Imported here so `b.insert_primary("status", Arc::new(StatusHandler))`
// resolves to the real impl (snapshot-consuming) instead of the prior
// `declare_handler!` stub. Alias migrates from [stat] to [info] per spec.
use crate::command::status::StatusHandler;
// TASK-AGS-808: real /model handler lives in `crate::command::model`.
// Imported here so `b.insert_primary("model", Arc::new(ModelHandler))`
// resolves to the real impl (snapshot-consuming READ + effect-slot WRITE)
// instead of the prior `declare_handler!` stub. Aliases migrate from
// [models] to [m, switch-model] per spec.
use crate::command::model::ModelHandler;
// TASK-AGS-809: real /cost handler lives in `crate::command::cost`.
// Imported here so `b.insert_primary("cost", Arc::new(CostHandler))`
// resolves to the real impl (snapshot-consuming READ, no WRITE side)
// instead of the prior `declare_handler!` stub. Aliases migrate from
// [] to [usage, billing] per spec REQ-FOR-D7 validation criterion 2.
use crate::command::cost::{CostHandler, CostSnapshot};
// TASK-AGS-810: real /resume handler lives in `crate::command::resume`.
// DIRECT-pattern body-migrate (sync archon_session API — no snapshot,
// no effect slot required). Aliases extended from [continue] to
// [continue, open-session] per spec REQ-FOR-D7 validation criterion 4.
use crate::command::resume::ResumeHandler;
// TASK-AGS-811: real /mcp handler lives in `crate::command::mcp`.
// SNAPSHOT-ONLY body-migrate (async McpServerManager reads move to the
// builder; no effect slot required — /mcp is read-only). Shipped stub
// at registry.rs:467 is REPLACED by this import + the insert_primary
// call below. No aliases — spec lists none and shipped stub had none.
use crate::command::mcp::{McpHandler, McpSnapshot};
// TASK-AGS-812: NEW /hooks primary (Q4=A gap-fix — `/hooks` did not
// exist in shipped slash.rs or registry.rs pre-AGS-812). DIRECT-pattern
// handler — sync `HookRegistry::load_all` + new `summaries()` accessor,
// no snapshot/effect-slot needed. No aliases (spec lists none). Primary
// count grows from 38 -> 39.
use crate::command::hooks::HooksHandler;
// TASK-AGS-815: real /fork handler lives in `crate::command::fork`.
// DIRECT-pattern body-migrate (sync `archon_session::fork::fork_session`
// + `SessionStore::open`; no snapshot/effect-slot needed — session_id
// threads through a new `CommandContext::session_id` field populated
// unconditionally by `build_command_context`). Shipped stub
// `declare_handler!(ForkHandler, ...)` at registry.rs:524 is REPLACED
// by this import + the insert_primary call below. No aliases — shipped
// stub had none and spec lists none.
use crate::command::fork::ForkHandler;
// TASK-AGS-814: real /context handler lives in `crate::command::context_cmd`.
// SNAPSHOT-ONLY body-migrate (single `session_stats.lock().await` moves
// to the builder; no effect slot required — /context is read-only).
// Shipped `declare_handler!(ContextHandler, ...)` stub at registry.rs:447
// is REPLACED by this import + the insert_primary call below. Aliases
// drop from shipped stub's `["ctx"]` to `[]` because the legacy match
// arm in slash.rs only matched `/context` literally — `/ctx` never
// worked. File name is `context_cmd.rs` not `context.rs` to avoid a
// collision with the existing `crate::command::context` builder module.
use crate::command::context_cmd::{ContextHandler, ContextSnapshot};
// TASK-AGS-816: NEW /voice primary (Q4=A gap-fix — `/voice` did not
// exist in shipped slash.rs or registry.rs pre-AGS-816). DIRECT-pattern
// handler — sync `archon_core::config::load_config`, no
// snapshot/effect-slot needed. No aliases (spec lists none). Primary
// count grows from 39 -> 40 (SECOND Batch-3 NEW-primary after AGS-812
// /hooks, which took the count from 38 -> 39).
use crate::command::voice::VoiceHandler;

/// Execution context threaded through every command handler.
///
/// Kept deliberately minimal for TASK-AGS-622: the registry's job is
/// shape, not plumbing. TASK-AGS-623 (dispatcher) grows this struct to
/// carry the real `SlashCommandContext` fields (fast mode, effort,
/// memory, config, etc.) once handlers are migrated off `main.rs`.
///
/// # TASK-AGS-822: Extension Pattern for Body-Migrate Tickets
///
/// Body-migrate tickets AGS-806..819 grow `CommandContext` **one field
/// at a time**, each field gated on exactly the deps the migrating
/// handler body needs. The rules each body-migrate follows:
///
/// 1. **Field visibility**: every new field is `pub(crate)` so handlers
///    in this crate can read/write it without adding trait indirection.
/// 2. **Shared services → `Arc<_>`**: `task_service`, `memory`,
///    `config`, `cost_tracker`, and similar long-lived services ship
///    as `Arc<dyn Trait>` (or `Arc<ConcreteType>`) so cheap clone-out
///    works at call time and handler bodies never hold a borrow on the
///    context longer than their `execute` call.
/// 3. **Construction site**: the field is populated in `src/session.rs`
///    inside the block that currently constructs `SlashCommandContext`
///    (and the shared `Registry` / `Dispatcher` immediately above it,
///    around session.rs:1817-1827). Body-migrates add the field to
///    that block in the same commit that adds the field here.
/// 4. **Threading contract**: `CommandContext` is passed `&mut` to
///    `CommandHandler::execute`, so fields are borrowed mutably by
///    handlers that mutate app state (e.g. picker selection) or
///    cloned-out of the Arc when the handler only needs a read view.
/// 5. **No unused deps**: AGS-822 deliberately leaves the struct at
///    exactly one field (`tui_tx`). This proves the extension pattern
///    compiles end-to-end WITHOUT committing to any specific
///    body-migrate's dep set. First body-migrate that actually needs
///    `task_service` is the first ticket that adds it.
///
/// Pattern reference (how the next field will slot in):
/// ```ignore
/// // e.g. pub(crate) task_service: Arc<dyn TaskService>,
/// // e.g. pub(crate) memory:       Arc<Memory>,
/// // e.g. pub(crate) config:       Arc<ArchonConfig>,
/// ```
///
/// The extension is additive — new fields append to the struct and to
/// the session.rs construction block in lockstep.
pub(crate) struct CommandContext {
    // TASK-AGS-822 extension-pattern reference (commented out — no
    // field added by THAT ticket; see body-migrate AGS-806..819):
    //   pub(crate) task_service: Arc<dyn TaskService>,
    /// TUI event sink for text deltas, errors, and state change
    /// notifications.
    pub(crate) tui_tx: tokio::sync::mpsc::Sender<TuiEvent>,
    /// TASK-AGS-807 snapshot-pattern field.
    ///
    /// Populated by `build_command_context` for `/status` (and its
    /// alias `/info`) ONLY. Every other command observes `None` and
    /// pays zero additional lock traffic. The sync
    /// [`CommandHandler::execute`] cannot await, so the dispatch site
    /// acquires the four `/status` async locks in advance and passes
    /// the owned values via this field.
    pub(crate) status_snapshot: Option<crate::command::status::StatusSnapshot>,
    /// TASK-AGS-808 snapshot-pattern field (READ side of /model).
    ///
    /// Populated by `build_command_context` for `/model` (and its
    /// aliases `/m`, `/switch-model`) ONLY. Every other command
    /// observes `None` and pays zero additional lock traffic. Mirrors
    /// the AGS-807 `status_snapshot` convention (separate typed struct
    /// per ticket; fields differ across handlers so snapshots are not
    /// cross-reused).
    pub(crate) model_snapshot: Option<crate::command::model::ModelSnapshot>,
    /// TASK-AGS-809 snapshot-pattern field (READ-only /cost).
    ///
    /// Populated by `build_command_context` for `/cost` (and its
    /// aliases `/usage`, `/billing`) ONLY. Every other command observes
    /// `None` and pays zero additional lock traffic. Per the AGS-822
    /// Rule 5 extension pattern: each body-migrate ticket appends one
    /// typed snapshot field — /cost is READ-only so there is NO
    /// matching `CommandEffect` variant.
    pub(crate) cost_snapshot: Option<CostSnapshot>,
    /// TASK-AGS-811 snapshot-pattern field (READ-only /mcp).
    ///
    /// Populated by `build_command_context` for `/mcp` ONLY (no
    /// aliases). Every other command observes `None` and pays zero
    /// additional lock traffic on `McpServerManager`. Per the AGS-822
    /// Rule 5 extension pattern: each body-migrate ticket appends one
    /// typed snapshot field — /mcp is READ-only so there is NO
    /// matching `CommandEffect` variant. Subcommands `connect` /
    /// `disconnect` / `reload` listed in the AGS-811 spec are
    /// SCOPE-HELD (shipped wins drift-reconcile); the first ticket
    /// that actually needs them adds the write-side field at that
    /// point.
    pub(crate) mcp_snapshot: Option<McpSnapshot>,
    /// TASK-AGS-814 snapshot-pattern field (READ-only /context).
    ///
    /// Populated by `build_command_context` for `/context` ONLY (no
    /// aliases — the shipped stub's `ctx` alias was cosmetic; see
    /// `context_cmd.rs` module rustdoc). Every other command observes
    /// `None` and pays zero additional lock traffic on `session_stats`.
    /// Per the AGS-822 Rule 5 extension pattern: each body-migrate
    /// ticket appends one typed snapshot field — /context is READ-only
    /// so there is NO matching `CommandEffect` variant.
    pub(crate) context_snapshot: Option<ContextSnapshot>,
    /// TASK-AGS-815 DIRECT-pattern field (/fork).
    ///
    /// Clone of `SlashCommandContext::session_id` populated
    /// UNCONDITIONALLY by `build_command_context` (not per-command —
    /// session_id is always meaningful and cheap to clone). `/fork`
    /// reads it to pass `source_id` to
    /// `archon_session::fork::fork_session`. `Option<String>` so the
    /// dispatcher/handler test fixtures can construct a
    /// `CommandContext` without standing up a full
    /// `SlashCommandContext` — in those tests the field observes
    /// `None` and the handler returns an Err-with-message describing
    /// the missing-session-id condition rather than panicking.
    /// No matching `CommandEffect` variant — `/fork` is a pure
    /// DIRECT-pattern sync body (no async mutex writes back to
    /// shared state).
    pub(crate) session_id: Option<String>,
    /// TASK-AGS-808 effect-slot field (WRITE side of /model and future
    /// write-tickets).
    ///
    /// The Rule-5 extension pattern: sync `CommandHandler::execute`
    /// cannot await mutex writes on shared state. Instead, a handler
    /// synchronously stashes a [`CommandEffect`] variant here; the
    /// dispatch site in `slash.rs::handle_slash_command` takes the
    /// value (consuming the slot via `.take()`) and awaits the write
    /// in `command::context::apply_effect` AFTER dispatch returns. The
    /// single-shot `Option` guarantees exactly-once application even
    /// if the dispatcher were to re-fire on the same context.
    pub(crate) pending_effect: Option<CommandEffect>,
}

/// Side-effect descriptors produced synchronously by
/// [`CommandHandler::execute`] and applied asynchronously after
/// dispatch returns.
///
/// TASK-AGS-808 introduces this enum to bridge the sync-handler /
/// async-shared-state gap for the `/model` write path. The shipped
/// body performed `*slash_ctx.model_override_shared.lock().await = ...`
/// inline, which is forbidden inside a sync trait method. Handlers now
/// stash the intended mutation as an enum variant in
/// `CommandContext::pending_effect`; `slash.rs` post-dispatch takes the
/// value and calls `command::context::apply_effect`, which awaits the
/// correct mutex write.
///
/// Future body-migrate tickets (AGS-809 /cost read-only, AGS-814
/// /context read-only, AGS-817 /memory sync-trait, AGS-819 /theme
/// write) may extend this enum with additional variants. Each variant
/// should:
///
/// 1. Carry owned data (no borrows, no `Arc<Mutex<_>>` guards).
/// 2. Map 1:1 to a single write-side field on `SlashCommandContext`.
/// 3. Be applied in `command::context::apply_effect` via a new match
///    arm alongside `SetModelOverride`.
#[derive(Debug, Clone)]
pub(crate) enum CommandEffect {
    /// Overwrite `SlashCommandContext::model_override_shared` with the
    /// resolved full model id. Produced by `ModelHandler::execute`
    /// after `validate_model_name` succeeds. Applied by
    /// `command::context::apply_effect`, which awaits the mutex write
    /// at the dispatch site in `slash.rs`.
    SetModelOverride(String),
}

/// Trait every registered slash command handler implements.
///
/// `execute` runs the handler against the supplied context and
/// positional argument list. `description` is a one-line human label
/// used by `/help`, the command picker, and future introspection.
///
/// TASK-AGS-802: `aliases()` returns zero or more alternative names
/// the registry routes to the same handler. Default `&[]` keeps every
/// pre-existing handler wire-compatible — only handlers that opt in by
/// overriding the method contribute to the alias map.
pub(crate) trait CommandHandler: Send + Sync {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()>;
    fn description(&self) -> &str;

    /// Alternative names that resolve to this handler. The registry
    /// builds an alias -> primary-name map at init time; `Registry::get`
    /// falls back to that map when the direct lookup misses.
    ///
    /// Default empty slice: handlers that do not declare aliases do not
    /// contribute any entries. No allocations at call time.
    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }
}

/// Typed command table.
///
/// Owns `Arc<dyn CommandHandler>` so the dispatcher can clone handlers
/// out of the map cheaply and invoke them without holding a borrow on
/// the registry. Insertion order is irrelevant; lookup is by name.
///
/// TASK-AGS-802: an `aliases` map routes alternative names onto their
/// primary command. `get()` consults `commands` first, then falls back
/// to `aliases` for alias -> primary -> handler resolution. The alias
/// map does NOT inflate `len()`; `alias_count()` reports the alias
/// total separately.
pub(crate) struct Registry {
    commands: HashMap<&'static str, Arc<dyn CommandHandler>>,
    aliases: HashMap<&'static str, &'static str>,
}

impl Registry {
    /// Look up a registered handler by command name (without the
    /// leading `/`). Returns a cloned `Arc`, or `None` if no handler
    /// is registered under that name.
    ///
    /// Resolution order: primary-name map first, then alias map.
    /// Aliases resolve by looking up the primary name they target and
    /// re-reading the commands map.
    pub(crate) fn get(&self, name: &str) -> Option<Arc<dyn CommandHandler>> {
        if let Some(h) = self.commands.get(name) {
            return Some(Arc::clone(h));
        }
        let primary = self.aliases.get(name)?;
        self.commands.get(primary).cloned()
    }

    /// Number of registered primary commands. Aliases are counted
    /// separately (see [`Registry::alias_count`]).
    #[allow(dead_code)]
    pub(crate) fn len(&self) -> usize {
        self.commands.len()
    }

    /// Number of registered aliases (not counted against `len()`).
    #[allow(dead_code)]
    pub(crate) fn alias_count(&self) -> usize {
        self.aliases.len()
    }

    /// All primary command names, in unspecified order. Used by the
    /// dispatcher's unknown-command path to feed
    /// [`crate::command::parser::suggest`] with the list of candidates,
    /// and reused by TASK-AGS-804 for fuzzy-match hints.
    pub(crate) fn names(&self) -> Vec<&'static str> {
        self.commands.keys().copied().collect()
    }

    /// Test-only helper: returns `true` if `alias` is registered in the
    /// alias map. The `recall_is_standalone_not_alias` test uses this
    /// to enforce Steven's directive that `/recall` stays a primary
    /// command and is never an alias for anything.
    #[cfg(test)]
    pub(crate) fn aliases_map_contains(&self, alias: &str) -> bool {
        self.aliases.contains_key(alias)
    }

    /// TASK-AGS-807 helper: returns `true` if `name` is registered as a
    /// PRIMARY command (not just reachable via the alias map).
    ///
    /// Used by `crate::command::context::resolve_primary_from_input`
    /// to decide whether the parsed input name is already the primary
    /// or needs an alias→primary lookup.
    pub(crate) fn is_primary(&self, name: &str) -> bool {
        self.commands.contains_key(name)
    }

    /// TASK-AGS-807 helper: map an alias to its primary command name.
    /// Returns `None` if `alias` is not registered in the alias map.
    ///
    /// Alias entries are internalized as `&'static str`, so we can
    /// return a borrowed static reference without cloning.
    pub(crate) fn primary_for_alias(&self, alias: &str) -> Option<&'static str> {
        self.aliases.get(alias).copied()
    }
}

// ---------------------------------------------------------------------------
// Registry builder (init-time assembly + collision detection)
// ---------------------------------------------------------------------------

/// Assembles a [`Registry`] with alias support and panics on any of
/// three collision classes at build time:
///
/// 1. **Primary/primary**: two primaries sharing the same name.
/// 2. **Alias/primary**: an alias whose string equals an existing
///    primary name.
/// 3. **Alias/alias**: two handlers claiming the same alias.
///
/// Insertion order matters: callers must insert ALL primaries before
/// any aliases so the alias-vs-primary check can see every primary
/// name in the commands map. `build()` enforces this by walking every
/// primary handler's `aliases()` method after primaries are frozen.
pub(crate) struct RegistryBuilder {
    commands: HashMap<&'static str, Arc<dyn CommandHandler>>,
    primary_order: Vec<&'static str>,
}

impl RegistryBuilder {
    pub(crate) fn new() -> Self {
        Self {
            commands: HashMap::new(),
            primary_order: Vec::new(),
        }
    }

    /// Insert a primary command. Panics if the name is already
    /// registered.
    pub(crate) fn insert_primary(
        &mut self,
        name: &'static str,
        handler: Arc<dyn CommandHandler>,
    ) {
        if self.commands.contains_key(name) {
            panic!(
                "duplicate primary slash command: /{name} registered twice"
            );
        }
        self.commands.insert(name, handler);
        self.primary_order.push(name);
    }

    /// Freeze the commands map, walk every handler's `aliases()`,
    /// build the alias index, and detect alias/primary and alias/alias
    /// collisions. Panics on any collision.
    pub(crate) fn build(self) -> Registry {
        let Self {
            commands,
            primary_order,
        } = self;
        let mut aliases: HashMap<&'static str, &'static str> = HashMap::new();
        for primary in &primary_order {
            let handler = commands
                .get(primary)
                .expect("primary registered via insert_primary");
            for alias in handler.aliases() {
                if commands.contains_key(alias) {
                    panic!(
                        "alias collides with primary: alias '{alias}' (on /{primary}) matches existing primary command /{alias}"
                    );
                }
                if let Some(prior) = aliases.get(alias) {
                    panic!(
                        "duplicate alias: '{alias}' registered by both /{prior} and /{primary}"
                    );
                }
                aliases.insert(alias, primary);
            }
        }
        Registry { commands, aliases }
    }
}

// ---------------------------------------------------------------------------
// Handler placeholders
// ---------------------------------------------------------------------------
//
// Every existing slash command gets a zero-sized handler struct with a
// stub `execute` body. TASK-AGS-624 will migrate the real handler logic
// out of `main.rs::handle_slash_command` into these `execute` bodies.
// The macro below keeps each declaration to a single line so the file
// stays well under the 500-line budget.

macro_rules! declare_handler {
    ($struct_name:ident, $description:literal) => {
        struct $struct_name;
        impl CommandHandler for $struct_name {
            fn execute(
                &self,
                _ctx: &mut CommandContext,
                _args: &[String],
            ) -> anyhow::Result<()> {
                // TASK-AGS-624 will migrate the real handler body here
                // from main.rs::handle_slash_command.
                Ok(())
            }
            fn description(&self) -> &str {
                $description
            }
        }
    };
    // TASK-AGS-802 arm: handler declared with a static alias slice.
    ($struct_name:ident, $description:literal, $aliases:expr) => {
        struct $struct_name;
        impl CommandHandler for $struct_name {
            fn execute(
                &self,
                _ctx: &mut CommandContext,
                _args: &[String],
            ) -> anyhow::Result<()> {
                Ok(())
            }
            fn description(&self) -> &str {
                $description
            }
            fn aliases(&self) -> &'static [&'static str] {
                $aliases
            }
        }
    };
}

declare_handler!(FastHandler, "Toggle fast mode (lower quality, faster responses)");
declare_handler!(CompactHandler, "Compact the current conversation history");
declare_handler!(ClearHandler, "Clear the current conversation", &["cls"]);
declare_handler!(
    ExportHandler,
    "Export the current session to a file",
    &["save"]
);
declare_handler!(ThinkingHandler, "Toggle extended thinking display on/off");
declare_handler!(EffortHandler, "Show or set reasoning effort (high|medium|low)");
declare_handler!(GardenHandler, "Run memory garden consolidation or show stats");
// TASK-AGS-808: ModelHandler moved to `crate::command::model` (real
// impl with body-migrated execute via snapshot pattern for READ +
// effect-slot pattern for WRITE, aliases migrated from [models] to
// [m, switch-model] per spec). Imported at the top of this file.
declare_handler!(CopyHandler, "Copy the last assistant message to the clipboard");
// TASK-AGS-814: ContextHandler moved to `crate::command::context_cmd`
// (real impl with body-migrated execute via SNAPSHOT-ONLY pattern,
// aliases dropped from stub's [ctx] to []). Imported at the top of
// this file. See module rustdoc for the naming rationale
// (`context_cmd.rs` not `context.rs` — collision with builder module).
// TASK-AGS-807: StatusHandler moved to `crate::command::status` (real
// impl with body-migrated execute via snapshot pattern, alias migrated
// from [stat] to [info] per spec REQ-FOR-D7 validation criterion 2).
// Imported at the top of this file.
// TASK-AGS-809: CostHandler moved to `crate::command::cost` (real impl
// with body-migrated execute via snapshot pattern, READ-only, aliases
// migrated from [] to [usage, billing] per spec REQ-FOR-D7 validation
// criterion 2). Imported at the top of this file.
declare_handler!(PermissionsHandler, "Show or update tool permissions");
// TASK-AGS-813: ConfigHandler gains aliases [settings, prefs] via
// alias-only drift-reconcile (shipped-wins). Spec called for /settings
// as a primary — body-migrate deferred to a post-Stage-6 ticket.
declare_handler!(
    ConfigHandler,
    "Show or update Archon configuration",
    &["settings", "prefs"]
);
declare_handler!(
    MemoryHandler,
    "Inspect or manage long-term memory",
    &["mem"]
);
declare_handler!(DoctorHandler, "Run environment health checks");
declare_handler!(BugHandler, "Report a bug with current session context");
declare_handler!(DiffHandler, "Show a diff of recent file modifications");
declare_handler!(DenialsHandler, "List tool-use denials recorded this session");
declare_handler!(LoginHandler, "Authenticate against the configured backend");
declare_handler!(VimHandler, "Toggle vim-style modal input");
declare_handler!(UsageHandler, "Show aggregate API usage for the session");
// TASK-AGS-806: TasksHandler moved to `crate::command::task` (real
// impl with body-migrated execute, alias set extended to
// [todo, ps, jobs], and TuiEvent::OpenView(ViewId::Tasks) forward-
// compat per AGS-822). Imported at the top of this file.
declare_handler!(ReleaseNotesHandler, "Show release notes for the current build");
declare_handler!(ReloadHandler, "Reload configuration from disk");
declare_handler!(LogoutHandler, "Clear stored credentials");
declare_handler!(
    HelpHandler,
    "Show help for commands and shortcuts",
    &["?", "h"]
);
declare_handler!(RenameHandler, "Rename the current session");
// TASK-AGS-810: ResumeHandler moved to `crate::command::resume` (real
// impl with body-migrated execute via DIRECT pattern — sync
// archon_session API reads, no snapshot/effect-slot needed). Aliases
// migrated from [continue] to [continue, open-session] per spec
// REQ-FOR-D7 validation criterion 4. Imported at the top of this file.
// TASK-AGS-811: McpHandler moved to `crate::command::mcp` (real impl
// with body-migrated execute via SNAPSHOT-ONLY pattern — async
// `McpServerManager::get_server_info` / `list_tools_for` calls move
// to the builder). No aliases (shipped stub had none; spec lists
// none). Imported at the top of this file.
// TASK-AGS-815: ForkHandler moved to `crate::command::fork` (real impl
// with body-migrated execute via DIRECT pattern — sync
// archon_session::fork::fork_session + SessionStore::open, no
// snapshot/effect-slot needed; session_id threads through
// CommandContext::session_id populated unconditionally by
// build_command_context). No aliases. Imported at the top of this file.
declare_handler!(CheckpointHandler, "Create or restore a session checkpoint");
declare_handler!(AddDirHandler, "Add a directory to the working context");
declare_handler!(ColorHandler, "Show or change the UI color scheme");
declare_handler!(ThemeHandler, "Show or change the UI theme");
// /recall stays a standalone primary command and has NO aliases —
// Steven directive. Do NOT add "recall" as an alias on /memory or any
// other handler.
declare_handler!(RecallHandler, "Recall memories matching a query");
declare_handler!(RulesHandler, "List, edit, or remove behavioral rules");
// TASK-AGS-805: /cancel thin wrapper. Body-migrate deferred (shipped
// CommandContext does not expose `task_service`; the stub returns
// `Ok(())` consistent with the 37 peer handlers). Aliases `stop` and
// `abort` route to this handler via the registry alias map.
declare_handler!(
    CancelHandler,
    "Cancel the currently running task",
    &["stop", "abort"]
);

/// Build the default command registry containing every slash command
/// currently dispatched from `main.rs::handle_slash_command`.
///
/// Each command name maps to a `pub(crate)` zero-sized handler struct
/// whose `execute` body is a no-op stub. Migrating the real bodies
/// out of `handle_slash_command` is scoped to TASK-AGS-624 / Phase 8.
pub(crate) fn default_registry() -> Registry {
    let mut b = RegistryBuilder::new();
    // Primaries FIRST — builder panics on duplicate primary names.
    b.insert_primary("fast", Arc::new(FastHandler));
    b.insert_primary("compact", Arc::new(CompactHandler));
    b.insert_primary("clear", Arc::new(ClearHandler));
    b.insert_primary("export", Arc::new(ExportHandler));
    b.insert_primary("thinking", Arc::new(ThinkingHandler));
    b.insert_primary("effort", Arc::new(EffortHandler));
    b.insert_primary("garden", Arc::new(GardenHandler));
    b.insert_primary("model", Arc::new(ModelHandler));
    b.insert_primary("copy", Arc::new(CopyHandler));
    b.insert_primary("context", Arc::new(ContextHandler));
    b.insert_primary("status", Arc::new(StatusHandler));
    b.insert_primary("cost", Arc::new(CostHandler));
    b.insert_primary("permissions", Arc::new(PermissionsHandler));
    b.insert_primary("config", Arc::new(ConfigHandler));
    b.insert_primary("memory", Arc::new(MemoryHandler));
    b.insert_primary("doctor", Arc::new(DoctorHandler));
    b.insert_primary("bug", Arc::new(BugHandler));
    b.insert_primary("diff", Arc::new(DiffHandler));
    b.insert_primary("denials", Arc::new(DenialsHandler));
    b.insert_primary("login", Arc::new(LoginHandler));
    b.insert_primary("vim", Arc::new(VimHandler));
    b.insert_primary("usage", Arc::new(UsageHandler));
    b.insert_primary("tasks", Arc::new(TasksHandler));
    b.insert_primary("release-notes", Arc::new(ReleaseNotesHandler));
    b.insert_primary("reload", Arc::new(ReloadHandler));
    b.insert_primary("logout", Arc::new(LogoutHandler));
    b.insert_primary("help", Arc::new(HelpHandler));
    b.insert_primary("rename", Arc::new(RenameHandler));
    b.insert_primary("resume", Arc::new(ResumeHandler));
    b.insert_primary("mcp", Arc::new(McpHandler));
    // TASK-AGS-812: NEW /hooks primary (gap-fix Q4=A, no aliases).
    b.insert_primary("hooks", Arc::new(HooksHandler));
    b.insert_primary("fork", Arc::new(ForkHandler));
    b.insert_primary("checkpoint", Arc::new(CheckpointHandler));
    b.insert_primary("add-dir", Arc::new(AddDirHandler));
    b.insert_primary("color", Arc::new(ColorHandler));
    b.insert_primary("theme", Arc::new(ThemeHandler));
    b.insert_primary("recall", Arc::new(RecallHandler));
    b.insert_primary("rules", Arc::new(RulesHandler));
    // TASK-AGS-805: /cancel primary (aliases: stop, abort).
    b.insert_primary("cancel", Arc::new(CancelHandler));
    // TASK-AGS-816: NEW /voice primary (gap-fix Q4=A, no aliases).
    b.insert_primary("voice", Arc::new(VoiceHandler));
    // Aliases are collected from each handler's aliases() method
    // inside RegistryBuilder::build(). Collisions panic.
    b.build()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Count of distinct command names extracted from the 37 match
    /// arms in `main.rs::handle_slash_command` as of TASK-AGS-622.
    /// Two of those arms (`/compact | /clear` and the `/thinking`
    /// family) contribute separately-named commands, so the baseline
    /// count was 37 unique names.
    ///
    /// TASK-AGS-805 adds `/cancel` (gap-fix thin wrapper) as a new
    /// primary, bringing the total to 38.
    ///
    /// TASK-AGS-812 adds `/hooks` (gap-fix Q4=A) as a new primary,
    /// bringing the total to 39.
    ///
    /// TASK-AGS-816 adds `/voice` (gap-fix Q4=A, SECOND Batch-3 NEW
    /// primary) as a new primary, bringing the total to 40.
    const EXPECTED_COMMAND_COUNT: usize = 40;

    #[test]
    fn default_registry_contains_all_commands() {
        let registry = default_registry();
        assert_eq!(
            registry.len(),
            EXPECTED_COMMAND_COUNT,
            "default_registry must register every pre-TASK-AGS-622 slash command"
        );
    }

    #[test]
    fn default_registry_includes_fast() {
        assert!(default_registry().get("fast").is_some());
    }

    #[test]
    fn default_registry_includes_help() {
        assert!(default_registry().get("help").is_some());
    }

    #[test]
    fn default_registry_includes_config() {
        assert!(default_registry().get("config").is_some());
    }

    #[test]
    fn default_registry_includes_rules() {
        assert!(default_registry().get("rules").is_some());
    }

    #[test]
    fn default_registry_includes_thinking() {
        assert!(default_registry().get("thinking").is_some());
    }

    #[test]
    fn default_registry_includes_compact_and_clear_separately() {
        let registry = default_registry();
        assert!(registry.get("compact").is_some());
        assert!(registry.get("clear").is_some());
    }

    #[test]
    fn unknown_command_returns_none() {
        assert!(default_registry().get("nonexistent").is_none());
    }

    #[test]
    fn handler_description_is_non_empty() {
        let registry = default_registry();
        let handler = registry.get("fast").expect("fast handler registered");
        assert!(!handler.description().is_empty());
    }

    #[test]
    fn registry_lookup_returns_arc() {
        let registry = default_registry();
        let first = registry.get("fast");
        let second = registry.get("fast");
        assert!(first.is_some());
        assert!(second.is_some());
    }

    // -----------------------------------------------------------------
    // TASK-AGS-802: 9 new tests for alias support + collision panics.
    // -----------------------------------------------------------------

    /// Minimal handler used by collision tests (test-local, no real body).
    struct TestHandler {
        desc: &'static str,
        aliases: &'static [&'static str],
    }
    impl CommandHandler for TestHandler {
        fn execute(
            &self,
            _ctx: &mut CommandContext,
            _args: &[String],
        ) -> anyhow::Result<()> {
            Ok(())
        }
        fn description(&self) -> &str {
            self.desc
        }
        fn aliases(&self) -> &'static [&'static str] {
            self.aliases
        }
    }

    /// Handler with no alias override — exercises the default empty-slice
    /// implementation on the trait.
    struct NoAliasHandler;
    impl CommandHandler for NoAliasHandler {
        fn execute(
            &self,
            _ctx: &mut CommandContext,
            _args: &[String],
        ) -> anyhow::Result<()> {
            Ok(())
        }
        fn description(&self) -> &str {
            "no-alias handler (test only)"
        }
    }

    #[test]
    fn aliases_method_default_empty() {
        // A handler that does NOT override aliases() returns &[].
        let h = NoAliasHandler;
        assert_eq!(h.aliases(), &[] as &[&'static str]);
    }

    #[test]
    fn default_registry_resolves_alias_to_primary() {
        // "h" is an alias for "help" in the starter set. Resolution
        // must return the SAME handler (same description) as the
        // primary lookup.
        let registry = default_registry();
        let via_primary = registry.get("help").expect("primary help registered");
        let via_alias = registry.get("h").expect("alias h resolves to help");
        assert_eq!(
            via_primary.description(),
            via_alias.description(),
            "alias must resolve to same handler as primary"
        );
    }

    #[test]
    fn default_registry_alias_count_minimum() {
        let registry = default_registry();
        assert!(
            registry.alias_count() >= 8,
            "starter set must have >= 8 aliases, got {}",
            registry.alias_count()
        );
    }

    #[test]
    #[should_panic(expected = "duplicate primary")]
    fn duplicate_primary_name_panics() {
        // Two primaries with the same name must panic at build time.
        let mut b = RegistryBuilder::new();
        b.insert_primary("dup", Arc::new(NoAliasHandler));
        b.insert_primary("dup", Arc::new(NoAliasHandler));
        let _ = b.build();
    }

    #[test]
    #[should_panic(expected = "alias collides with primary")]
    fn alias_collides_with_primary_panics() {
        // An alias equal to an existing primary name must panic.
        let h = Arc::new(TestHandler {
            desc: "has alias 'existing'",
            aliases: &["existing"],
        });
        let other = Arc::new(NoAliasHandler);
        let mut b = RegistryBuilder::new();
        b.insert_primary("existing", other);
        b.insert_primary("mycmd", h);
        let _ = b.build();
    }

    #[test]
    #[should_panic(expected = "duplicate alias")]
    fn alias_collides_with_alias_panics() {
        // Two handlers claiming the same alias must panic.
        let a = Arc::new(TestHandler {
            desc: "handler a",
            aliases: &["shared"],
        });
        let b_h = Arc::new(TestHandler {
            desc: "handler b",
            aliases: &["shared"],
        });
        let mut b = RegistryBuilder::new();
        b.insert_primary("alpha", a);
        b.insert_primary("beta", b_h);
        let _ = b.build();
    }

    #[test]
    fn registry_len_counts_primaries_only() {
        // Aliases must NOT inflate the primary count.
        let registry = default_registry();
        assert_eq!(
            registry.len(),
            EXPECTED_COMMAND_COUNT,
            "len() must count primaries only, not primaries + aliases"
        );
    }

    #[test]
    fn registry_names_returns_all_primaries() {
        let registry = default_registry();
        let names = registry.names();
        assert_eq!(
            names.len(),
            EXPECTED_COMMAND_COUNT,
            "names() must return one entry per primary command"
        );
        // Spot-check a few well-known primaries.
        assert!(names.contains(&"help"));
        assert!(names.contains(&"recall"));
        assert!(names.contains(&"config"));
    }

    #[test]
    fn recall_is_standalone_not_alias() {
        // /recall stays a primary command and is NOT registered as an
        // alias for anything (Steven directive).
        let registry = default_registry();
        let handler = registry.get("recall").expect("recall is a primary");
        assert!(
            handler.description().to_lowercase().contains("recall")
                || handler.description().to_lowercase().contains("memor"),
            "recall handler description should reference recall/memory, got: {}",
            handler.description()
        );
        assert!(
            !registry.aliases_map_contains("recall"),
            "recall must NOT appear as an alias"
        );
    }

    // -----------------------------------------------------------------
    // TASK-AGS-805: /cancel registration + aliases (stop, abort).
    // Body-migrate is deferred until CommandContext exposes a task
    // service; these tests verify the registry-level wiring only.
    // -----------------------------------------------------------------

    #[test]
    fn cancel_primary_registered() {
        let registry = default_registry();
        let handler = registry
            .get("cancel")
            .expect("cancel must be registered as a primary");
        assert!(
            !handler.description().is_empty(),
            "cancel handler must carry a non-empty description"
        );
    }

    // -----------------------------------------------------------------
    // TASK-AGS-807: /status alias `info` resolves to the /status handler.
    // -----------------------------------------------------------------

    #[test]
    fn registry_resolves_status_alias_info() {
        let reg = default_registry();
        let primary = reg
            .get("status")
            .expect("status primary must be registered");
        let via_info = reg
            .get("info")
            .expect("'info' alias must resolve to /status per AGS-807");
        assert_eq!(
            primary.description(),
            via_info.description(),
            "'info' must resolve to the same handler as /status"
        );
        // Also pin the Registry helper APIs introduced for the
        // builder's alias-aware primary-name resolution.
        assert!(reg.is_primary("status"));
        assert!(!reg.is_primary("info"));
        assert_eq!(reg.primary_for_alias("info"), Some("status"));
        assert_eq!(reg.primary_for_alias("status"), None);
    }

    #[test]
    fn cancel_aliases_resolve_to_cancel_handler() {
        let registry = default_registry();
        let primary = registry
            .get("cancel")
            .expect("cancel primary registered");
        let via_stop = registry
            .get("stop")
            .expect("alias 'stop' must resolve to cancel");
        let via_abort = registry
            .get("abort")
            .expect("alias 'abort' must resolve to cancel");
        assert_eq!(
            primary.description(),
            via_stop.description(),
            "'stop' must resolve to the same handler as /cancel"
        );
        assert_eq!(
            primary.description(),
            via_abort.description(),
            "'abort' must resolve to the same handler as /cancel"
        );
    }

    // -----------------------------------------------------------------
    // TASK-AGS-808: /model aliases [m, switch-model] + CommandEffect
    // enum sanity. The /model body-migrate moves ModelHandler out of
    // the declare_handler! stub and into `crate::command::model`.
    // -----------------------------------------------------------------

    #[test]
    fn registry_resolves_model_aliases_m_and_switch_model() {
        let reg = default_registry();
        let primary = reg
            .get("model")
            .expect("model primary must be registered");
        let via_m = reg
            .get("m")
            .expect("'m' alias must resolve to /model per AGS-808");
        let via_switch_model = reg
            .get("switch-model")
            .expect("'switch-model' alias must resolve to /model per AGS-808");
        assert_eq!(
            primary.description(),
            via_m.description(),
            "'m' must resolve to the same handler as /model"
        );
        assert_eq!(
            primary.description(),
            via_switch_model.description(),
            "'switch-model' must resolve to the same handler as /model"
        );
        // Pin the Registry helper APIs — `model` is a primary,
        // `m` is not.
        assert!(reg.is_primary("model"));
        assert!(!reg.is_primary("m"));
        assert!(!reg.is_primary("switch-model"));
        assert_eq!(reg.primary_for_alias("m"), Some("model"));
        assert_eq!(reg.primary_for_alias("switch-model"), Some("model"));
        assert_eq!(reg.primary_for_alias("model"), None);
    }

    // -----------------------------------------------------------------
    // TASK-AGS-809: /cost aliases [billing] (collision-adjusted from
    // the spec-requested [usage, billing] — see cost.rs rustdoc for
    // the CONFIRM R-item: `usage` is already a shipped primary).
    // -----------------------------------------------------------------

    #[test]
    fn registry_resolves_cost_aliases_usage_and_billing() {
        let reg = default_registry();
        let primary = reg
            .get("cost")
            .expect("cost primary must be registered");
        let via_billing = reg
            .get("billing")
            .expect("'billing' alias must resolve to /cost per AGS-809");
        assert_eq!(
            primary.description(),
            via_billing.description(),
            "'billing' must resolve to the same handler as /cost"
        );

        // `usage` stays a PRIMARY (UsageHandler) — must NOT resolve to
        // /cost. Enforces the collision-avoidance invariant.
        let via_usage = reg
            .get("usage")
            .expect("'usage' must still resolve — it is a shipped primary");
        assert_ne!(
            primary.description(),
            via_usage.description(),
            "'usage' must remain bound to UsageHandler, not /cost"
        );

        // Pin the Registry helper APIs — `cost` and `usage` are BOTH
        // primaries (independent); `billing` is the only /cost alias.
        assert!(reg.is_primary("cost"));
        assert!(reg.is_primary("usage"));
        assert!(!reg.is_primary("billing"));
        assert_eq!(reg.primary_for_alias("billing"), Some("cost"));
        assert_eq!(reg.primary_for_alias("usage"), None);
        assert_eq!(reg.primary_for_alias("cost"), None);
    }

    // -----------------------------------------------------------------
    // TASK-AGS-810: /resume aliases [continue, open-session] resolve.
    // DIRECT-pattern body-migrate — no snapshot or effect slot. This
    // test pins the alias surface so future ticketing cannot silently
    // drop `open-session` (AGS-810 spec validation criterion 4).
    // -----------------------------------------------------------------

    #[test]
    fn registry_resolves_resume_aliases_continue_and_open_session() {
        let reg = default_registry();
        let primary = reg
            .get("resume")
            .expect("resume primary must be registered");
        let via_continue = reg
            .get("continue")
            .expect("'continue' alias must resolve to /resume");
        let via_open_session = reg
            .get("open-session")
            .expect(
                "'open-session' alias must resolve to /resume per AGS-810",
            );
        assert_eq!(
            primary.description(),
            via_continue.description(),
            "'continue' must resolve to the same handler as /resume"
        );
        assert_eq!(
            primary.description(),
            via_open_session.description(),
            "'open-session' must resolve to the same handler as /resume"
        );

        // Pin the Registry helper APIs — `resume` is a primary,
        // `continue` and `open-session` are aliases (not primaries).
        assert!(reg.is_primary("resume"));
        assert!(!reg.is_primary("continue"));
        assert!(!reg.is_primary("open-session"));
        assert_eq!(reg.primary_for_alias("continue"), Some("resume"));
        assert_eq!(reg.primary_for_alias("open-session"), Some("resume"));
        assert_eq!(reg.primary_for_alias("resume"), None);
    }

    // -----------------------------------------------------------------
    // TASK-AGS-811: /mcp primary registration (no aliases). The /mcp
    // body-migrate moves McpHandler out of the declare_handler! stub
    // and into `crate::command::mcp`. Shipped stub had no aliases and
    // the AGS-811 spec lists none either — this test pins that
    // invariant so future ticketing cannot silently introduce one
    // without updating the registry collision-detection tests.
    // -----------------------------------------------------------------

    // -----------------------------------------------------------------
    // TASK-AGS-812: /hooks primary registration (no aliases). The
    // /hooks gap-fix adds a brand-new primary — there was NO prior
    // /hooks entry in the shipped match block or registry. Pin the
    // invariant so future ticketing cannot silently introduce an
    // alias without updating the registry collision-detection tests,
    // and cannot silently promote a sibling handler to share the
    // `hooks` primary name.
    // -----------------------------------------------------------------

    // -----------------------------------------------------------------
    // TASK-AGS-813: /settings and /prefs alias onto /config primary.
    // ALIAS-ONLY ticket — no new primary, no body-migrate. Spec called
    // for /settings as a primary with body+get/set; shipped-wins
    // drift-reconcile inverts the relationship: existing /config
    // primary gains [settings, prefs] aliases. Pin the alias surface
    // and the primary/alias directionality so future ticketing cannot
    // silently flip it or drop an alias.
    // -----------------------------------------------------------------

    #[test]
    fn registry_resolves_config_aliases_settings_and_prefs() {
        let reg = default_registry();
        assert_eq!(reg.primary_for_alias("settings"), Some("config"));
        assert_eq!(reg.primary_for_alias("prefs"), Some("config"));
        assert_eq!(reg.primary_for_alias("config"), None); // primary, not alias
        assert!(!reg.is_primary("settings")); // alias-only, not a primary
        assert!(!reg.is_primary("prefs"));
        assert!(reg.is_primary("config")); // primary remains
    }

    #[test]
    fn registry_hooks_primary_with_no_aliases() {
        let reg = default_registry();
        let primary = reg
            .get("hooks")
            .expect("hooks primary must be registered post AGS-812");
        let desc = primary.description().to_lowercase();
        assert!(
            desc.contains("hook"),
            "HooksHandler description should reference 'hook', got: {}",
            primary.description()
        );
        // `hooks` is a primary — not an alias of anything.
        assert!(reg.is_primary("hooks"));
        assert_eq!(reg.primary_for_alias("hooks"), None);
        // No alias entry points to `hooks`.
        assert!(!reg.aliases_map_contains("hooks"));
    }

    // -----------------------------------------------------------------
    // TASK-AGS-816: /voice primary registration (no aliases). The
    // /voice gap-fix adds a brand-new primary — there was NO prior
    // /voice entry in the shipped match block or registry. SECOND
    // Batch-3 NEW primary (after AGS-812 /hooks). Pin the invariant so
    // future ticketing cannot silently introduce an alias without
    // updating the registry collision-detection tests, and cannot
    // silently promote a sibling handler to share the `voice` primary
    // name.
    // -----------------------------------------------------------------

    #[test]
    fn registry_voice_primary_with_no_aliases() {
        let reg = default_registry();
        let primary = reg
            .get("voice")
            .expect("voice primary must be registered post AGS-816");
        let desc = primary.description().to_lowercase();
        assert!(
            desc.contains("voice"),
            "VoiceHandler description should reference 'voice', got: {}",
            primary.description()
        );
        // `voice` is a primary — not an alias of anything.
        assert!(reg.is_primary("voice"));
        assert_eq!(reg.primary_for_alias("voice"), None);
        // No alias entry points to `voice`.
        assert!(!reg.aliases_map_contains("voice"));
    }

    // -----------------------------------------------------------------
    // TASK-AGS-814: /context primary registration (no aliases). The
    // /context body-migrate moves ContextHandler out of the
    // declare_handler! stub and into `crate::command::context_cmd`.
    // Shipped stub had `&["ctx"]` but the legacy match arm in slash.rs
    // only matched `/context` literally — the alias was cosmetic. Real
    // handler drops it to `&[]` to align with user-visible behaviour.
    // Pin the invariant so future ticketing cannot silently re-add
    // `ctx` (or any other alias) without updating the registry
    // collision-detection tests.
    // -----------------------------------------------------------------

    #[test]
    fn registry_context_primary_with_no_aliases() {
        let reg = default_registry();
        let primary = reg
            .get("context")
            .expect("context primary must be registered post AGS-814");
        let desc = primary.description().to_lowercase();
        assert!(
            desc.contains("context")
                || desc.contains("window")
                || desc.contains("usage"),
            "ContextHandler description should reference \
             context/window/usage, got: {}",
            primary.description()
        );
        // `context` is a primary — not an alias of anything.
        assert!(reg.is_primary("context"));
        assert_eq!(reg.primary_for_alias("context"), None);
        // No alias entry points to `context`. Also spot-check that
        // the shipped stub's `ctx` alias is GONE — AGS-814 drops it.
        assert!(!reg.aliases_map_contains("context"));
        assert!(
            !reg.aliases_map_contains("ctx"),
            "'ctx' alias must NOT be registered post AGS-814 — the \
             shipped stub had it but the legacy match arm only matched \
             `/context` literally so the alias was cosmetic"
        );
        assert_eq!(reg.primary_for_alias("ctx"), None);
    }

    #[test]
    fn registry_mcp_primary_with_no_aliases() {
        let reg = default_registry();
        let primary = reg
            .get("mcp")
            .expect("mcp primary must be registered post AGS-811");
        let desc = primary.description().to_lowercase();
        assert!(
            desc.contains("mcp") || desc.contains("server"),
            "McpHandler description should reference mcp/server, got: {}",
            primary.description()
        );
        // `mcp` is a primary — not an alias of anything.
        assert!(reg.is_primary("mcp"));
        assert_eq!(reg.primary_for_alias("mcp"), None);
        // No alias entry points to `mcp`. Walk the aliases_map via the
        // test-only helper for a spot-check of common collision
        // candidates — none should resolve to /mcp.
        assert!(!reg.aliases_map_contains("mcp"));
    }

    // -----------------------------------------------------------------
    // TASK-AGS-815: /fork primary registration (no aliases). The
    // /fork body-migrate moves ForkHandler out of the
    // declare_handler! stub at registry.rs:524 and into
    // `crate::command::fork`. Shipped stub had `&[]` (no aliases);
    // spec lists none; handler ships `&[]`. Pin the invariant so
    // future ticketing cannot silently add an alias without updating
    // the registry collision-detection tests.
    // -----------------------------------------------------------------

    #[test]
    fn registry_fork_primary_with_no_aliases() {
        let reg = default_registry();
        let primary = reg
            .get("fork")
            .expect("fork primary must be registered post AGS-815");
        let desc = primary.description().to_lowercase();
        assert!(
            desc.contains("fork") || desc.contains("session"),
            "ForkHandler description should reference fork/session, \
             got: {}",
            primary.description()
        );
        // `fork` is a primary — not an alias of anything.
        assert!(reg.is_primary("fork"));
        assert_eq!(reg.primary_for_alias("fork"), None);
        // No alias entry points to `fork`.
        assert!(!reg.aliases_map_contains("fork"));
    }

    #[test]
    fn command_effect_debug_and_clone() {
        // Sanity: CommandEffect derives Debug + Clone and the
        // SetModelOverride variant round-trips its payload without
        // panic. Prevents accidental removal of the derives that
        // ModelHandler tests depend on for assertions.
        let e = CommandEffect::SetModelOverride("claude-sonnet-4-6".to_string());
        let cloned = e.clone();
        match cloned {
            CommandEffect::SetModelOverride(s) => {
                assert_eq!(s, "claude-sonnet-4-6");
            }
        }
        // Debug impl must not panic — format! exercises it.
        let _ = format!("{e:?}");
    }
}
