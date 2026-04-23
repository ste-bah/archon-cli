//! Slash command registry.
//!
//! # Breadcrumb line-number conventions (POST-STAGE-6)
//!
//! Comment-level `registry.rs:NNN` and `slash.rs:NNN` references
//! throughout the command/ module are historical — they cite the
//! SHIPPED line of a declare_handler! stub (pre-POST-6-NO-STUB) or
//! a shipped match-arm body in slash.rs (pre-POST-6-FALLTHROUGH).
//! The cited content has been deleted/relocated by the
//! POST-STAGE-6 migration stream (B01-B24 + POST-6-NO-STUB +
//! POST-6-FALLTHROUGH + POST-6-DISPATCH-SMOKE). For current
//! locations, grep by symbol name rather than line number.
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
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
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
// TASK-AGS-818: real /export handler lives in `crate::command::export`.
// CANARY-pattern registry-hygiene migration (Option D) — shipped
// /export body stays in session.rs:2409-2480 (intercept-before-
// dispatcher) under a zero-diff invariant held since AGS-805. The
// handler here exists only to (a) clear the `declare_handler!` stub
// and (b) emit a diagnostic canary message if the dispatcher ever DOES
// reach it, which would signal a dispatch-ordering regression. Aliases
// `["save"]` are PRESERVED per shipped-wins drift-reconcile (AGS-817
// /memory precedent). Real body-migrate deferred to POST-STAGE-6
// (ticket AGS-POST-6-EXPORT). See `src/command/export.rs` module
// rustdoc for R1..R5.
use crate::command::export::ExportHandler;
// TASK-AGS-817: real /memory handler lives in `crate::command::memory`.
// DIRECT-pattern body-migrate (sync `archon_memory::MemoryTrait` — all
// 12 methods plain `fn`; no snapshot/effect-slot needed). The handler
// reads `Arc<dyn MemoryTrait>` from a new `CommandContext::memory` field
// populated UNCONDITIONALLY by `build_command_context` (mirrors AGS-815
// session_id cross-cutting precedent). Shipped stub
// `declare_handler!(MemoryHandler, "Inspect or manage long-term memory",
// &["mem"])` at registry.rs:521-525 is REPLACED by this import + the
// insert_primary call below. Aliases `["mem"]` are PRESERVED per
// shipped-wins drift-reconcile (see command/memory.rs Aliases rustdoc).
use crate::command::memory::MemoryHandler;
// TASK-AGS-POST-6-BODIES-B01-FAST: real /fast handler lives in
// `crate::command::fast`. DIRECT-pattern body-migrate (sync atomic
// toggle on `Arc<AtomicBool>`; no snapshot/effect-slot needed). The
// handler reads `Option<Arc<AtomicBool>>` from a new
// `CommandContext::fast_mode_shared` field populated UNCONDITIONALLY
// by `build_command_context` (mirrors AGS-815 session_id and AGS-817
// memory cross-cutting precedent). Shipped stub
// `declare_handler!(FastHandler, "Toggle fast mode (lower quality,
// faster responses)")` at registry.rs:546 is REPLACED by this import
// + the insert_primary call below. No aliases — shipped stub had
// none and spec lists none.
use crate::command::fast::FastHandler;
// TASK-AGS-POST-6-BODIES-B02-THINKING: real /thinking handler lives in
// `crate::command::thinking`. DIRECT-pattern body-migrate (sync atomic
// store on `Arc<AtomicBool>` + ThinkingToggle/TextDelta TuiEvent
// emissions; no snapshot/effect-slot needed). The handler reads
// `Option<Arc<AtomicBool>>` from a new `CommandContext::show_thinking`
// field populated UNCONDITIONALLY by `build_command_context` (mirrors
// AGS-815 session_id, AGS-817 memory, and B01-FAST fast_mode_shared
// cross-cutting precedent). Shipped stub
// `declare_handler!(ThinkingHandler, "Toggle extended thinking display
// on/off")` at registry.rs:587 is REPLACED by this import + the
// insert_primary call below. No aliases — shipped stub had none and
// spec lists none. Subcommands `on`/`off`/empty are positional args
// dispatched through the same primary, NOT aliases.
use crate::command::thinking::ThinkingHandler;
// TASK-AGS-POST-6-BODIES-B03-BUG: real /bug handler lives in
// `crate::command::bug`. DIRECT-pattern body-migrate (TRIVIAL variant —
// no state, no args, no subcommand, no snapshot/effect-slot, no new
// CommandContext field). Single TextDelta emission of the bug-report
// URL; trailing args ignored (always emit). Shipped stub
// `declare_handler!(BugHandler, "Report a bug with current session
// context")` at registry.rs:658 is REPLACED by this import + the
// insert_primary call below. No aliases — shipped stub had none and
// spec lists none. Simpler than B01-FAST and B02-THINKING.
use crate::command::bug::BugHandler;
// TASK-AGS-POST-6-BODIES-B04-DIFF: real /diff handler lives in
// `crate::command::diff`. DIRECT with-effect body-migrate (sync handler
// stashes `CommandEffect::RunGitDiffStat(PathBuf)`; dispatch-site
// `apply_effect` awaits the existing `handle_diff_command` helper at
// slash.rs:120 which spawns `git diff --stat` via tokio::process).
// Subprocess await requires the effect-slot indirection — cannot run
// inside sync `CommandHandler::execute`. The handler reads
// `Option<PathBuf>` from a new `CommandContext::working_dir` field
// populated UNCONDITIONALLY by `build_command_context` (mirrors AGS-815
// session_id, AGS-817 memory, B01-FAST fast_mode_shared, B02-THINKING
// show_thinking cross-cutting precedent). Shipped stub
// `declare_handler!(DiffHandler, "Show a diff of recent file
// modifications")` at registry.rs:673 is REPLACED by this import + the
// insert_primary call below. No aliases — shipped stub had none and
// spec lists none. FOURTH Batch-A body-migrate (after B01-FAST,
// B02-THINKING, B03-BUG).
use crate::command::diff::DiffHandler;
// TASK-AGS-POST-6-BODIES-B05-VIM: real /vim handler lives in
// `crate::command::vim`. DIRECT-pattern body-migrate (emit-only sync
// handler — two `try_send` calls replace the shipped
// `tui_tx.send().await` pair at slash.rs:407-413; no state mutation,
// no new CommandContext field). FIFTH Batch-A body-migrate (after
// B01-FAST, B02-THINKING, B03-BUG, B04-DIFF). Shipped stub
// `declare_handler!(VimHandler, "Toggle vim-style modal input")` at
// registry.rs:728 is REPLACED by this import + the insert_primary
// call below. No aliases — shipped stub had none and spec lists none.
use crate::command::vim::VimHandler;
// TASK-AGS-POST-6-BODIES-B06-HELP: real /help handler lives in
// `crate::command::help`. DIRECT-with-field body-migrate (sync
// `SkillRegistry::format_help` / `format_skill_help` — both plain `fn`;
// no snapshot/effect-slot needed). The handler reads
// `Option<Arc<SkillRegistry>>` from a new `CommandContext::skill_registry`
// field populated UNCONDITIONALLY by `build_command_context` (mirrors
// AGS-815 session_id, AGS-817 memory, B01-FAST fast_mode_shared,
// B02-THINKING show_thinking, B04-DIFF working_dir cross-cutting
// precedent). SIXTH Batch-A body-migrate (after B01-FAST, B02-THINKING,
// B03-BUG, B04-DIFF, B05-VIM). Shipped stub
// `declare_handler!(HelpHandler, "Show help for commands and shortcuts",
// &["?", "h"])` at registry.rs:749 is REPLACED by this import + the
// insert_primary call below. Aliases `["?", "h"]` are PRESERVED per
// shipped-wins drift-reconcile, carried on the handler via the
// `CommandHandler::aliases()` trait method.
use crate::command::help::HelpHandler;
// TASK-AGS-POST-6-BODIES-B07-RELEASE-NOTES: real /release-notes handler
// lives in `crate::command::release_notes`. DIRECT-pattern body-migrate
// (emit-only sync handler — single `ctx.tui_tx.try_send(TuiEvent::TextDelta(..))`
// with byte-identical static literal; no snapshot/effect-slot needed,
// no new CommandContext field added). SEVENTH Batch-A body-migrate
// (after B01-FAST, B02-THINKING, B03-BUG, B04-DIFF, B05-VIM, B06-HELP).
// Shipped stub `declare_handler!(ReleaseNotesHandler, "Show release notes for the current build")`
// at registry.rs:787 is REPLACED by this import + the insert_primary
// call below. No aliases — shipped stub had none and spec lists none.
use crate::command::release_notes::ReleaseNotesHandler;
// TASK-AGS-POST-6-BODIES-B20-RELOAD: real /reload handler lives in
// `crate::command::reload`. DIRECT-pattern body-migrate —
// `archon_core::config_watcher::force_reload(config_paths: &[PathBuf],
// current: &ArchonConfig) -> Result<(ArchonConfig, Vec<String>),
// ConfigError>` is sync; no snapshot/effect-slot needed. A new
// cross-cutting `CommandContext::config_path: Option<PathBuf>` field
// is populated UNCONDITIONALLY by `build_command_context` per the
// AGS-815 session_id / AGS-817 memory precedent.
use crate::command::reload::ReloadHandler;
// TASK-AGS-POST-6-BODIES-B17-RENAME: real /rename handler lives in
// `crate::command::rename`. DIRECT-pattern body-migrate — both
// `archon_session::storage::SessionStore::open` and
// `archon_session::naming::set_session_name` are sync, and
// `CommandContext::session_id: Option<String>` is already populated
// unconditionally by `build_command_context` per the AGS-815 /fork
// precedent. No snapshot/effect-slot needed, no new CommandContext
// field added. Shipped stub `declare_handler!(RenameHandler, "Rename
// the current session")` at registry.rs:1234 is REPLACED by this
// import + the insert_primary call below. No aliases — shipped stub
// used the two-arg declare_handler! form (no aliases slice) and spec
// lists none. See `src/command/rename.rs` module rustdoc for the
// full R1-R7 invariant list.
use crate::command::rename::RenameHandler;
// TASK-AGS-POST-6-BODIES-B18-RECALL: real /recall handler lives in
// `crate::command::recall`. DIRECT-sync-via-MemoryTrait body-migrate —
// `archon_memory::MemoryTrait::recall_memories(query, limit)` is a
// plain sync method on the object-safe trait and
// `CommandContext::memory: Option<Arc<dyn MemoryTrait>>` is already
// populated unconditionally by `build_command_context` per the
// AGS-817 /memory precedent (context.rs:69 —
// `memory: Some(Arc::clone(&slash_ctx.memory))`). No snapshot/effect-
// slot needed, no new CommandContext field added. Shipped stub
// `declare_handler!(RecallHandler, "Recall memories matching a
// query")` at registry.rs:1305 is REPLACED by this import + the
// insert_primary call at registry.rs:1363 below. No aliases — Steven
// directive at registry.rs:1302-1304 forbids aliasing `recall` onto
// any other handler. R7 double-fire: the legacy match arm at
// slash.rs:569-615 stays live through Gates 1-4; Gate 5 deletes it
// in a separate subagent run. See `src/command/recall.rs` module
// rustdoc for the full R1-R7 invariant list.
use crate::command::recall::RecallHandler;
// TASK-AGS-POST-6-BODIES-B21-CHECKPOINT: real /checkpoint handler lives
// in `crate::command::checkpoint`. DIRECT-pattern body-migrate (not
// EFFECT-SLOT as the B21 task tag suggested — recon proved every
// touched archon-session entry point is sync:
// `CheckpointStore::open`, `list_modified`, and `restore` are plain
// `fn`). Reuses the AGS-815 unconditional
// `CommandContext::session_id: Option<String>` field — no new
// context.rs wiring, no snapshot/effect-slot, no new CommandContext
// field added. Shipped stub `declare_handler!(CheckpointHandler,
// "Create or restore a session checkpoint")` at registry.rs:1361 is
// REPLACED by this import + the insert_primary call at registry.rs:1467
// below. No aliases — shipped stub used the 2-arg declare_handler!
// form and spec lists none. R7 double-fire: legacy match arm at
// slash.rs:452-527 stays live through Gates 1-4; Gate 5 deletes it
// in a separate subagent run. See `src/command/checkpoint.rs` module
// rustdoc for the full R1-R7 invariant list, and the B17 /rename
// precedent at `src/command/rename.rs`.
use crate::command::checkpoint::CheckpointHandler;
// TASK-AGS-POST-6-BODIES-B24-COMPACT-CLEAR: real /clear handler lives
// in `crate::command::clear` (THIN-WRAPPER body-migrate — byte-identical
// no-op `Ok(())` with zero emissions, matching the shipped
// `declare_handler!` stub at registry.rs:1208 pre-B24). Real clear
// behavior lives UPSTREAM at `src/session.rs:2257`, which intercepts
// `/clear` before the dispatcher runs because it needs
// `agent.lock().await` for `clear_conversation` + personality snapshot.
// Real body-migrate deferred to POST-STAGE-6 (same deferral pattern as
// AGS-818 /export). Aliases `&["cls"]` PRESERVED per AGS-817 /memory
// and AGS-818 /export shipped-wins precedent. See `src/command/clear.rs`
// module rustdoc for full R1..R4 invariant list.
use crate::command::clear::ClearHandler;
// TASK-AGS-POST-6-BODIES-B24-COMPACT-CLEAR: real /compact handler lives
// in `crate::command::compact` (THIN-WRAPPER body-migrate — byte-
// identical no-op `Ok(())` with zero emissions, matching the shipped
// `declare_handler!` stub at registry.rs:1207 pre-B24). Real compact
// behavior lives UPSTREAM at `src/session.rs:2241`, which intercepts
// `/compact` (and `/compact <sub>`) before the dispatcher runs because
// it needs `agent.lock().await` for the async `Agent::compact(..)` call.
// Real body-migrate deferred to POST-STAGE-6 (same deferral pattern as
// AGS-818 /export). No aliases — shipped stub used the two-arg
// declare_handler! form. See `src/command/compact.rs` module rustdoc
// for full R1..R4 invariant list.
use crate::command::compact::CompactHandler;
// TASK-AGS-POST-6-BODIES-B19-RULES: real /rules handler lives in
// `crate::command::rules`. DIRECT-sync-via-MemoryTrait body-migrate
// (not SNAPSHOT as the B19 task tag suggests — recon proved
// `archon_consciousness::rules::RulesEngine::new(&dyn MemoryTrait)`
// and every method exercised by /rules (`get_rules_sorted`,
// `update_rule`, `remove_rule`) are sync on the object-safe
// `MemoryTrait`). `CommandContext::memory: Option<Arc<dyn
// MemoryTrait>>` is already populated unconditionally by
// `build_command_context` per the AGS-817 /memory precedent
// (context.rs:69 — `memory: Some(Arc::clone(&slash_ctx.memory))`),
// so no new context.rs wiring, no snapshot/effect-slot, no new
// CommandContext field added. Shipped stub `declare_handler!(
// RulesHandler, "List, edit, or remove behavioral rules")` at
// registry.rs:1336 is REPLACED by this import + the insert_primary
// call at registry.rs:1394 below. No aliases — shipped stub used
// the 2-arg declare_handler! form. R7 double-fire: the legacy
// match arm at slash.rs:591-706 stays live through Gates 1-4;
// Gate 5 deletes it in a separate subagent run. See
// `src/command/rules.rs` module rustdoc for the full R1-R7
// invariant list, and the B18 /recall precedent at
// `src/command/recall.rs`.
use crate::command::rules::RulesHandler;
// TASK-AGS-POST-6-BODIES-B22-LOGIN: real /login slash-command handler
// lives in `crate::command::login` (DIRECT body-migrate — sync
// `dirs::home_dir()` / `.exists()` filesystem probe + string-format +
// single `ctx.tui_tx.try_send(TuiEvent::TextDelta(msg))` emission; no
// snapshot, no effect slot). The handler reads a new
// `CommandContext::auth_label: Option<String>` populated UNCONDITIONALLY
// by `build_command_context` (AGS-815 / AGS-817 / B20 cross-cutting
// precedent). Shipped stub `declare_handler!(LoginHandler, "Authenticate
// against the configured backend")` at registry.rs:1295 is REPLACED by
// this import + the breadcrumb comment at the old site. No aliases —
// shipped stub used the 2-arg declare_handler! form. R7 double-fire:
// the legacy match arm at slash.rs:285-309 stays live through Gates
// 1-4; Gate 5 deletes it in a separate subagent run. Note: the pre-
// existing `handle_login` async fn in `crate::command::login` (TUI-325
// OAuth CLI entry point) is unrelated and untouched.
use crate::command::login::LoginHandler;
// TASK-AGS-POST-6-BODIES-B23-LOGOUT: real /logout handler lives in
// `crate::command::logout` (real impl with body-migrated execute via
// DIRECT pattern — sync filesystem probe `dirs::home_dir().join(".archon")
// .join(".credentials.json").exists()` + sync `std::fs::remove_file` +
// three branches emitting `TuiEvent` via `ctx.tui_tx.try_send(..)`
// replaces the `tui_tx.send(..).await` calls that lived in the legacy
// slash.rs:365-392 match arm). Shipped stub `declare_handler!(
// LogoutHandler, "Clear stored credentials")` at registry.rs:1393 is
// REPLACED by this import + the breadcrumb comment at the old site.
// No aliases — shipped stub used the 2-arg declare_handler! form (no
// aliases slice) and spec lists none. NO new CommandContext field
// required — /logout reads no cross-cutting state; filesystem probe
// resolves against `dirs::home_dir()` inline. R7 double-fire: the
// legacy match arm at slash.rs:365-392 stays live through Gates 1-4;
// Gate 5 deletes it in a separate parent-context run. See
// .gates/TASK-AGS-POST-6-BODIES-B23-LOGOUT/ for the full gate trail.
use crate::command::logout::LogoutHandler;
// TASK-AGS-POST-6-BODIES-B08-DENIALS: real /denials handler lives in
// `crate::command::denials`. SNAPSHOT-ONLY body-migrate (async
// `denial_log.lock().await` + sync `DenialLog::format_display(20)` move
// to the builder; no effect slot required — /denials is read-only).
// The handler reads `Option<DenialSnapshot>` from a new
// `CommandContext::denial_snapshot` field populated BY build_command_context
// ONLY when the primary resolves to `/denials` (mirrors AGS-807 status,
// AGS-809 cost, AGS-811 mcp, AGS-814 context SNAPSHOT precedent —
// not the unconditional DIRECT-field pattern of B01-FAST etc.).
// EIGHTH Batch-A body-migrate (after B01-FAST, B02-THINKING, B03-BUG,
// B04-DIFF, B05-VIM, B06-HELP, B07-RELEASE-NOTES). Shipped stub
// `declare_handler!(DenialsHandler, "List tool-use denials recorded
// this session")` at registry.rs:786 is REPLACED by this import + the
// insert_primary call below. No aliases — shipped stub used the
// two-arg declare_handler! form (no aliases slice) and spec lists none.
use crate::command::denials::{DenialSnapshot, DenialsHandler};
// TASK-AGS-POST-6-BODIES-B09-COLOR: real /color handler lives in
// `crate::command::color`. DIRECT-pattern body-migrate (sync
// `archon_tui::theme::parse_color` — plain `fn` match on the arg
// string; no snapshot/effect-slot needed, no new CommandContext field
// added). Mirrors AGS-819 /theme precedent exactly. Shipped stub
// `declare_handler!(ColorHandler, "Show or change the UI color scheme")`
// at registry.rs:874 is REPLACED by this import + the insert_primary
// call below. No aliases — shipped stub had none and AGS-817 shipped-
// wins rule preserves zero aliases. Accent-color mutation is signalled
// via `TuiEvent::SetAccentColor(Color)` to the TUI event loop; the
// handler does NOT write to SlashCommandContext, so no `CommandEffect`
// variant is required (mirrors AGS-819 /theme's DIRECT pattern).
use crate::command::color::ColorHandler;
// TASK-AGS-819: real /theme handler lives in `crate::command::theme`.
// DIRECT-pattern body-migrate (sync theme helpers — `theme_by_name` +
// `available_themes` are both plain `fn` lookups; no snapshot/effect-
// slot needed, no new CommandContext field added). FIFTH Batch-3
// ticket. Shipped stub `declare_handler!(ThemeHandler, "Show or
// change the UI theme")` at registry.rs:607 is REPLACED by this
// import + the insert_primary call below. No aliases — shipped stub
// had none and AGS-817 shipped-wins rule preserves zero aliases.
// Theme mutation is signalled via `TuiEvent::SetTheme(name)` to the
// TUI event loop; the handler does NOT write to SlashCommandContext,
// so no `CommandEffect` variant is required (see registry.rs:272 NOTE
// — the speculative AGS-819 "write" extension turned out to be DIRECT
// pattern, not effect-slot).
use crate::command::theme::ThemeHandler;
// TASK-AGS-POST-6-BODIES-B10-ADDDIR: real /add-dir handler lives in
// `crate::command::add_dir`. EFFECT-SLOT body-migrate (Path B
// reclassification — shipped body at slash.rs:679 contains
// `ctx.extra_dirs.lock().await.push(..)` on a `tokio::sync::Mutex`,
// forcing the deferred async mutation via
// `CommandEffect::AddExtraDir(PathBuf)`). Shipped stub
// `declare_handler!(AddDirHandler, "Add a directory to the working
// context")` at registry.rs:886 is REPLACED by this import + the
// insert_primary call below. No aliases (shipped stub had none; AGS-817
// shipped-wins rule preserves zero aliases). `apply_effect` in
// `src/command/context.rs` awaits the push and emits the tracing::info!
// record byte-identical to shipped slash.rs:679 + 683.
use crate::command::add_dir::AddDirHandler;
// TASK-AGS-POST-6-BODIES-B11-EFFORT: real /effort handler lives in
// `crate::command::effort`. HYBRID body-migrate (SNAPSHOT + EFFECT-SLOT
// + SIDECAR) — the shipped body at slash.rs:92-122 performs THREE
// actions that cannot all run inside a sync `CommandHandler::execute`:
//
//   1. Read `effort_state.level()` — SNAPSHOT pattern (builder
//      pre-populates `CommandContext::effort_snapshot` by awaiting
//      `slash_ctx.effort_level_shared.lock().await`).
//   2. Write `*ctx.effort_level_shared.lock().await = level` —
//      EFFECT-SLOT pattern via new `CommandEffect::SetEffortLevelShared`
//      variant; `apply_effect` awaits the mutex write.
//   3. Write `effort_state.set_level(level)` on the session-local
//      `&mut EffortState` — NEW SIDECAR slot
//      (`CommandContext::pending_effort_set`) drained at the slash.rs
//      dispatch site where the `&mut effort_state` parameter is in
//      scope.
//
// Shipped stub `declare_handler!(EffortHandler, "Show or set reasoning
// effort (high|medium|low)")` at registry.rs:801 is REPLACED by this
// import + the insert_primary call below. No aliases (shipped stub had
// none; AGS-817 shipped-wins rule preserves zero aliases). Mirrors
// AGS-808 /model snapshot/effect-slot pattern and B10-ADDDIR's
// effect-slot mutex-write deferral; the sidecar is new in B11.
use crate::command::effort::EffortHandler;
// TASK-AGS-POST-6-BODIES-B12-PERMISSIONS: real /permissions handler lives
// in `crate::command::permissions`. HYBRID body-migrate (SNAPSHOT +
// EFFECT-SLOT, NO sidecar) — the shipped body at slash.rs:295-336
// performs THREE actions that cannot all run inside a sync
// `CommandHandler::execute`:
//
//   1. Async read `ctx.permission_mode.lock().await` — SNAPSHOT pattern
//      (builder pre-populates `CommandContext::permissions_snapshot` by
//      awaiting `slash_ctx.permission_mode.lock().await` and also
//      capturing the sync `bool allow_bypass_permissions`).
//   2. Sync read `ctx.allow_bypass_permissions` — bundled into the
//      snapshot above to minimise the extension surface (one snapshot
//      field rather than a DIRECT-pattern cross-cutting field).
//   3. Async write `*ctx.permission_mode.lock().await = resolved` plus
//      `TuiEvent::PermissionModeChanged(resolved)` emission —
//      EFFECT-SLOT pattern via new `CommandEffect::SetPermissionMode`
//      variant; `apply_effect` awaits the mutex write AND sends the
//      state-change event (using `.send().await` since apply_effect is
//      already async — the event MUST be awaited to match shipped
//      emission-after-write ordering).
//
// Shipped stub `declare_handler!(PermissionsHandler, "Show or update
// tool permissions")` at registry.rs:914 is REPLACED by this import +
// the insert_primary call below. No aliases (shipped stub used the
// two-arg form; AGS-817 shipped-wins rule preserves zero aliases).
// Mirrors AGS-808 /model snapshot/effect-slot pattern and B11-EFFORT's
// HYBRID split (minus the sidecar, since /permissions has no session-
// local stack state to mutate).
use crate::command::permissions::PermissionsHandler;
// TASK-AGS-POST-6-BODIES-B13-GARDEN: real /garden handler lives in
// `crate::command::garden`. DIRECT-sync-via-MemoryTrait body-migrate —
// both `archon_memory::garden::format_garden_stats(&dyn MemoryTrait,
// usize)` and `archon_memory::garden::consolidate(&dyn MemoryTrait,
// &GardenConfig)` are fully SYNC (no async fn, no .await), matching the
// AGS-817 /memory DIRECT-pattern precedent. The handler reads
// `Arc<dyn MemoryTrait>` from the existing `CommandContext::memory`
// field (added in AGS-817) and the new `CommandContext::garden_config`
// field populated UNCONDITIONALLY by `build_command_context` (mirrors
// the AGS-817 `memory` cross-cutting precedent — not gated on the
// primary name). Shipped stub `declare_handler!(GardenHandler, "Run
// memory garden consolidation or show stats")` at registry.rs:958 is
// REPLACED by this import + the insert_primary call below. No aliases
// (shipped stub used the two-arg declare_handler! form; AGS-817
// shipped-wins rule preserves zero aliases).
use crate::command::garden::GardenHandler;
// TASK-AGS-POST-6-BODIES-B14-COPY: real /copy handler lives in
// `crate::command::copy`. SNAPSHOT-pattern body-migrate — the handler
// reads `ctx.copy_snapshot: Option<CopySnapshot>` (pre-captured clone
// of `slash_ctx.last_assistant_response.lock().await`) and delegates
// subprocess work to an internal `ClipboardRunner` trait (production
// `SystemClipboardRunner` preserves shipped xclip/clip.exe/pbcopy
// detection + spawn byte-for-byte; tests inject `MockClipboardRunner`).
// Shipped stub `declare_handler!(CopyHandler, "Copy the last assistant
// message to the clipboard")` at registry.rs:1014 is REPLACED by this
// import + the insert_primary call below. No aliases (shipped stub
// used the two-arg declare_handler! form; shipped-wins rule preserves
// zero aliases).
use crate::command::copy::CopyHandler;
// TASK-AGS-POST-6-BODIES-B16-USAGE: real /usage handler lives in
// `crate::command::usage`. SNAPSHOT-pattern body-migrate (single
// `session_stats.lock().await` moves to the builder; no effect slot
// required — /usage is read-only). The handler reads
// `Option<UsageSnapshot>` from a new `CommandContext::usage_snapshot`
// field populated BY `build_command_context` ONLY when the primary
// resolves to `/usage` (mirrors AGS-807 status, AGS-809 cost, AGS-811
// mcp, AGS-814 context, B08 denials, B15 doctor SNAPSHOT precedent —
// not the unconditional DIRECT-field pattern of B01-FAST etc.).
// Shipped stub `declare_handler!(UsageHandler, "Show aggregate API
// usage for the session")` at registry.rs:1166 is REPLACED by this
// import + the insert_primary call below. No aliases — shipped stub
// used the two-arg declare_handler! form (no aliases slice) and spec
// lists none. /usage is the reason /cost (AGS-809) cannot register
// `usage` as an alias: /usage is already a shipped primary.
use crate::command::usage::{UsageHandler, UsageSnapshot};
// TASK-AGS-POST-6-BODIES-B15-DOCTOR: real /doctor handler lives in
// `crate::command::doctor`. SNAPSHOT-DELEGATE body-migrate — the
// shipped delegate `handle_doctor_command` at src/command/doctor.rs
// already composed the diagnostic text asynchronously; this ticket
// extracts the composition into `build_doctor_text(&SlashCommandContext)
// -> String` and wires a new `build_doctor_snapshot` into
// `build_command_context`. The sync handler reads
// `ctx.doctor_snapshot: Option<DoctorSnapshot>` and emits the composed
// text as a single `TuiEvent::TextDelta` via `try_send`. Mirrors B14
// /copy SNAPSHOT precedent (the pre-existing `handle_doctor_command`
// delegate stays live during the Gates 1-4 double-fire window per R7;
// Gate 5 removes the legacy match arm at slash.rs:230-234). Shipped
// stub `declare_handler!(DoctorHandler, "Run environment health checks")`
// at registry.rs:1095 is REPLACED by this import + the insert_primary
// call below. No aliases (shipped stub used the two-arg
// declare_handler! form; shipped-wins rule preserves zero aliases).
use crate::command::doctor::DoctorHandler;

// TASK-AGS-POST-6-NO-STUB: ConfigHandler moved to
// `crate::command::config::ConfigHandler` as a THIN-WRAPPER (same
// pattern as B24 /compact, /clear). Real async work still lives at
// `src/command/slash.rs:247`. Byte-identical no-op `Ok(())` with
// zero emissions. Aliases `&["settings", "prefs"]` PRESERVED per
// AGS-813 + AGS-817 shipped-wins.
use crate::command::config::ConfigHandler;

// TASK-AGS-POST-6-NO-STUB: CancelHandler moved to
// `crate::command::cancel::CancelHandler` as a THIN-WRAPPER (same
// pattern as B24 /compact, /clear). Byte-identical no-op `Ok(())`
// with zero emissions. Aliases `&["stop", "abort"]` PRESERVED per
// AGS-805 + AGS-817 shipped-wins. Silent-no-op UX gap tracked by
// ticket #91 POST-6-CANCEL-AUDIT (separate follow-up).
use crate::command::cancel::CancelHandler;

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
    /// TASK-AGS-817 DIRECT-pattern field (/memory).
    ///
    /// Shared memory handle for `/memory` (DIRECT pattern). `Arc` clone
    /// per dispatch is cheap (~8 bytes + atomic refcount increment).
    /// Populated UNCONDITIONALLY in context.rs outer builder literal
    /// (mirrors the AGS-815 `session_id` cross-cutting precedent, not
    /// gated on the primary name). `None` sentinel reserved for test
    /// fixtures that construct `CommandContext` directly without
    /// standing up a full `SlashCommandContext`; in those tests the
    /// handler observes `None` and returns an Err-with-message
    /// describing the missing-memory condition rather than panicking.
    /// `archon_memory::MemoryTrait` is fully sync (all 12 trait methods
    /// are plain `fn`) so no matching `CommandEffect` variant is
    /// required — `/memory clear` performs the `clear_all()` mutation
    /// via a direct sync call inside `execute`, not an async write-back.
    pub(crate) memory: Option<Arc<dyn archon_memory::MemoryTrait>>,
    /// TASK-AGS-POST-6-BODIES-B13-GARDEN DIRECT-pattern field (/garden).
    ///
    /// Clone of `SlashCommandContext::garden_config` populated
    /// UNCONDITIONALLY by `build_command_context` (mirrors the AGS-815
    /// `session_id` and AGS-817 `memory` cross-cutting precedent — not
    /// gated on the primary name). `/garden` (default branch) reads it
    /// to pass `&GardenConfig` into the sync
    /// `archon_memory::garden::consolidate(&dyn MemoryTrait,
    /// &GardenConfig)` entry point; `/garden stats` does not read it.
    /// `GardenConfig` derives `Clone` so cloning it per dispatch is
    /// cheap (small fixed-size struct of numeric thresholds — no Arc,
    /// no heap allocation beyond the struct itself). `None` sentinel
    /// reserved for test fixtures that construct `CommandContext`
    /// directly without standing up a full `SlashCommandContext`; in
    /// those tests the consolidate branch observes `None` and returns
    /// an Err-with-message describing the missing-config condition
    /// rather than panicking. Both `format_garden_stats` and
    /// `consolidate` are fully sync (all 12 MemoryTrait methods are
    /// plain `fn`) so no matching `CommandEffect` variant is required.
    pub(crate) garden_config:
        Option<archon_memory::garden::GardenConfig>,
    /// TASK-AGS-POST-6-BODIES-B01-FAST DIRECT-pattern field (/fast).
    ///
    /// Clone of `SlashCommandContext::fast_mode_shared` populated
    /// UNCONDITIONALLY by `build_command_context` (mirrors the AGS-815
    /// `session_id` and AGS-817 `memory` cross-cutting precedent — not
    /// gated on the primary name). `/fast` reads and atomically mutates
    /// it. `Option<Arc<AtomicBool>>` so the handler test fixtures can
    /// construct a `CommandContext` without standing up a full
    /// `SlashCommandContext`; when `None` the handler returns an
    /// Err-with-message describing the missing-shared-state condition
    /// rather than panicking. No matching `CommandEffect` variant — the
    /// mutation is a sync atomic store.
    pub(crate) fast_mode_shared: Option<Arc<AtomicBool>>,
    /// TASK-AGS-POST-6-BODIES-B02-THINKING DIRECT-pattern field
    /// (/thinking).
    ///
    /// Clone of `SlashCommandContext::show_thinking` populated
    /// UNCONDITIONALLY by `build_command_context` (mirrors the AGS-815
    /// `session_id`, AGS-817 `memory`, and B01-FAST `fast_mode_shared`
    /// cross-cutting precedent — not gated on the primary name).
    /// `/thinking` reads it (to log a no-op?) and atomically stores
    /// the new state from the parsed `on`/`off`/empty subcommand.
    /// `Option<Arc<AtomicBool>>` so the handler test fixtures can
    /// construct a `CommandContext` without standing up a full
    /// `SlashCommandContext`; when `None` the handler returns an
    /// Err-with-message describing the missing-shared-state condition
    /// rather than panicking. No matching `CommandEffect` variant — the
    /// mutation is a sync atomic store.
    pub(crate) show_thinking: Option<Arc<AtomicBool>>,
    /// TASK-AGS-POST-6-BODIES-B04-DIFF DIRECT-with-effect-pattern field
    /// (/diff).
    ///
    /// Clone of `SlashCommandContext::working_dir` populated
    /// UNCONDITIONALLY by `build_command_context` (mirrors the AGS-815
    /// `session_id`, AGS-817 `memory`, B01-FAST `fast_mode_shared`, and
    /// B02-THINKING `show_thinking` cross-cutting precedent — not gated
    /// on the primary name). `/diff` reads it to produce the
    /// `CommandEffect::RunGitDiffStat(PathBuf)` effect; apply_effect
    /// passes the cloned path to `crate::command::slash::handle_diff_command`
    /// which spawns `git diff --stat` via `tokio::process::Command`.
    /// `Option<PathBuf>` so the handler test fixtures can construct a
    /// `CommandContext` without standing up a full `SlashCommandContext`;
    /// when `None` the handler emits a `TuiEvent::Error` describing the
    /// missing-shared-state condition (mirroring B01-FAST's
    /// fast_mode_shared=None Err-with-message pattern, adapted for an
    /// event emission path since /diff must stay Ok(()) to keep the
    /// dispatcher contract uniform).
    pub(crate) working_dir: Option<PathBuf>,
    /// TASK-AGS-POST-6-BODIES-B06-HELP DIRECT-pattern field (/help).
    ///
    /// Clone of `SlashCommandContext::skill_registry` populated
    /// UNCONDITIONALLY by `build_command_context` (mirrors the AGS-815
    /// `session_id`, AGS-817 `memory`, B01-FAST `fast_mode_shared`,
    /// B02-THINKING `show_thinking`, and B04-DIFF `working_dir` cross-
    /// cutting precedent — not gated on the primary name). `/help` reads
    /// it to call the sync `SkillRegistry::format_help()` /
    /// `format_skill_help()` methods for the extended-commands suffix
    /// and per-skill detail output.
    ///
    /// `Option<Arc<SkillRegistry>>` so the handler test fixtures can
    /// construct a `CommandContext` without standing up a full
    /// `SlashCommandContext`; when `None` the handler still emits the
    /// static core-commands header (it does not depend on the registry)
    /// but skips the skill-registry-sourced suffix, and the single-
    /// command arg path falls through to the unknown-command Error
    /// branch. Production always populates
    /// `Some(Arc::clone(&slash_ctx.skill_registry))`.
    ///
    /// No matching `CommandEffect` variant — `/help` is a pure DIRECT-
    /// pattern read (no async mutex writes back to shared state).
    pub(crate) skill_registry:
        Option<Arc<archon_core::skills::SkillRegistry>>,
    /// TASK-AGS-POST-6-BODIES-B08-DENIALS SNAPSHOT-pattern field
    /// (READ-only /denials).
    ///
    /// Populated by `build_command_context` for `/denials` ONLY (no
    /// aliases — the shipped stub at registry.rs:786 used the two-arg
    /// declare_handler! form). Every other command observes `None`
    /// and pays zero additional lock traffic on
    /// `SlashCommandContext::denial_log`. Per the AGS-822 Rule 5
    /// extension pattern: each body-migrate ticket that needs an
    /// async-locked snapshot appends one typed snapshot field —
    /// /denials is READ-only so there is NO matching `CommandEffect`
    /// variant (mirrors AGS-811 /mcp and AGS-814 /context).
    pub(crate) denial_snapshot: Option<DenialSnapshot>,
    /// TASK-AGS-POST-6-BODIES-B11-EFFORT SNAPSHOT-pattern field (READ
    /// side of /effort).
    ///
    /// Populated by `build_command_context` for `/effort` ONLY (no
    /// aliases — shipped stub at registry.rs:801 used the two-arg
    /// declare_handler! form). Every other command observes `None`
    /// and pays zero additional lock traffic on
    /// `SlashCommandContext::effort_level_shared`. Mirrors AGS-807
    /// `status_snapshot`, AGS-808 `model_snapshot`, and B08
    /// `denial_snapshot` convention. Carries an owned `EffortLevel`
    /// (Copy) so the sync handler reads without any lock.
    pub(crate) effort_snapshot: Option<crate::command::effort::EffortSnapshot>,
    /// TASK-AGS-POST-6-BODIES-B12-PERMISSIONS SNAPSHOT-pattern field
    /// (HYBRID — READ side + bypass-allow guard for /permissions).
    ///
    /// Populated by `build_command_context` for `/permissions` ONLY (no
    /// aliases — shipped stub at registry.rs:914 used the two-arg
    /// declare_handler! form). Every other command observes `None` and
    /// pays zero additional lock traffic on `permission_mode`. Carries
    /// BOTH `current_mode: String` (captured via
    /// `slash_ctx.permission_mode.lock().await`) AND
    /// `allow_bypass_permissions: bool` (copied from the sync field on
    /// `SlashCommandContext`). Bundling both into one snapshot
    /// minimises the extension surface — one snapshot per primary, no
    /// second DIRECT-pattern cross-cutting field. Mirrors AGS-807
    /// `status_snapshot`, AGS-808 `model_snapshot`, B08 `denial_snapshot`,
    /// and B11 `effort_snapshot` snapshot gating rule.
    pub(crate) permissions_snapshot:
        Option<crate::command::permissions::PermissionsSnapshot>,
    /// TASK-AGS-POST-6-BODIES-B14-COPY SNAPSHOT-pattern field (READ-only
    /// /copy).
    ///
    /// Populated by `build_command_context` for `/copy` ONLY (no
    /// aliases — shipped stub at registry.rs:1014 used the two-arg
    /// declare_handler! form). Every other command observes `None`
    /// and pays zero additional lock traffic on
    /// `SlashCommandContext::last_assistant_response`. Mirrors AGS-807
    /// `status_snapshot`, AGS-808 `model_snapshot`, B08 `denial_snapshot`,
    /// B11 `effort_snapshot`, and B12 `permissions_snapshot` gating rule.
    /// Carries an owned `String` (clone of the current last-assistant-
    /// response) so the sync handler reads without holding a lock across
    /// the potentially-blocking clipboard subprocess spawn.
    pub(crate) copy_snapshot: Option<crate::command::copy::CopySnapshot>,
    /// TASK-AGS-POST-6-BODIES-B15-DOCTOR SNAPSHOT-DELEGATE field
    /// (READ-only /doctor).
    ///
    /// Populated by `build_command_context` for `/doctor` ONLY (no
    /// aliases — shipped stub at registry.rs:1095 used the two-arg
    /// declare_handler! form). Every other command observes `None` and
    /// pays zero additional lock traffic on
    /// `SlashCommandContext::mcp_manager` +
    /// `SlashCommandContext::model_override_shared`. Mirrors AGS-807
    /// `status_snapshot`, AGS-808 `model_snapshot`, B08 `denial_snapshot`,
    /// B11 `effort_snapshot`, B12 `permissions_snapshot`, and B14
    /// `copy_snapshot` gating rule. Carries a single owned `String` —
    /// the fully-composed diagnostic text produced by
    /// `doctor::build_doctor_text` — so the sync handler emits via
    /// `try_send` without holding any lock.
    pub(crate) doctor_snapshot: Option<crate::command::doctor::DoctorSnapshot>,
    /// TASK-AGS-POST-6-BODIES-B16-USAGE SNAPSHOT-pattern field
    /// (READ-only /usage).
    ///
    /// Populated by `build_command_context` for `/usage` ONLY (no
    /// aliases — shipped stub at registry.rs:1166 used the two-arg
    /// declare_handler! form). Every other command observes `None` and
    /// pays zero additional lock traffic on
    /// `SlashCommandContext::session_stats`. Mirrors AGS-807
    /// `status_snapshot`, AGS-809 `cost_snapshot`, B08 `denial_snapshot`,
    /// B11 `effort_snapshot`, B12 `permissions_snapshot`, B14
    /// `copy_snapshot`, and B15 `doctor_snapshot` gating rule. Carries
    /// owned scalar fields (input/output tokens, turn count, pre-
    /// computed costs) plus an owned `cache_stats_line: String` so the
    /// sync handler emits via `try_send` without holding any lock.
    pub(crate) usage_snapshot: Option<UsageSnapshot>,
    /// TASK-AGS-POST-6-BODIES-B20-RELOAD DIRECT-pattern field (/reload).
    ///
    /// Clone of `SlashCommandContext::config_path` populated
    /// UNCONDITIONALLY by `build_command_context` (mirrors the AGS-815
    /// `session_id`, AGS-817 `memory`, B01-FAST `fast_mode_shared`,
    /// B02-THINKING `show_thinking`, B04-DIFF `working_dir`, B06-HELP
    /// `skill_registry`, and B13-GARDEN `garden_config` cross-cutting
    /// precedent — not gated on the primary name). `/reload` reads it
    /// to pass `&[PathBuf]` into the sync
    /// `archon_core::config_watcher::force_reload(config_paths:
    /// &[PathBuf], current: &ArchonConfig)` entry point via
    /// `std::slice::from_ref(config_path)`. `PathBuf` clone per
    /// dispatch is cheap (one Vec<u8> alloc). `None` sentinel reserved
    /// for test fixtures that construct `CommandContext` directly
    /// without standing up a full `SlashCommandContext`; in those
    /// tests the handler observes `None` and returns an
    /// Err-with-message describing the missing-config_path condition
    /// rather than panicking. `archon_core::config_watcher::force_reload`
    /// is fully sync (no `async fn`, no `.await`) so no matching
    /// `CommandEffect` variant is required — `/reload` performs its
    /// read-and-diff synchronously inside `execute`.
    pub(crate) config_path: Option<std::path::PathBuf>,
    /// TASK-AGS-POST-6-BODIES-B22-LOGIN DIRECT-pattern field (/login).
    ///
    /// Clone of `SlashCommandContext::auth_label` populated
    /// UNCONDITIONALLY by `build_command_context` (mirrors the AGS-815
    /// `session_id`, AGS-817 `memory`, B01-FAST `fast_mode_shared`,
    /// B02-THINKING `show_thinking`, B04-DIFF `working_dir`, B06-HELP
    /// `skill_registry`, B13-GARDEN `garden_config`, and B20-RELOAD
    /// `config_path` cross-cutting precedent — not gated on the
    /// primary name). `/login` reads it to include the active auth
    /// method in the emitted `TuiEvent::TextDelta` message (see
    /// `crate::command::login::LoginHandler::execute`). `String` clone
    /// per dispatch is cheap (one heap alloc). `None` sentinel
    /// reserved for test fixtures that construct `CommandContext`
    /// directly without standing up a full `SlashCommandContext`; in
    /// those tests the handler observes `None` and returns an
    /// Err-with-message describing the missing-auth_label condition
    /// rather than panicking. Matches the AGS-815
    /// `fork_handler_execute_without_session_id_returns_err` and B20
    /// `execute_without_config_path_returns_err` pattern. No matching
    /// `CommandEffect` variant — `/login` is a pure DIRECT-pattern
    /// read (no async mutex writes back to shared state).
    pub(crate) auth_label: Option<String>,
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
    /// TASK-AGS-POST-6-BODIES-B11-EFFORT SIDECAR field (LOCAL write
    /// side of /effort).
    ///
    /// The /effort body-migrate is the first handler that needs to
    /// mutate session-local stack state (`&mut EffortState`) in
    /// addition to the shared-mutex state covered by the effect-slot.
    /// `pending_effect` handles the shared-mutex write via
    /// `CommandEffect::SetEffortLevelShared`; this sidecar slot
    /// carries the same `EffortLevel` to the slash.rs dispatch site,
    /// where the caller's `&mut effort_state` parameter is drained
    /// AFTER `apply_effect` returns by calling
    /// `effort_state.set_level(level)`. Single-shot (`.take()` on
    /// drain) just like `pending_effect`.
    ///
    /// Initialised to `None` by `build_command_context`; populated in
    /// lockstep with `pending_effect` by `EffortHandler::execute` so
    /// the two slots are never out of sync.
    pub(crate) pending_effort_set: Option<archon_llm::effort::EffortLevel>,
    /// TASK-AGS-POST-6-EXPORT-MIGRATE SIDECAR-SLOT field (WRITE side of
    /// /export; drain lives in session.rs).
    ///
    /// `Option<Arc<std::sync::Mutex<Option<ExportDescriptor>>>>`.
    /// Populated UNCONDITIONALLY by `build_command_context` (cloned
    /// `Arc` from `SlashCommandContext::pending_export_shared`). The
    /// outer `Option` is `None` only in test fixtures that construct
    /// `CommandContext` directly without a full `SlashCommandContext`;
    /// in that case `ExportHandler::execute` emits a wiring-regression
    /// error instead of stashing the descriptor. Production always
    /// observes `Some(Arc::clone(...))`.
    ///
    /// The handler writes `*slot.lock().unwrap() = Some(desc)` synchronously;
    /// session.rs's drain block (inside the `if handled {` branch of
    /// the input-processor task) calls `.take()` on the same shared
    /// `std::sync::Mutex`, obtains the `Agent` mutex guard via
    /// `agent.lock().await`, reads `conversation_state().messages`,
    /// and runs the file-write I/O. `std::sync::Mutex` (not
    /// `tokio::sync::Mutex`) because the handler is sync and the drain
    /// site holds the lock only across a single `.take()` call —
    /// zero `.await` is held across either lock acquisition.
    ///
    /// Why NOT a plain `Option<ExportDescriptor>` like
    /// `pending_effort_set`: the drain must run in session.rs where
    /// `__cmd_ctx` (the dispatch-local `CommandContext`) does not
    /// exist. A shared `Arc<Mutex<...>>` is the one mechanism that
    /// lets the sync handler write and session.rs drain without
    /// forcing an edit to `slash.rs` (which is a hard scope guard for
    /// this ticket).
    pub(crate) pending_export: Option<
        Arc<std::sync::Mutex<Option<crate::command::export::ExportDescriptor>>>,
    >,
}

// TASK-AGS-POST-6-TRY-SEND: wraps `tui_tx.try_send` at every handler
// call site so the copy-paste `let _ = ctx.tui_tx.try_send(...)` pattern
// collapses to `ctx.emit(...)` while also distinguishing `Full` (warn —
// benign backpressure on the 256-slot prod buffer) from `Closed`
// (error — the TUI receiver task has died). Happy-path delivery is
// byte-identical to the shipped `let _ = try_send(...)` behavior.
impl CommandContext {
    /// Deliver a [`TuiEvent`] to the TUI channel. Logs on failure;
    /// never panics.
    ///
    /// Semantics match [`tokio::sync::mpsc::Sender::try_send`]: success
    /// is best-effort delivery to a 256-slot bounded channel (prod
    /// buffer in `session.rs`). NO retry, NO `.await` — this is called
    /// from sync [`CommandHandler::execute`] bodies where any blocking
    /// wait would deadlock the tokio runtime that owns the calling
    /// task. [`TrySendError::Full`](tokio::sync::mpsc::error::TrySendError::Full)
    /// emits a `tracing::warn!` and drops the event (matches the
    /// shipped silent-drop); [`TrySendError::Closed`](tokio::sync::mpsc::error::TrySendError::Closed)
    /// emits a `tracing::error!` and drops the event (TUI receiver
    /// task is dead — operator-visible terminal state).
    pub(crate) fn emit(&self, event: TuiEvent) {
        use tokio::sync::mpsc::error::TrySendError;
        match self.tui_tx.try_send(event) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                tracing::warn!(
                    target: "archon_cli::command::tui",
                    "tui_tx full (256-slot buffer saturated) — event dropped"
                );
            }
            Err(TrySendError::Closed(_)) => {
                tracing::error!(
                    target: "archon_cli::command::tui",
                    "tui_tx closed — TUI receiver task is dead"
                );
            }
        }
    }
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
/// /context read-only, AGS-817 /memory sync-trait) may extend this
/// enum with additional variants.
///
/// NOTE (AGS-819): the original speculative list also named
/// "AGS-819 /theme write" as a candidate effect-slot extension, but
/// the actual /theme migration turned out to be DIRECT pattern, NOT
/// effect-slot — `TuiEvent::SetTheme(name)` is the canonical theme-
/// mutation channel (consumed by the TUI event loop), so the handler
/// has no `SlashCommandContext` field to write back. See
/// `src/command/theme.rs` module rustdoc R1 for the full pattern
/// rationale.
///
/// Each new variant should:
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
    /// TASK-AGS-POST-6-BODIES-B04-DIFF: spawn `git diff --stat` against
    /// the supplied working directory. Produced by `DiffHandler::execute`
    /// (sync stash). Applied by `command::context::apply_effect`, which
    /// awaits the subprocess call via the existing LIVE
    /// `crate::command::slash::handle_diff_command(&tui_tx, &path)`
    /// helper. Carries an owned `PathBuf` (clone of
    /// `SlashCommandContext::working_dir`) to avoid any borrow on
    /// `SlashCommandContext` lifetime through the effect-slot.
    RunGitDiffStat(PathBuf),
    /// TASK-AGS-POST-6-BODIES-B10-ADDDIR: push the validated directory
    /// onto `SlashCommandContext::extra_dirs` (Arc<tokio::sync::Mutex<...>>).
    /// Produced by `AddDirHandler::execute` (sync stash). Applied by
    /// `command::context::apply_effect`, which awaits the mutex push and
    /// emits the tracing::info! record. Carries an owned `PathBuf` so
    /// no borrow on `SlashCommandContext` leaks through the effect-slot.
    AddExtraDir(PathBuf),
    /// TASK-AGS-POST-6-BODIES-B11-EFFORT: overwrite
    /// `SlashCommandContext::effort_level_shared` (`Arc<tokio::sync::
    /// Mutex<EffortLevel>>`) with the validated level. Produced by
    /// `EffortHandler::execute` (sync stash). Applied by
    /// `command::context::apply_effect`, which awaits the mutex write.
    /// Carries an owned `EffortLevel` (Copy) so no borrow on
    /// `SlashCommandContext` leaks through the effect-slot.
    ///
    /// HYBRID pattern — the handler ALSO stashes
    /// `CommandContext::pending_effort_set` (SIDECAR) for the session-
    /// local `&mut EffortState` write that the slash.rs dispatch site
    /// performs after `apply_effect` returns. The effect here only
    /// covers the shared-mutex half of the mutation. Byte-identity
    /// claim versus shipped slash.rs:108-109 (paired
    /// `effort_state.set_level(level); *ctx.effort_level_shared.
    /// lock().await = level;`) is preserved jointly by this variant
    /// and the sidecar. Mirrors AGS-808 `SetModelOverride` + B10
    /// `AddExtraDir` effect-slot precedent.
    SetEffortLevelShared(archon_llm::effort::EffortLevel),
    /// TASK-AGS-POST-6-BODIES-B12-PERMISSIONS: overwrite
    /// `SlashCommandContext::permission_mode` (`Arc<tokio::sync::
    /// Mutex<String>>`) with the validated permission mode. Produced by
    /// `PermissionsHandler::execute` (sync stash). Applied by
    /// `command::context::apply_effect`, which awaits the mutex write
    /// AND emits `TuiEvent::PermissionModeChanged(resolved)` via
    /// `tui_tx.send(..).await` AFTER the write — the event MUST be
    /// awaited to preserve shipped emission-after-write ordering at
    /// slash.rs:320-323. Carries an owned `String` (the validated mode
    /// name) so no borrow on `SlashCommandContext` leaks through the
    /// effect-slot. HYBRID pattern pair with
    /// `CommandContext::permissions_snapshot` (READ side) — see
    /// `src/command/permissions.rs` module rustdoc R1 for the full
    /// split rationale. Mirrors AGS-808 `SetModelOverride`, B10
    /// `AddExtraDir`, and B11 `SetEffortLevelShared` effect-slot
    /// precedent.
    SetPermissionMode(String),
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

// TASK-AGS-POST-6-NO-STUB: `macro_rules! declare_handler` DELETED.
//
// The macro-generated no-op stub pattern previously occupied this
// block (two arms: 2-arg form `[$struct, $description]` and 3-arg form
// `[$struct, $description, $aliases]`). Its two final residual
// invocations — ConfigHandler and CancelHandler — have been migrated
// to THIN-WRAPPER modules (`src/command/config.rs::ConfigHandler` and
// `src/command/cancel.rs::CancelHandler`) that preserve the exact
// shipped description + aliases + no-op `Ok(())` execute body. With
// the macro's callers gone the macro itself is removed to eliminate
// the "stub handler" concept from the registry entirely. Every
// registered command now has a real (or byte-identically-wrapped)
// module under `src/command/`.

// TASK-AGS-POST-6-BODIES-B01-FAST: FastHandler moved to
// `crate::command::fast` (real impl with body-migrated execute via
// DIRECT pattern — sync atomic store on Option<Arc<AtomicBool>>
// from CommandContext, no snapshot/effect-slot needed). Imported at
// the top of this file.
// TASK-AGS-POST-6-BODIES-B24-COMPACT-CLEAR: CompactHandler moved to
// `crate::command::compact` (THIN-WRAPPER pattern — byte-identical
// no-op `Ok(())` with zero emissions). Real /compact body lives
// UPSTREAM at `src/session.rs:2241` which intercepts before the
// dispatcher runs because it needs `agent.lock().await` for the async
// `Agent::compact(..)` call. Real body-migrate deferred to
// POST-STAGE-6 (same deferral as AGS-818 /export). No aliases (shipped
// stub used the two-arg form). Imported at the top of this file.
// TASK-AGS-POST-6-BODIES-B24-COMPACT-CLEAR: ClearHandler moved to
// `crate::command::clear` (THIN-WRAPPER pattern — byte-identical
// no-op `Ok(())` with zero emissions). Real /clear body lives
// UPSTREAM at `src/session.rs:2257` which intercepts before the
// dispatcher runs because it needs `agent.lock().await` for
// `clear_conversation` + personality snapshot. Real body-migrate
// deferred to POST-STAGE-6. Aliases `&["cls"]` PRESERVED per AGS-817
// /memory and AGS-818 /export shipped-wins precedent. Imported at the
// top of this file.
// TASK-AGS-818: ExportHandler moved to `crate::command::export` (real
// impl with CANARY-pattern execute body — session.rs:2409-2480
// intercepts /export before the handler is reachable, so arrival at
// the handler indicates a dispatch ordering bug; see export.rs module
// rustdoc for R1..R5). Aliases `["save"]` are PRESERVED per shipped-
// wins drift-reconcile (AGS-817 /memory precedent). Imported at the
// top of this file. Real body-migrate deferred to POST-STAGE-6
// (ticket AGS-POST-6-EXPORT).
// TASK-AGS-POST-6-BODIES-B02-THINKING: ThinkingHandler moved to
// `src/command/thinking.rs` (DIRECT pattern — sync atomic store on
// CommandContext.show_thinking + TuiEvent::ThinkingToggle + TextDelta
// emissions, subcommand-parsed from args.first()). Real impl and
// tests live in the dedicated module; stub replaced by registry
// import at the top of this file.
// TASK-AGS-POST-6-BODIES-B11-EFFORT: EffortHandler moved to
// `src/command/effort.rs` (real impl with body-migrated execute via
// HYBRID pattern — SNAPSHOT for READ, EFFECT-SLOT for shared mutex
// write, SIDECAR for session-local EffortState write). Shipped stub
// `declare_handler!(EffortHandler, "Show or set reasoning effort
// (high|medium|low)")` at registry.rs:801 is REPLACED by this
// breadcrumb + the import at the top of this file. No aliases —
// shipped stub had none and AGS-817 shipped-wins rule preserves zero
// aliases. Readers looking for the real type should jump to:
//
//   * `crate::command::effort::EffortHandler` — the zero-sized
//     handler struct + `CommandHandler` impl.
//   * `CommandContext::effort_snapshot` (registry.rs above) — the
//     SNAPSHOT field populated by `build_command_context` for
//     `/effort`.
//   * `CommandContext::pending_effort_set` (registry.rs above) — the
//     SIDECAR slot for the local `&mut EffortState` write drained at
//     slash.rs after `apply_effect`.
//   * `CommandEffect::SetEffortLevelShared` (registry.rs above) —
//     the effect variant for the shared-mutex write applied by
//     `command::context::apply_effect`.
// TASK-AGS-POST-6-BODIES-B13-GARDEN: GardenHandler moved to
// `crate::command::garden` (real impl with body-migrated execute via
// DIRECT-sync-via-MemoryTrait pattern — both
// `archon_memory::garden::format_garden_stats` and
// `archon_memory::garden::consolidate` are fully SYNC, matching the
// AGS-817 /memory DIRECT-pattern precedent exactly; no snapshot/effect-
// slot needed). Reads `Arc<dyn MemoryTrait>` from the existing
// `CommandContext::memory` field (AGS-817) plus the new
// `CommandContext::garden_config` DIRECT field populated
// UNCONDITIONALLY by `build_command_context`. Shipped stub
// `declare_handler!(GardenHandler, "Run memory garden consolidation or
// show stats")` at registry.rs:958 is REPLACED by this breadcrumb + the
// import at the top of this file. No aliases (shipped stub used the
// two-arg declare_handler! form; AGS-817 shipped-wins rule preserves
// zero aliases). Mirrors B10-ADDDIR/B09-COLOR breadcrumb style.
// TASK-AGS-808: ModelHandler moved to `crate::command::model` (real
// impl with body-migrated execute via snapshot pattern for READ +
// effect-slot pattern for WRITE, aliases migrated from [models] to
// [m, switch-model] per spec). Imported at the top of this file.
// TASK-AGS-POST-6-BODIES-B14-COPY: CopyHandler moved to
// `crate::command::copy` (real impl with body-migrated execute via
// SNAPSHOT-pattern — `ctx.copy_snapshot` carries a pre-captured clone
// of `slash_ctx.last_assistant_response.lock().await`; the handler
// delegates xclip/clip.exe/pbcopy detection + spawn to an internal
// `ClipboardRunner` trait for testability — production
// `SystemClipboardRunner` preserves shipped slash.rs:163-237 byte-for-
// byte). Shipped stub `declare_handler!(CopyHandler, "Copy the last
// assistant message to the clipboard")` at registry.rs:1014 is
// REPLACED by this breadcrumb + the import at the top of this file.
// No aliases (shipped stub used the two-arg declare_handler! form;
// shipped-wins rule preserves zero aliases). Mirrors B13-GARDEN /
// B12-PERMISSIONS breadcrumb style.
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
// TASK-AGS-POST-6-BODIES-B12-PERMISSIONS: PermissionsHandler moved to
// `crate::command::permissions` (real impl with body-migrated execute
// via HYBRID pattern — SNAPSHOT (`permissions_snapshot` carries
// `current_mode: String` AND `allow_bypass_permissions: bool`) for the
// READ + bypass-guard branches + EFFECT-SLOT via new
// `CommandEffect::SetPermissionMode(String)` for the async mutex write
// and PermissionModeChanged emission). NO sidecar — /permissions has
// no session-local stack state to mutate (unlike /effort's
// EffortState). Shipped stub
// `declare_handler!(PermissionsHandler, "Show or update tool permissions")`
// at registry.rs:914 is REPLACED by this breadcrumb + the import at the
// top of this file. No aliases (shipped stub used the two-arg form;
// AGS-817 shipped-wins rule preserves zero aliases). See
// .gates/TASK-AGS-POST-6-BODIES-B12-PERMISSIONS/ for the full gate
// trail.
// TASK-AGS-813: ConfigHandler gains aliases [settings, prefs] via
// alias-only drift-reconcile (shipped-wins). Spec called for /settings
// as a primary — body-migrate deferred to a post-Stage-6 ticket.
// TASK-AGS-POST-6-NO-STUB: ConfigHandler moved to
// `crate::command::config::ConfigHandler` as a THIN-WRAPPER (same
// pattern as B24 /compact and /clear). Real async work still lives at
// `src/command/slash.rs:247`; this handler's `execute` returns Ok(())
// WITHOUT emitting any TuiEvent, byte-identical to the deleted
// `declare_handler!` macro stub. Aliases `&["settings", "prefs"]`
// PRESERVED per AGS-813 + AGS-817 shipped-wins precedent. Imported at
// the top of this file. See the insert_primary site below for the
// `Arc::new(ConfigHandler::new())` wiring.
// TASK-AGS-817: MemoryHandler moved to `crate::command::memory` (real
// impl with body-migrated execute via DIRECT pattern — sync
// `archon_memory::MemoryTrait`, no snapshot/effect-slot needed). The
// real handler preserves the shipped `["mem"]` alias set per
// shipped-wins drift-reconcile. Imported at the top of this file.
// TASK-AGS-POST-6-BODIES-B15-DOCTOR: DoctorHandler moved to
// `crate::command::doctor` (real impl with body-migrated execute via
// SNAPSHOT-DELEGATE pattern — the shipped async delegate
// `handle_doctor_command` composed the diagnostic text in-place; this
// ticket extracts the composition into `build_doctor_text` and
// threads the owned String through `CommandContext::doctor_snapshot`).
// Shipped stub `declare_handler!(DoctorHandler, "Run environment
// health checks")` at registry.rs:1095 is REPLACED by this breadcrumb
// + the import at the top of this file. No aliases — shipped stub
// had none and spec lists none. See .gates/TASK-AGS-POST-6-BODIES-B15-DOCTOR/
// for the full gate trail.
// TASK-AGS-POST-6-BODIES-B03-BUG: BugHandler moved to
// `crate::command::bug` (real impl with body-migrated execute via
// DIRECT pattern — trivial variant, no state, no args, no snapshot/
// effect-slot, no new CommandContext field). Single TextDelta emission
// of the bug-report URL. Imported at the top of this file.
// TASK-AGS-POST-6-BODIES-B04-DIFF: DiffHandler moved to
// `crate::command::diff` (real impl with body-migrated execute via
// DIRECT with-effect pattern — subprocess `git diff --stat` await
// requires effect-slot; handler stashes CommandEffect::RunGitDiffStat(
// PathBuf) and apply_effect awaits the existing LIVE
// `crate::command::slash::handle_diff_command` helper). Imported at
// the top of this file.
// TASK-AGS-POST-6-BODIES-B08-DENIALS: DenialsHandler moved to
// `crate::command::denials` (real impl with body-migrated execute via
// SNAPSHOT-ONLY pattern — async `denial_log.lock().await` + sync
// `DenialLog::format_display(20)` move to the builder; handler consumes
// `ctx.denial_snapshot` and emits `TuiEvent::TextDelta(format!("\\n{text}\\n"))`).
// EIGHTH Batch-A body-migrate (after B01-FAST, B02-THINKING, B03-BUG,
// B04-DIFF, B05-VIM, B06-HELP, B07-RELEASE-NOTES). No aliases — shipped
// stub used the two-arg declare_handler! form (no aliases slice) and
// spec lists none. See .gates/TASK-AGS-POST-6-BODIES-B08-DENIALS/ for
// the full gate trail.
// TASK-AGS-POST-6-BODIES-B22-LOGIN: LoginHandler moved to
// `crate::command::login` (real impl with body-migrated execute via
// DIRECT pattern — sync filesystem probe `dirs::home_dir().join(".archon")
// .join(".credentials.json").exists()` + byte-identical message build
// + single `ctx.tui_tx.try_send(TuiEvent::TextDelta(msg))` emission
// replaces the `tui_tx.send(..).await` call that lived in the legacy
// slash.rs:285-309 match arm). Imported at the top of this file. No
// aliases — shipped stub used the 2-arg declare_handler! form (no
// aliases slice) and spec lists none. Adds a new
// `CommandContext::auth_label: Option<String>` field populated
// UNCONDITIONALLY by `build_command_context` (AGS-815 session_id /
// AGS-817 memory / B20 config_path cross-cutting precedent). See
// .gates/TASK-AGS-POST-6-BODIES-B22-LOGIN/ for the full gate trail.
// TASK-AGS-POST-6-BODIES-B05-VIM: VimHandler moved to
// `crate::command::vim` (real impl with body-migrated execute via
// DIRECT pattern — emit-only sync handler). Imported at the top of
// this file.
// TASK-AGS-POST-6-BODIES-B16-USAGE: UsageHandler moved to
// `crate::command::usage` (real impl with body-migrated execute via
// SNAPSHOT pattern — single `session_stats.lock().await` moves to the
// builder; handler consumes `ctx.usage_snapshot` and emits
// `TuiEvent::TextDelta(format!(..))` using the shipped byte-identical
// format string with `.4` precision and aligned labels). Shipped stub
// `declare_handler!(UsageHandler, "Show aggregate API usage for the
// session")` at registry.rs:1166 is REPLACED by this breadcrumb + the
// import at the top of this file. No aliases — shipped stub used the
// two-arg declare_handler! form (no aliases slice) and spec lists none.
// /usage is the reason /cost (AGS-809) cannot register `usage` as an
// alias: /usage is already a shipped primary. See
// .gates/TASK-AGS-POST-6-BODIES-B16-USAGE/ for the full gate trail.
// TASK-AGS-806: TasksHandler moved to `crate::command::task` (real
// impl with body-migrated execute, alias set extended to
// [todo, ps, jobs], and TuiEvent::OpenView(ViewId::Tasks) forward-
// compat per AGS-822). Imported at the top of this file.
// TASK-AGS-POST-6-BODIES-B07-RELEASE-NOTES: ReleaseNotesHandler moved to
// `crate::command::release_notes` (real impl with body-migrated execute
// via DIRECT pattern — single sync `ctx.tui_tx.try_send(TuiEvent::TextDelta(..))`
// with byte-identical static literal from slash.rs:451-461, no
// snapshot/effect-slot/new CommandContext field needed). SEVENTH
// Batch-A body-migrate (after B01-FAST, B02-THINKING, B03-BUG, B04-DIFF,
// B05-VIM, B06-HELP). Shipped stub
// `declare_handler!(ReleaseNotesHandler, "Show release notes for the current build")`
// at registry.rs:787 is REPLACED by this breadcrumb + the import at
// the top of this file. No aliases — shipped stub had none and spec
// lists none. See .gates/TASK-AGS-POST-6-BODIES-B07-RELEASE-NOTES/
// for the full gate trail.
// TASK-AGS-POST-6-BODIES-B20-RELOAD: ReloadHandler moved to
// `crate::command::reload` (real impl with body-migrated execute via
// DIRECT pattern — `archon_core::config_watcher::force_reload(
// config_paths: &[PathBuf], current: &ArchonConfig)` is sync; no
// snapshot/effect-slot needed). A new cross-cutting
// `CommandContext::config_path: Option<PathBuf>` field is populated
// UNCONDITIONALLY by `build_command_context` per the AGS-815 session_id
// / AGS-817 memory precedent. Shipped stub
// `declare_handler!(ReloadHandler, "Reload configuration from disk")`
// at this line is REPLACED by this breadcrumb + the import at the top
// of this file. No aliases — shipped stub had none and spec lists
// none. See `src/command/reload.rs` module rustdoc for the full
// R1-R7 invariant list, and the B17 /rename precedent for the
// DIRECT-pattern template.
// TASK-AGS-POST-6-BODIES-B23-LOGOUT: LogoutHandler moved to
// `crate::command::logout` (real impl with body-migrated execute via
// DIRECT pattern — sync `dirs::home_dir().join(".archon")
// .join(".credentials.json")` probe + sync `std::fs::remove_file` +
// three `ctx.tui_tx.try_send(TuiEvent::..)` branches replace the
// `tui_tx.send(..).await` calls that lived in the legacy
// slash.rs:365-392 match arm). Imported at the top of this file. No
// aliases — shipped stub used the 2-arg declare_handler! form (no
// aliases slice) and spec lists none. NO new CommandContext field
// (/logout reads no cross-cutting state — filesystem probe resolves
// against `dirs::home_dir()` inline). insert_primary call below now
// uses `LogoutHandler::new()` matching peer body-migrated handlers.
// See `src/command/logout.rs` module rustdoc for the full R1-R7
// invariant list, and the B22 /login precedent for the DIRECT-pattern
// template. See .gates/TASK-AGS-POST-6-BODIES-B23-LOGOUT/ for the
// full gate trail.
// TASK-AGS-POST-6-BODIES-B06-HELP: HelpHandler moved to
// `crate::command::help` (real impl with body-migrated execute via
// DIRECT-with-field pattern — sync SkillRegistry::format_help /
// format_skill_help, no snapshot/effect-slot needed; aliases ["?", "h"]
// preserved via CommandHandler::aliases() trait method). Imported at
// the top of this file.
// TASK-AGS-POST-6-BODIES-B17-RENAME: RenameHandler moved to
// `crate::command::rename` (real impl with body-migrated execute via
// DIRECT pattern — both archon_session::storage::SessionStore::open
// and archon_session::naming::set_session_name are sync; no
// snapshot/effect-slot needed). Reuses the AGS-815 unconditional
// `CommandContext::session_id: Option<String>` field — no new
// context.rs wiring. Shipped stub `declare_handler!(RenameHandler,
// "Rename the current session")` at this line is REPLACED; the real
// impl is imported at the top of this file and registered via
// `insert_primary("rename", Arc::new(RenameHandler::new()))` at the
// insert-block below (line ~1342). Legacy match arm at
// slash.rs:422-462 stays live through Gates 1-4 (double-fire) per
// R7; Gate 5 deletes it in a separate subagent run.
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
// TASK-AGS-POST-6-BODIES-B21-CHECKPOINT: CheckpointHandler moved to
// `crate::command::checkpoint` (real impl with body-migrated execute
// via DIRECT pattern — sync
// `archon_session::checkpoint::CheckpointStore::open` / `list_modified`
// / `restore`, no snapshot/effect-slot needed, no new CommandContext
// field added — REUSES the AGS-815 `session_id` field). No aliases
// (shipped stub used the 2-arg declare_handler! form). Imported at the
// top of this file. See insert_primary site at registry.rs:1467 below.
// R7 double-fire: legacy match arm at slash.rs:452-527 stays live
// through Gates 1-4; Gate 5 deletes it in a separate subagent run —
// do NOT touch slash.rs in this ticket. See `src/command/checkpoint.rs`
// module rustdoc for the full R1-R7 invariant list, and the B17 /rename
// precedent at `src/command/rename.rs`.
// TASK-AGS-POST-6-BODIES-B10-ADDDIR: AddDirHandler moved to
// `crate::command::add_dir` (real impl with body-migrated execute via
// EFFECT-SLOT pattern — shipped body at slash.rs:679 contained
// `ctx.extra_dirs.lock().await.push(..)` on a `tokio::sync::Mutex`
// forcing the deferred async mutation via the new
// `CommandEffect::AddExtraDir(PathBuf)` variant; `apply_effect` in
// `src/command/context.rs` awaits the push and emits the
// tracing::info! record). Mirrors AGS-808 /model and B04-DIFF effect-
// slot precedent. No aliases (shipped stub had none; AGS-817
// shipped-wins rule preserves zero aliases). Imported at the top of
// this file.
// TASK-AGS-POST-6-BODIES-B09-COLOR: ColorHandler moved to
// `crate::command::color` (real impl with body-migrated execute via
// DIRECT pattern — sync `archon_tui::theme::parse_color`, no
// snapshot/effect-slot needed, no new CommandContext field added).
// Mirrors AGS-819 /theme precedent. No aliases (shipped stub had none;
// AGS-817 shipped-wins rule preserves zero aliases). Imported at the
// top of this file.
// TASK-AGS-819: ThemeHandler moved to `crate::command::theme` (real
// impl with body-migrated execute via DIRECT pattern — sync theme
// helpers `theme_by_name` + `available_themes`, no snapshot/effect-
// slot needed, no new CommandContext field added). FIFTH Batch-3
// ticket. No aliases (shipped stub had none; AGS-817 shipped-wins
// rule preserves zero aliases). Imported at the top of this file.
// /recall stays a standalone primary command and has NO aliases —
// Steven directive. Do NOT add "recall" as an alias on /memory or any
// other handler.
// TASK-AGS-POST-6-BODIES-B18-RECALL: RecallHandler moved to
// `crate::command::recall` (real impl with body-migrated execute via
// DIRECT-sync-via-MemoryTrait pattern — sync
// `archon_memory::MemoryTrait::recall_memories`, no snapshot/effect-
// slot needed, no new CommandContext field added — REUSES the
// AGS-817 `memory` field). No aliases (Steven directive at
// registry.rs:1320-1322; shipped stub used the 2-arg declare_handler!
// form). Imported at the top of this file. See insert_primary site
// at registry.rs:1363 below. R7 double-fire: legacy match arm at
// slash.rs:569-615 stays live through Gates 1-4; Gate 5 deletes it
// in a separate subagent run — do NOT touch slash.rs in this
// ticket. See `src/command/recall.rs` module rustdoc for the full
// R1-R7 invariant list.
// TASK-AGS-POST-6-BODIES-B19-RULES: RulesHandler moved to
// `crate::command::rules` (real impl with body-migrated execute via
// DIRECT-sync-via-MemoryTrait pattern — sync `RulesEngine` methods
// on `&dyn MemoryTrait`, no snapshot/effect-slot needed, no new
// CommandContext field added — REUSES the AGS-817 `memory` field).
// No aliases (shipped stub used the 2-arg declare_handler! form).
// Imported at the top of this file. See insert_primary site at
// registry.rs:1394 below. R7 double-fire: legacy match arm at
// slash.rs:591-706 stays live through Gates 1-4; Gate 5 deletes
// it in a separate subagent run — do NOT touch slash.rs in this
// ticket. See `src/command/rules.rs` module rustdoc for the full
// R1-R7 invariant list, and the B18 /recall precedent at
// `src/command/recall.rs`.
// TASK-AGS-805: /cancel thin wrapper. Body-migrate deferred (shipped
// CommandContext does not expose `task_service`; the stub returns
// `Ok(())` consistent with the 37 peer handlers). Aliases `stop` and
// `abort` route to this handler via the registry alias map.
// TASK-AGS-POST-6-NO-STUB: CancelHandler moved to
// `crate::command::cancel::CancelHandler` as a THIN-WRAPPER (same
// pattern as B24 /compact and /clear + config.rs ConfigHandler).
// Byte-identical no-op: `execute` returns `Ok(())` WITHOUT emitting
// any TuiEvent. The silent-no-op UX gap (no "Cancel requested" / "No
// task running" feedback) is INTENTIONALLY preserved here — fixing
// it is ticket #91 POST-6-CANCEL-AUDIT. Aliases `&["stop", "abort"]`
// PRESERVED per AGS-805 + AGS-817 shipped-wins precedent. Imported at
// the top of this file. See the insert_primary site below for the
// `Arc::new(CancelHandler::new())` wiring.

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
    b.insert_primary("compact", Arc::new(CompactHandler::new()));
    b.insert_primary("clear", Arc::new(ClearHandler::new()));
    b.insert_primary("export", Arc::new(ExportHandler));
    b.insert_primary("thinking", Arc::new(ThinkingHandler));
    b.insert_primary("effort", Arc::new(EffortHandler));
    b.insert_primary("garden", Arc::new(GardenHandler));
    b.insert_primary("model", Arc::new(ModelHandler));
    b.insert_primary("copy", Arc::new(CopyHandler::new()));
    b.insert_primary("context", Arc::new(ContextHandler));
    b.insert_primary("status", Arc::new(StatusHandler));
    b.insert_primary("cost", Arc::new(CostHandler));
    b.insert_primary("permissions", Arc::new(PermissionsHandler));
    // TASK-TUI-626: /plan Plan Mode toggle (SNAPSHOT+EFFECT via SetPermissionMode("plan")).
    b.insert_primary("plan", Arc::new(crate::command::plan::PlanHandler));
    b.insert_primary("config", Arc::new(ConfigHandler::new()));
    b.insert_primary("memory", Arc::new(MemoryHandler));
    b.insert_primary("doctor", Arc::new(DoctorHandler::new()));
    b.insert_primary("bug", Arc::new(BugHandler));
    b.insert_primary("diff", Arc::new(DiffHandler));
    b.insert_primary("denials", Arc::new(DenialsHandler));
    b.insert_primary("login", Arc::new(LoginHandler::new()));
    b.insert_primary("vim", Arc::new(VimHandler));
    b.insert_primary("usage", Arc::new(UsageHandler::new()));
    b.insert_primary("tasks", Arc::new(TasksHandler));
    // TASK-TUI-623: /tag session tag toggle.
    b.insert_primary("tag", Arc::new(crate::command::tag::TagHandler::new()));
    // TASK-TUI-621: hidden stub — dispatchable when typed explicitly,
    // but OMITTED from archon-tui::commands::all_commands() so the
    // autocomplete / command picker never surfaces it.
    b.insert_primary("teleport", Arc::new(crate::command::teleport::TeleportHandler));
    b.insert_primary("release-notes", Arc::new(ReleaseNotesHandler));
    b.insert_primary("reload", Arc::new(ReloadHandler::new()));
    b.insert_primary("logout", Arc::new(LogoutHandler::new()));
    b.insert_primary("help", Arc::new(HelpHandler));
    b.insert_primary("rename", Arc::new(RenameHandler::new()));
    b.insert_primary("resume", Arc::new(ResumeHandler));
    // TASK-TUI-622: /review PR code-review prompt builder.
    b.insert_primary("review", Arc::new(crate::command::review::ReviewHandler::new()));
    // TASK-TUI-624: /commit AI git-commit prompt builder.
    b.insert_primary("commit", Arc::new(crate::command::commit::CommitHandler::new()));
    // TASK-TUI-628: /sandbox Bubble-mode toggle.
    b.insert_primary("sandbox", Arc::new(crate::command::sandbox::SandboxHandler::new()));
    // TASK-TUI-620: /rewind message-selector overlay launcher.
    b.insert_primary("rewind", Arc::new(crate::command::rewind::RewindHandler::new()));
    // TASK-TUI-627: /skills skills-menu overlay launcher.
    b.insert_primary("skills", Arc::new(crate::command::skills::SkillsHandler::new()));
    // TASK-TUI-625: /session remote-URL + QR code display.
    b.insert_primary("session", Arc::new(crate::command::session::SessionHandler::new()));
    b.insert_primary("mcp", Arc::new(McpHandler));
    // TASK-AGS-812: NEW /hooks primary (gap-fix Q4=A, no aliases).
    b.insert_primary("hooks", Arc::new(HooksHandler));
    b.insert_primary("fork", Arc::new(ForkHandler));
    b.insert_primary("checkpoint", Arc::new(CheckpointHandler::new()));
    b.insert_primary("add-dir", Arc::new(AddDirHandler));
    b.insert_primary("color", Arc::new(ColorHandler));
    b.insert_primary("theme", Arc::new(ThemeHandler));
    b.insert_primary("recall", Arc::new(RecallHandler::new()));
    b.insert_primary("rules", Arc::new(RulesHandler::new()));
    // TASK-AGS-805: /cancel primary (aliases: stop, abort).
    b.insert_primary("cancel", Arc::new(CancelHandler::new()));
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
    // TASK-AGS-819: /theme primary registration (no aliases). The
    // /theme body-migrate moves ThemeHandler out of the
    // declare_handler! stub at registry.rs:607 and into
    // `crate::command::theme`. Shipped stub had no alias slice; spec
    // lists none; handler ships `&[]` per AGS-817 shipped-wins rule
    // (zero aliases shipped → zero aliases preserved). FIFTH Batch-3
    // ticket — EXPECTED_COMMAND_COUNT stays at 40 (body-migrate, not
    // gap-fix). Pin the invariant so future ticketing cannot silently
    // add an alias without updating the registry collision-detection
    // tests.
    // -----------------------------------------------------------------

    #[test]
    fn registry_theme_primary_with_no_aliases() {
        let reg = default_registry();
        let primary = reg
            .get("theme")
            .expect("theme primary must be registered post AGS-819");
        let desc = primary.description().to_lowercase();
        assert!(
            desc.contains("theme") || desc.contains("ui"),
            "ThemeHandler description should reference theme/ui, got: {}",
            primary.description()
        );
        // `theme` is a primary — not an alias of anything.
        assert!(reg.is_primary("theme"));
        assert_eq!(reg.primary_for_alias("theme"), None);
        // Spot-check alias-less invariant: `aliases_for` analogue —
        // no alias entry points to `theme`.
        assert!(!reg.aliases_map_contains("theme"));
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

    // -----------------------------------------------------------------
    // TASK-AGS-817: /memory primary registration (alias: `mem`). The
    // /memory body-migrate moves MemoryHandler out of the
    // declare_handler! stub at registry.rs:521-525 and into
    // `crate::command::memory`. Shipped stub carried `&["mem"]`; the
    // spec (orchestrator directive) called for `&[]` but the body-
    // migrate preserves `["mem"]` per shipped-wins drift-reconcile
    // (dropping the alias would regress operators using /mem today).
    // Pin the invariant so future ticketing cannot silently drop the
    // alias or promote a sibling handler to share the `memory` primary
    // name.
    // -----------------------------------------------------------------

    #[test]
    fn registry_memory_primary_with_mem_alias() {
        let reg = default_registry();
        let primary = reg
            .get("memory")
            .expect("memory primary must be registered post AGS-817");
        let desc = primary.description().to_lowercase();
        assert!(
            desc.contains("memor"),
            "MemoryHandler description should reference 'memory', got: {}",
            primary.description()
        );
        // `memory` is a primary — not an alias of anything.
        assert!(reg.is_primary("memory"));
        assert_eq!(reg.primary_for_alias("memory"), None);
        // `mem` is the PRESERVED alias (shipped-wins drift-reconcile).
        assert_eq!(reg.primary_for_alias("mem"), Some("memory"));
        assert!(!reg.is_primary("mem"));
        // The alias resolves to the same handler.
        let via_alias = reg
            .get("mem")
            .expect("'mem' alias must resolve to /memory per AGS-817");
        assert_eq!(
            primary.description(),
            via_alias.description(),
            "'mem' must resolve to the same handler as /memory"
        );
    }

    // -----------------------------------------------------------------
    // TASK-AGS-818: /export primary registration (alias: `save`). The
    // /export body-migrate (Option D / CANARY pattern, registry-hygiene
    // only) moves ExportHandler out of the declare_handler! stub at
    // registry.rs:513-517 and into `crate::command::export`. Shipped
    // stub carried `&["save"]`; the real handler preserves the alias
    // per shipped-wins drift-reconcile (AGS-817 /memory precedent).
    // The real /export BODY stays in session.rs:2409-2480 — session.rs
    // zero-diff invariant held since AGS-805 is preserved by Option D,
    // with real body-migrate deferred to POST-STAGE-6 (ticket
    // AGS-POST-6-EXPORT). Pin the invariant so future ticketing cannot
    // silently drop the `save` alias or promote a sibling handler to
    // share the `export` primary name.
    // -----------------------------------------------------------------

    #[test]
    fn registry_export_primary_with_save_alias() {
        let reg = default_registry();
        let primary = reg
            .get("export")
            .expect("export primary must be registered post AGS-818");
        let desc = primary.description().to_lowercase();
        assert!(
            desc.contains("export") || desc.contains("session"),
            "ExportHandler description should reference export/session, \
             got: {}",
            primary.description()
        );
        // `export` is a primary — not an alias of anything.
        assert!(reg.is_primary("export"));
        assert_eq!(reg.primary_for_alias("export"), None);
        // `save` is the PRESERVED alias (shipped-wins drift-reconcile).
        assert_eq!(reg.primary_for_alias("save"), Some("export"));
        assert!(!reg.is_primary("save"));
        // The alias resolves to the same handler.
        let via_alias = reg
            .get("save")
            .expect("'save' alias must resolve to /export per AGS-818");
        assert_eq!(
            primary.description(),
            via_alias.description(),
            "'save' must resolve to the same handler as /export"
        );
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
            // TASK-AGS-POST-6-BODIES-B04-DIFF: RunGitDiffStat is the
            // second variant, added by the /diff migration. This test
            // only constructs SetModelOverride, so RunGitDiffStat is
            // unreachable here; the arm exists solely to satisfy
            // exhaustiveness and guard against silent drift if a future
            // variant is added without updating this pin.
            CommandEffect::RunGitDiffStat(_) => {
                unreachable!("this test only constructs SetModelOverride")
            }
            // TASK-AGS-POST-6-BODIES-B10-ADDDIR: AddExtraDir is the third
            // variant, added by the /add-dir migration. This test only
            // constructs SetModelOverride, so AddExtraDir is unreachable
            // here; the arm exists solely to satisfy exhaustiveness and
            // guard against silent drift if a future variant is added
            // without updating this pin.
            CommandEffect::AddExtraDir(_) => {
                unreachable!("this test only constructs SetModelOverride")
            }
            // TASK-AGS-POST-6-BODIES-B11-EFFORT: SetEffortLevelShared is
            // the fourth variant, added by the /effort migration. This
            // test only constructs SetModelOverride, so
            // SetEffortLevelShared is unreachable here; the arm exists
            // solely to satisfy exhaustiveness and guard against silent
            // drift if a future variant is added without updating this
            // pin.
            CommandEffect::SetEffortLevelShared(_) => {
                unreachable!("this test only constructs SetModelOverride")
            }
            // TASK-AGS-POST-6-BODIES-B12-PERMISSIONS: SetPermissionMode is
            // the fifth variant, added by the /permissions migration. This
            // test only constructs SetModelOverride, so SetPermissionMode
            // is unreachable here; the arm exists solely to satisfy
            // exhaustiveness and guard against silent drift if a future
            // variant is added without updating this pin.
            CommandEffect::SetPermissionMode(_) => {
                unreachable!("this test only constructs SetModelOverride")
            }
        }
        // Debug impl must not panic — format! exercises it.
        let _ = format!("{e:?}");
    }

    // -----------------------------------------------------------------
    // TASK-AGS-820: Registry integration test (Option C, 3 invariants).
    //
    // Prior HALT finding (agentId a95fe1d3b42139765): 27
    // `declare_handler!` stubs still registered in `default_registry()`.
    // Batch-3 (AGS-805..819) migrated only 13/40 commands. Orchestrator
    // accepted Option C: DROP the NO-STUB invariant, keep COUNT +
    // ALIAS-RESOLUTION + PARSER round-trip. NO-STUB is SCOPE-HELD to
    // new ticket AGS-POST-6-NO-STUB.
    //
    // The three invariants together prove WIRING INTEGRITY: every
    // primary is registered, every alias resolves to the same handler
    // instance as its primary, and every primary round-trips through
    // the parser back into the registry. They do NOT prove runtime
    // dispatch correctness — that is SCOPE-HELD to
    // AGS-POST-6-DISPATCH-SMOKE (per-handler execute() + TuiEvent
    // smoke).
    //
    // # R-items
    //
    // - R1 STRUCTURAL-NOT-DISPATCH: This test proves wiring integrity,
    //   not runtime behavior. Per-handler `execute()` + `TuiEvent` smoke
    //   is SCOPE-HELD to AGS-POST-6-DISPATCH-SMOKE. We deliberately do
    //   NOT call `handler.execute()`, do NOT assert `TuiEvent`
    //   emission, and do NOT build `CommandContext` fixtures here.
    //
    // - R2 HANDLER-IDENTITY-MECHANISM: Alias↔primary handler identity
    //   is compared via `Arc::ptr_eq` on the `Arc<dyn CommandHandler>`
    //   values returned by `Registry::get`. Justification: both the
    //   primary-name path (line 364-366) and the alias path (line
    //   367-368) read from the SAME `self.commands` map and
    //   `Arc::clone` the SAME stored Arc. So alias and primary
    //   lookups return two `Arc` handles to the SAME allocation, and
    //   `Arc::ptr_eq` returns `true`. This avoids adding any new
    //   method to the `CommandHandler` trait (which the task brief
    //   explicitly forbids for the `is_stub` hook and which would be
    //   unnecessary surface for the handler_id hook).
    //
    // - R3 PARSER-REGISTRY-COHERENCE: Round-trip `/{name}` through
    //   `CommandParser::parse` for every primary in
    //   `default_registry()`, then look the parsed name back up in
    //   the registry. One-directional (registry → parser → registry)
    //   per orchestrator scope. Asymmetric direction (parser → registry
    //   → parser) is out of scope — the parser's free function has no
    //   enumerable domain.
    //
    // - R4 FAIL-AT-SPECIFIC-LINK: Each invariant collects failures
    //   into a `Vec<String>` rather than panicking at the first
    //   failure. The final `assert!` concatenates all failure
    //   messages with newlines, so a single test run surfaces EVERY
    //   broken command/alias simultaneously instead of forcing N test
    //   iterations to discover them one at a time. Each message names
    //   the specific command/alias that triggered the failure.
    //
    // - R5 STAGE-SCOPE (DESIGN) — 4-row decomposition of the 40
    //   registered primaries. Prior revisions of this rustdoc used a
    //   2-row frame ("13 body-migrates + 27 stubs") that lied by
    //   aggregation — it folded a CANARY handler, an alias-only
    //   primary, and a Q4=A violation into a single "body-migrate"
    //   bucket. The 4-row decomposition below is authoritative.
    //
    //   Row A — REAL BODY-MIGRATE (12 primaries, sync CommandHandler
    //   impls with shipped imperative logic moved into Handler::execute
    //   or an informational thin-wrapper when no shipped body existed):
    //     /tasks (AGS-806), /status (AGS-807), /model (AGS-808), /cost
    //     (AGS-809), /resume (AGS-810), /mcp (AGS-811), /hooks thin-
    //     wrapper gap-fix (AGS-812), /context (AGS-814), /fork
    //     (AGS-815), /voice thin-wrapper gap-fix (AGS-816), /memory
    //     (AGS-817), /theme (AGS-819).
    //
    //   Row B — CANARY (1 primary, shipped body stays in session.rs
    //   because it needs agent.lock().await which the sync execute
    //   signature cannot service; handler emits a diagnostic TextDelta
    //   if it ever fires — see src/command/export.rs rustdoc R1..R5):
    //     /export (AGS-818).
    //
    //   Row C — ALIAS-HOST STUB (1 primary, functionally a
    //   declare_handler! stub like Row D, categorized separately to
    //   preserve AGS-813's shipped-wins alias-reconcile provenance —
    //   /config is the primary that HOSTS the /settings and /prefs
    //   aliases; /settings → /config resolves through the aliases
    //   HashMap inside `Registry::get` to /config's handler, and
    //   /config's execute currently returns `Ok(())` until body-
    //   migrate lands):
    //     /config (AGS-813 hosts aliases /settings + /prefs).
    //
    //   Row D — PURE STUB (26 primaries, `declare_handler!` macro
    //   invocations with no shipped slash body reached from this
    //   registry; body-migrates DEFERRED to AGS-POST-6-NO-STUB). The
    //   26 are enumerated below by primary name for the benefit of any
    //   future reader who needs the complete list without counting
    //   macro sites:
    //     /cancel, /fast, /compact, /clear, /thinking, /effort,
    //     /garden, /copy, /permissions, /doctor, /bug, /diff,
    //     /denials, /login, /vim, /usage, /release-notes, /reload,
    //     /logout, /help, /rename, /checkpoint, /add-dir, /color,
    //     /recall, /rules.
    //
    //   Row totals: 12 + 1 + 1 + 26 = 40 primaries = EXPECTED_COMMAND_
    //   COUNT. Of the 26 pure stubs, AGS-802 registered them as PARSER
    //   PLACEHOLDERS so `/name` tokens parse and dispatch through this
    //   registry; the runtime behavior for most of them is either
    //   session.rs interception (like /export in Row B) or no-op until
    //   body-migrate lands. The NO-STUB invariant is DEFERRED to
    //   AGS-POST-6-NO-STUB.
    //
    // - R-item STAGE-DRIFT — /cancel Q4=A violation (documented here,
    //   NOT fixed mid-AGS-820). Stage 6 Q4=A ("thin-wrapper for
    //   missing commands") required AGS-805 to deliver a handler that
    //   emits an informational TextDelta when a user types `/cancel`
    //   (the real cancel mechanism is TUI Ctrl-C → dispatcher.cancel_
    //   current() at src/session.rs:2120 and headless --cancel-task
    //   → main.rs::handle_task_cancel at main.rs:193). AGS-805 instead
    //   shipped /cancel as a silent `Ok(())` stub via declare_handler!,
    //   so typing `/cancel` today produces no operator feedback at
    //   all. This is a Q4=A violation classified as STAGE-DRIFT for
    //   Phase C reconciliation — the fix is a follow-up ticket, not
    //   an AGS-820 amendment. Row D above lists /cancel as a pure
    //   stub because that is the observed state, not the intended
    //   state.
    //
    // - R-item METRICS-PROPAGATION-CORRECTION — Sherlock Gate 3
    //   independent warning-count verification caught an error that
    //   propagated across ~17 Stage 6 commit messages (AGS-802..819
    //   and AGS-822). Those commits documented a cargo-warnings
    //   baseline of 40 for the `archon` bin; real baseline per
    //   independent rebuild is 56. AGS-820 adds zero new warnings
    //   (the invariant "no new warnings introduced by this ticket"
    //   still holds), but the absolute figure in older commit
    //   messages is wrong. Canonical warning command (LOCKED going
    //   forward, `^warning:` anchor excludes in-source strings):
    //       cargo build -j1 --bin archon 2>&1 | grep -c '^warning:'
    //   Every future ticket must run this command independently and
    //   must NOT propagate a figure from prior commit messages. No
    //   history rewrite is planned — the correction starts here.
    // -----------------------------------------------------------------

    #[test]
    fn registry_integration_all_commands_wired() {
        use crate::command::parser::CommandParser;

        let registry = default_registry();
        let mut failures: Vec<String> = Vec::new();

        // -------------------------------------------------------------
        // INVARIANT 1 — COUNT
        // -------------------------------------------------------------
        // `default_registry().len()` must equal the expected primary-
        // count constant. If the count drifts, the test names WHICH
        // direction it drifted in the failure message so the operator
        // can reconcile without re-running the test.
        let actual = registry.len();
        if actual != EXPECTED_COMMAND_COUNT {
            failures.push(format!(
                "COUNT invariant failed: expected {EXPECTED_COMMAND_COUNT}, got {actual}"
            ));
        }

        // -------------------------------------------------------------
        // INVARIANT 2 — ALIAS-RESOLUTION
        // -------------------------------------------------------------
        // For every alias declared by every primary handler, assert
        // that `registry.get(alias)` returns an `Arc` pointing at the
        // SAME allocation as `registry.get(primary)`.
        //
        // Iteration strategy: walk `registry.names()` (every primary),
        // fetch the primary handler, read `handler.aliases()` for its
        // static alias list, then do a registry lookup for each alias
        // and compare with `Arc::ptr_eq`. This walks the full
        // (primary, alias) space without needing a public iterator
        // over the private `aliases` HashMap.
        for primary_name in registry.names() {
            let primary_handler = match registry.get(primary_name) {
                Some(h) => h,
                None => {
                    failures.push(format!(
                        "ALIAS-RESOLUTION invariant failed: primary '{primary_name}' \
                         enumerated via names() but missing from registry.get()"
                    ));
                    continue;
                }
            };
            for alias in primary_handler.aliases() {
                let alias_handler = match registry.get(alias) {
                    Some(h) => h,
                    None => {
                        failures.push(format!(
                            "ALIAS-RESOLUTION invariant failed: alias '{alias}' → handler '<missing>' \
                             does NOT match primary '{primary_name}' → handler '{primary}'",
                            primary = primary_name,
                        ));
                        continue;
                    }
                };
                // R2: Arc::ptr_eq on Arc<dyn CommandHandler> returns
                // true iff both handles point to the same allocation.
                // Registry::get for a primary and its alias both
                // Arc::clone the SAME stored Arc, so ptr_eq must hold.
                if !Arc::ptr_eq(&primary_handler, &alias_handler) {
                    failures.push(format!(
                        "ALIAS-RESOLUTION invariant failed: alias '{alias}' → handler '{alias_desc}' \
                         does NOT match primary '{primary_name}' → handler '{primary_desc}'",
                        alias_desc = alias_handler.description(),
                        primary_desc = primary_handler.description(),
                    ));
                }
            }
        }

        // -------------------------------------------------------------
        // INVARIANT 3 — PARSER ROUND-TRIP
        // -------------------------------------------------------------
        // For every primary name N in default_registry():
        //   (a) `CommandParser::parse(&format!("/{N}"))` must succeed.
        //   (b) The resulting `ParsedCommand::name` must resolve to a
        //       handler in the registry via `Registry::get`.
        //
        // (a)+(b) together prove the parser recognizes every primary
        // and the registry can route the parsed output back to its
        // handler. One-directional per R3.
        for primary_name in registry.names() {
            let input = format!("/{primary_name}");
            match CommandParser::parse(&input) {
                Ok(parsed) => {
                    if registry.get(&parsed.name).is_none() {
                        failures.push(format!(
                            "PARSER round-trip invariant failed: primary '{primary_name}' \
                             → parser result 'Ok(name={parsed_name:?})' → registry lookup 'None'",
                            parsed_name = parsed.name,
                        ));
                    }
                }
                Err(e) => {
                    failures.push(format!(
                        "PARSER round-trip invariant failed: primary '{primary_name}' \
                         → parser result 'Err({e:?})' → registry lookup '<skipped: parse failed>'"
                    ));
                }
            }
        }

        // R4: collect-and-report — one test run surfaces every broken
        // command/alias simultaneously instead of panicking at the
        // first failure.
        assert!(
            failures.is_empty(),
            "registry_integration_all_commands_wired: {} invariant failure(s):\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    // -----------------------------------------------------------------
    // TASK-AGS-POST-6-TRY-SEND — CommandContext::emit helper tests.
    //
    // The `emit` helper wraps `tui_tx.try_send` at 105 call sites across
    // 34 files under src/command/, trading the copy-paste `let _ =
    // tui_tx.try_send(...)` pattern for a single method that
    // distinguishes `TrySendError::Full` (warn — benign backpressure on
    // the 256-slot prod buffer) from `TrySendError::Closed` (error —
    // the TUI receiver task is dead).
    //
    // Three tests cover the three match arms: Ok, Full, Closed. None
    // use `CtxBuilder` directly — the Full branch needs a buffer-1
    // channel (builder uses 16), and every test wants the full
    // `CommandContext` literal so that any future field addition causes
    // a localized compile error here rather than a silent drop.
    // -----------------------------------------------------------------

    /// Construct a minimal `CommandContext` with a caller-supplied
    /// `(Sender, Receiver)` pair. Every snapshot / shared-state field
    /// is `None` — emit() only touches `tui_tx`, so the other fields
    /// are irrelevant and kept uninitialized to avoid dragging in
    /// archon-memory / archon-core fixture deps.
    fn make_emit_test_ctx(
        tui_tx: tokio::sync::mpsc::Sender<TuiEvent>,
    ) -> CommandContext {
        CommandContext {
            tui_tx,
            status_snapshot: None,
            model_snapshot: None,
            cost_snapshot: None,
            mcp_snapshot: None,
            context_snapshot: None,
            session_id: None,
            memory: None,
            garden_config: None,
            fast_mode_shared: None,
            show_thinking: None,
            working_dir: None,
            skill_registry: None,
            denial_snapshot: None,
            effort_snapshot: None,
            permissions_snapshot: None,
            copy_snapshot: None,
            doctor_snapshot: None,
            usage_snapshot: None,
            config_path: None,
            auth_label: None,
            pending_effect: None,
            pending_effort_set: None,
            pending_export: None,
        }
    }

    /// Happy path — emit pushes the event into the channel and a
    /// subsequent `try_recv` observes it byte-equivalent.
    #[test]
    fn emit_happy_path_delivers_event() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<TuiEvent>(4);
        let ctx = make_emit_test_ctx(tx);

        ctx.emit(TuiEvent::TextDelta("hello".to_string()));

        match rx.try_recv() {
            Ok(TuiEvent::TextDelta(s)) => assert_eq!(s, "hello"),
            other => panic!(
                "expected Ok(TextDelta(\"hello\")), got {other:?}"
            ),
        }
    }

    /// Full-channel branch — construct a buffer-1 channel, fill it,
    /// then call `emit` again. Must not panic; second event is
    /// silently dropped (production behavior matches shipped `let _ =
    /// try_send`). The `tracing::warn!` call is fire-and-forget; we
    /// only assert the no-panic + drop-semantics contract.
    #[test]
    fn emit_full_channel_warns_and_does_not_panic() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<TuiEvent>(1);
        let ctx = make_emit_test_ctx(tx);

        // Fill the 1-slot buffer.
        ctx.emit(TuiEvent::TextDelta("first".to_string()));
        // Second emit hits Full — must return without panicking.
        ctx.emit(TuiEvent::TextDelta("second (dropped)".to_string()));

        // The first event is still sitting in the buffer; the second
        // was dropped. Drain and verify.
        match rx.try_recv() {
            Ok(TuiEvent::TextDelta(s)) => assert_eq!(s, "first"),
            other => panic!("expected first event buffered, got {other:?}"),
        }
        assert!(
            rx.try_recv().is_err(),
            "second event should have been dropped on Full"
        );
    }

    /// Closed-channel branch — drop the receiver before calling emit.
    /// Must not panic; event is silently dropped.
    #[test]
    fn emit_closed_channel_errors_and_does_not_panic() {
        let (tx, rx) = tokio::sync::mpsc::channel::<TuiEvent>(4);
        drop(rx);
        let ctx = make_emit_test_ctx(tx);

        // Receiver is gone — try_send returns Closed. emit must not
        // panic and must not propagate the error.
        ctx.emit(TuiEvent::TextDelta("orphaned".to_string()));
    }
}
