//! Slash command handler. Extracted from main.rs.

use std::path::PathBuf;
use anyhow::anyhow;
// TASK-AGS-POST-6-BODIES-B19-RULES: /rules body migrated to
// src/command/rules.rs (DIRECT-sync-via-MemoryTrait pattern). The
// shipped `use archon_consciousness::rules::RulesEngine;` import is
// removed — the legacy arm at previously :591-706 has been replaced
// with a breadcrumb, and the new RulesHandler constructs
// `RulesEngine::new(memory.as_ref())` inside its own module.
use archon_llm::effort::EffortState;
use archon_llm::fast_mode::FastModeState;
use archon_tools::task_manager;
use archon_tui::app::TuiEvent;
use crate::command::config::handle_config_command;
// TASK-AGS-POST-6-BODIES-B15-DOCTOR: /doctor body migrated to
// src/command/doctor.rs (SNAPSHOT-DELEGATE pattern). The shipped
// `use crate::command::doctor::handle_doctor_command;` import is
// removed — the delegate has been deleted, all composition runs
// through `build_doctor_text` from `build_doctor_snapshot` at
// dispatch time, and the sync `DoctorHandler::execute` consumes the
// pre-built `DoctorSnapshot`.
use crate::command::registry::CommandContext;
use crate::slash_context::SlashCommandContext;

/// Handle slash commands. Returns `true` if the command was recognized and handled.
pub(crate) async fn handle_slash_command(
    input: &str,
    _fast_mode: &mut FastModeState,
    effort_state: &mut EffortState,
    tui_tx: &tokio::sync::mpsc::Sender<TuiEvent>,
    ctx: &mut SlashCommandContext,
) -> bool {
    // TASK-AGS-623 dispatcher gate (PATH A hybrid).
    //
    // Every slash input now flows through exactly one `Dispatcher::dispatch
    // call: parser → registry lookup → handler (currently no-op stubs from
    // TASK-AGS-622) or `TuiEvent::Error("Unknown command: /{name}")` on miss.
    // Recognized commands fall through to the legacy 43-arm match below,
    // which continues to perform the actual command bodies until TASK-AGS-624
    // migrates those bodies into the registry's stub `execute` methods.
    // Non-slash / empty / bare-`/` inputs short-circuit with `false` — the
    // same behaviour the TASK-AGS-621 parser gate provided.
    // TASK-AGS-807 snapshot-pattern builder. Pre-populates
    // `CommandContext::status_snapshot` (owned values, no locks) when
    // the primary command resolves to /status or its alias /info.
    // Sync CommandHandler::execute cannot await; the builder bridges
    // that gap here at the dispatch site where .await is legal.
    let mut __cmd_ctx = crate::command::context::build_command_context(
        input,
        tui_tx.clone(),
        ctx,
    )
    .await;
    let _ = ctx.dispatcher.dispatch(&mut __cmd_ctx, input);
    // TASK-AGS-808 effect-slot drain. Handlers that need to write to
    // async-guarded shared state (e.g. /model mutating
    // `model_override_shared`) stash a CommandEffect in
    // `pending_effect` synchronously; we consume it with `.take()`
    // here — where `.await` is legal — and apply the mutation via
    // `command::context::apply_effect`. Single-shot by construction.
    if let Some(effect) = __cmd_ctx.pending_effect.take() {
        // TASK-AGS-POST-6-BODIES-B04-DIFF: `tui_tx` threaded into
        // `apply_effect` so the RunGitDiffStat variant can call the
        // existing LIVE `handle_diff_command(tui_tx, &path)` helper
        // at slash.rs:961 without having to clone the sender into the
        // effect variant itself. Prior signature `(effect, slash_ctx)`
        // stays wire-compatible for SetModelOverride (which ignores
        // `tui_tx`).
        crate::command::context::apply_effect(effect, ctx, tui_tx).await;
    }
    // TASK-AGS-POST-6-BODIES-B11-EFFORT: sidecar drain for the local
    // `effort_state: &mut EffortState` parameter. `EffortHandler::execute`
    // stashes BOTH the shared-mutex effect (drained above via
    // `CommandEffect::SetEffortLevelShared` + apply_effect) AND this
    // sidecar slot. The shared-mutex path covers
    // `SlashCommandContext::effort_level_shared`; this drain covers the
    // session-local `EffortState` stack variable that only exists in
    // this function's scope and cannot be written from inside the
    // handler. Single-shot (.take()) by construction; a None here means
    // the handler did not hit the WRITE branch.
    if let Some(level) = __cmd_ctx.pending_effort_set.take() {
        effort_state.set_level(level);
    }
    if !ctx.dispatcher.recognizes(input) {
        return false;
    }

    match input {
        // TASK-AGS-POST-6-BODIES-B01-FAST: /fast match arm deleted. Real
        // body now lives in `src/command/fast.rs` as
        // `impl CommandHandler for FastHandler` (DIRECT pattern —
        // sync atomic toggle on CommandContext.fast_mode_shared).
        // Registry dispatch at the top of this function reaches
        // FastHandler before this match block; arrival at a `/fast`
        // arm here would indicate a dispatch ordering regression.
        // TASK-AGS-POST-6-BODIES-B24-COMPACT-CLEAR: /compact and /clear
        // sentinel arm DELETED. Real handler bodies live in
        // `crate::command::compact::CompactHandler` and
        // `crate::command::clear::ClearHandler` (THIN-WRAPPER pattern —
        // both are sync no-ops returning `Ok(())` with zero emissions,
        // byte-identical to the deleted `declare_handler!` stubs at
        // registry.rs:1207-1208). The REAL /compact and /clear
        // conversation-state mutation bodies remain intercepted
        // UPSTREAM at `src/session.rs:2241` (/compact) and :2257
        // (/clear) because they need `agent.lock().await` which is
        // not available in the handler scope — same POST-STAGE-6
        // deferral pattern as /export (AGS-818). Registry dispatch
        // at the top of this function reaches Compact/Clear handlers
        // before this match block; arrival at a /compact or /clear
        // arm here would indicate a dispatch ordering regression.
        // Wired at registry.rs:1586 insert_primary("compact",
        // Arc::new(CompactHandler::new())) and :1587 insert_primary(
        // "clear", Arc::new(ClearHandler::new())) with `&["cls"]`
        // alias preserved via the real handler's aliases() method.
        // See .gates/TASK-AGS-POST-6-BODIES-B24-COMPACT-CLEAR/.
        // TASK-AGS-818: /export body lives in session.rs:2409-2480 — the
        // TUI input loop intercepts /export before reaching this match
        // block because it needs agent.lock().await for conversation
        // state. The Option D canary in `crate::command::export` only
        // fires if that intercept regresses. Real body-migrate deferred
        // to POST-STAGE-6 (ticket AGS-POST-6-EXPORT).
        // TASK-AGS-POST-6-BODIES-B02-THINKING: /thinking match arms
        // deleted. Real body now lives in `src/command/thinking.rs`
        // as `impl CommandHandler for ThinkingHandler` (DIRECT pattern
        // — sync atomic store on CommandContext.show_thinking +
        // TuiEvent::ThinkingToggle + TextDelta emissions, subcommand-
        // parsed from args.first()). Registry dispatch at the top of
        // this function reaches ThinkingHandler BEFORE this match
        // block fires — dispatcher.dispatch() runs first, then the
        // dispatcher.recognizes() short-circuit returns true before
        // falling into this match. Arrival at a `/thinking` arm here
        // would indicate a dispatch ordering regression.
        // ── /effort ────────────────────────────────────────────
        // TASK-AGS-POST-6-BODIES-B11-EFFORT: shipped arm DELETED.
        // Handler: crate::command::effort::EffortHandler (HYBRID pattern —
        // SNAPSHOT + EFFECT-SLOT + SIDECAR). READ side consumes the
        // `effort_snapshot: Option<EffortSnapshot>` field on
        // CommandContext populated by build_command_context (mirrors
        // AGS-807 StatusSnapshot). ASYNC WRITE side stashes
        // CommandEffect::SetEffortLevelShared(EffortLevel) drained by
        // apply_effect at slash.rs:59. LOCAL WRITE side stashes
        // ctx.pending_effort_set (SIDECAR slot) drained at slash.rs:71
        // where the `&mut effort_state` stack variable is in scope.
        // Wired at crate::command::registry::default_registry
        // insert_primary "effort". Option 3 default arm at
        // slash.rs:~909 (`_ => true,`) returns true for every
        // dispatcher-routed command; session.rs:2491
        // `if handled { continue; }` prevents any dispatcher-driven
        // double-fire. Mirrors the B10-ADDDIR breadcrumb shape at
        // slash.rs:681-692.
        // ── /garden ────────────────────────────────────────────
        // TASK-AGS-POST-6-BODIES-B13-GARDEN: shipped arm DELETED.
        // Handler: crate::command::garden::GardenHandler (DIRECT-sync-via-
        // MemoryTrait pattern — no SNAPSHOT, no EFFECT-SLOT, no SIDECAR).
        // GardenHandler.execute is a sync fn that calls
        // archon_memory::garden::{format_garden_stats, consolidate}
        // directly via `ctx.memory.as_ref()` and `&ctx.garden_config`,
        // emitting TuiEvent via `try_send`. Mirrors the B17 MemoryHandler
        // precedent (src/command/memory.rs). The `garden_config` field on
        // CommandContext (registry.rs:479-480) is populated unconditionally
        // by build_command_context (context.rs:78). No pre-dispatch
        // side-effects: consolidate mutates memory across phase passes,
        // so it MUST run on dispatch (not snapshot-time). Registry
        // dispatch at the top of this function reaches GardenHandler
        // before this match block. Option 3 default arm at slash.rs:858
        // (`_ => true,`) returns true for every dispatcher-routed command;
        // session.rs:2491 `if handled { continue; }` prevents any
        // dispatcher-driven double-fire. Mirrors the B10-ADDDIR /
        // B11-EFFORT / B12-PERMISSIONS breadcrumb shape.
        // ── /model ─────────────────────────────────────────────
        // Body migrated to src/command/model.rs (TASK-AGS-808).
        // Read side (no-args): ModelSnapshot populated by
        // build_command_context. Write side (with arg):
        // CommandEffect::SetModelOverride stored in
        // CommandContext::pending_effect; slash.rs post-dispatch
        // apply_effect awaits the mutex write on
        // slash_ctx.model_override_shared. Aliases: [m, switch-model].
        // Do not re-add the legacy arm — TUI-410 lesson.
        // ── /copy ───────────────────────────────────────────────
        // Body migrated to src/command/copy.rs (TASK-AGS-POST-6-BODIES-
        // B14-COPY). Dispatcher at slash.rs:40-45 (PATH A hybrid) fires
        // CopyHandler via registry lookup. CopySnapshot is populated by
        // build_command_context before dispatch (single
        // `last_assistant_response.lock().await` in the builder — see
        // context.rs:240-253 for the gated match arm). The handler
        // delegates xclip/clip.exe/pbcopy detection + spawn to an
        // internal `ClipboardRunner` trait; production wires
        // `SystemClipboardRunner` which preserves the shipped
        // subprocess work BYTE-FOR-BYTE (same `which` detection order,
        // same Command::new spawn with stdin piped + write_all + wait).
        // Tests inject `MockClipboardRunner` for deterministic Ok/NoTool/
        // SpawnFailed coverage. No aliases (shipped stub used two-arg
        // declare_handler! form). Option 3 default arm at slash.rs:854
        // (drifted from :844 pre-delta due to this arm deletion —
        // functionally identical) short-circuits handle_slash_command to
        // return true for every dispatcher-routed command. Do not re-add
        // the legacy arm — TUI-410 lesson. See registry.rs:621 for the
        // CopySnapshot field, registry.rs:1048-1060 for the breadcrumb
        // replacing the former declare_handler!(CopyHandler, ...) stub,
        // and registry.rs:1222 for the insert_primary call using
        // `CopyHandler::new()` (which wires SystemClipboardRunner).
        // ── /context ────────────────────────────────────────────
        // Body migrated to src/command/context_cmd.rs (TASK-AGS-814).
        // Dispatcher at slash.rs:40-45 (PATH A hybrid) fires
        // ContextHandler via registry lookup. ContextSnapshot is
        // populated by build_command_context before dispatch
        // (single `session_stats.lock().await` in the builder —
        // SNAPSHOT-ONLY pattern). Aliases dropped from stub's
        // `["ctx"]` to `[]` because the legacy match arm only matched
        // `/context` literally — `/ctx` never worked for users. Do not
        // re-add the legacy arm — TUI-410 dead-code lesson.
        // ── /status ────────────────────────────────────────────
        // Body migrated to src/command/status.rs (TASK-AGS-807).
        // Dispatcher at slash.rs:35-41 (PATH A hybrid) fires StatusHandler
        // via alias-aware registry lookup. StatusSnapshot is populated by
        // build_command_context before dispatch. Aliases: [info].
        // Do not re-add the legacy arm — see TUI-410 lesson.
        // ── /cost ──────────────────────────────────────────────
        // Body migrated to src/command/cost.rs (TASK-AGS-809).
        // Dispatcher at slash.rs:40-55 (PATH A hybrid) fires CostHandler
        // via alias-aware registry lookup. CostSnapshot is populated by
        // build_command_context before dispatch (single mutex acquisition
        // on session_stats; cache_stats_line + hard_label pre-computed).
        // Aliases: [billing] only — spec wanted [usage, billing] but
        // `usage` is already a shipped primary (UsageHandler) so the
        // collision-free subset is all we apply. See cost.rs rustdoc
        // for the CONFIRM R-item. Do not re-add the legacy arm — see
        // TUI-410 lesson.
        // ── /permissions: body migrated to src/command/permissions.rs
        //    (TASK-AGS-POST-6-BODIES-B12-PERMISSIONS, HYBRID = SNAPSHOT
        //    + EFFECT-SLOT pattern). Dispatcher PATH A at slash.rs:40
        //    fires PermissionsHandler::execute via the registry BEFORE
        //    this arm; Option 3 default `_ => true,` at slash.rs:884
        //    short-circuits handle_slash_command to prevent the former
        //    arm from being re-entered. Handler stashes
        //    CommandEffect::SetPermissionMode(resolved) on
        //    ctx.pending_effect and try_send's byte-identical TextDelta.
        //    The drain at slash.rs:51-60 awaits
        //    crate::command::context::apply_effect which performs the
        //    async lock-write on slash_ctx.permission_mode FIRST, then
        //    emits TuiEvent::PermissionModeChanged. Registry wiring:
        //    registry.rs:1136 insert_primary("permissions", ...).
        //    Legacy arm deleted per POST-6-FALLTHROUGH precedent
        //    (B01-B04, B09, B10, B11).
        // ── /config [key] [value] ──────────────────────────────
        s if s == "/config" || s.starts_with("/config ") => {
            handle_config_command(s, tui_tx, ctx).await;
            true
        }
        // ── /memory: body migrated to src/command/memory.rs (AGS-817,
        //    DIRECT pattern). Dispatcher PATH A at slash.rs:40 fires
        //    MemoryHandler::execute via the registry BEFORE this arm;
        //    build_command_context populates `CommandContext::memory`
        //    UNCONDITIONALLY (mirrors AGS-815 session_id — Arc<dyn
        //    MemoryTrait> is cheap to clone). Arm deleted per TUI-410
        //    dead-code rule. Do NOT re-add — see TUI-410 lesson.
        //    ──────────────────────────────────────────────────────
        // ── /doctor ── migrated to src/command/doctor.rs (TASK-AGS-POST-6-BODIES-B15-DOCTOR) ──
        // SNAPSHOT-DELEGATE pattern: `build_command_context` awaits
        // `doctor::build_doctor_snapshot(slash_ctx)` when the primary
        // name resolves to `/doctor` and populates
        // `CommandContext::doctor_snapshot`. The registered
        // DoctorHandler (registry.rs:1272) consumes that snapshot
        // synchronously and emits a single TextDelta via try_send.
        // The legacy shipped delegate `handle_doctor_command` was
        // DELETED along with this match arm — see
        // `src/command/doctor.rs` for the current implementation and
        // 7 tests (5 unit + 2 dispatcher-integration) pinning
        // byte-identity with the shipped composition. Default arm
        // `_ => true,` at slash.rs:764 short-circuits any accidental
        // re-entry.
        // ── /bug ── migrated to src/command/bug.rs (TASK-AGS-POST-6-BODIES-B03-BUG) ──
        // DIRECT pattern trivial variant (no state, no args, single TextDelta emission).
        // The registry dispatch at the top of handle_slash_command reaches BugHandler
        // before this match block evaluates; `recognizes("/bug")` short-circuits true
        // so `/bug` inputs never fall through to this region. Arrival here would
        // indicate a dispatch ordering regression — preserve the breadcrumb as a
        // forensic marker rather than deleting silently.
        // ── /diff ── migrated to src/command/diff.rs (TASK-AGS-POST-6-BODIES-B04-DIFF) ──
        // DIRECT with-effect pattern: DiffHandler stashes
        // CommandEffect::RunGitDiffStat(PathBuf); apply_effect drains
        // the slot at slash.rs:51-60 and calls the live
        // handle_diff_command(tui_tx, &path) subprocess helper at
        // slash.rs:930 BEFORE this match block executes. The
        // dispatcher.recognizes("/diff") short-circuit at line 61
        // does NOT early-return (it recognizes the command, so the
        // negation is false), but the effect has already been
        // drained, so reaching this region with a /diff input would
        // re-fire the subprocess and double-emit output — a
        // regression forensic marker. Preserved as a breadcrumb
        // per the B01/B02/B03 precedent.
        // ── /denials ──────────────────────────────────────────
        // TASK-AGS-POST-6-BODIES-B08-DENIALS: shipped arm DELETED.
        // Handler: crate::command::denials::DenialsHandler (SNAPSHOT
        // pattern — builder pre-computes formatted String from
        // denial_log.lock().await + format_display(20) and stores it
        // on CommandContext.denial_snapshot; sync handler emits
        // TextDelta via try_send). Wired at
        // crate::command::registry::default_registry (insert_primary
        // "denials"). Option 3 default arm at slash.rs:909 returns
        // true for every dispatcher-routed command.
        // ── /login ─────────────────────────────────────────────
        // TASK-AGS-POST-6-BODIES-B22-LOGIN: body migrated to
        // `crate::command::login::LoginHandler` (DIRECT pattern — sync
        // `impl CommandHandler` consumes new
        // `CommandContext::auth_label: Option<String>` field added at
        // registry.rs:826 and unconditionally populated at
        // context.rs:175 per AGS-815 /fork session_id precedent; no
        // credential I/O needed — recon of legacy arm here proved it
        // was a pure `dirs::home_dir()`+`.exists()` status display.
        // 8 push_str branches (4 authenticated + 4 not-authenticated,
        // including header + Method line) emitted byte-identical as a
        // single `TuiEvent::TextDelta` via `try_send`. Dispatcher
        // routes `/login` through Registry at registry.rs:1533
        // (`insert_primary("login", Arc::new(LoginHandler::new()))`);
        // import at :297, breadcrumb at :1333. Default arm at
        // slash.rs (drifted position) still returns `true`.
        // See .gates/TASK-AGS-POST-6-BODIES-B22-LOGIN/.
        // ── /vim ───────────────────────────────────────────────
        // TASK-AGS-POST-6-BODIES-B05-VIM: body migrated to
        // `crate::command::vim::VimHandler` (Option C DIRECT pattern —
        // emit-only sync handler, two `ctx.tui_tx.try_send(...)` calls
        // replace the two `tui_tx.send(..).await` emissions that lived
        // here. Dispatcher recognizes `/vim` via registry.rs
        // insert_primary and routes through CommandHandler::execute
        // before this match block is reached; this arm is now
        // unreachable and removed. See .gates/TASK-AGS-POST-6-BODIES-
        // B05-VIM/gate-6.md for the sherlock final review.
        // ── /usage ────────────────────────────────────────────
        // TASK-AGS-POST-6-BODIES-B16-USAGE Gate 5: shipped `/usage`
        // match arm DELETED. Routing now owned by the dispatcher +
        // registry:
        //   - crate::command::usage::UsageHandler (impl — SNAPSHOT
        //     pattern; description preserved byte-for-byte)
        //   - crate::command::usage::{UsageSnapshot, build_usage_snapshot}
        //     (builder awaits session_stats.lock() BEFORE dispatch;
        //     handler consumes ctx.usage_snapshot via try_send TextDelta)
        //   - crate::command::registry::CommandContext::usage_snapshot
        //     (field at registry.rs:685)
        //   - crate::command::registry::default_registry (registration
        //     via insert_primary at registry.rs:1321; breadcrumb
        //     replacing the prior declare_handler! stub at
        //     registry.rs:1197)
        //   - crate::command::context::build_command_context at the
        //     Some("usage") arm (context.rs:284) populates the snapshot
        //     only when the primary resolves to /usage
        //   - Option 3 default arm at slash.rs ~`_ => true,` prevents
        //     skill-registry fallback double-fire.
        // See .gates/TASK-AGS-POST-6-BODIES-B16-USAGE/ for the gate
        // trail and sherlock verdicts.
        // ── /tasks ────────────────────────────────────────────
        // TASK-AGS-806: body migrated to
        // `crate::command::task::TasksHandler` (registered as a
        // primary in registry.rs). Legacy match arm removed; the
        // dispatcher routes /tasks (and aliases todo/ps/jobs)
        // through the registry path.
        // ── /release-notes ────────────────────────────────────
        // TASK-AGS-POST-6-BODIES-B07-RELEASE-NOTES: body migrated to
        // `crate::command::release_notes::ReleaseNotesHandler` (registered
        // as a primary in registry.rs:890). Legacy match arm removed;
        // the dispatcher routes /release-notes through the registry path
        // with the Option 3 default arm at slash.rs:920 (`_ => true,`)
        // short-circuiting skill-fallback. See
        // .gates/TASK-AGS-POST-6-BODIES-B07-RELEASE-NOTES/ for the full
        // gate trail.
        // ── /reload body-migrated to `crate::command::reload::ReloadHandler`
        //    (TASK-AGS-POST-6-BODIES-B20-RELOAD). Pattern: DIRECT
        //    (NOT EFFECT-SLOT as task tag suggested — recon proved
        //    `archon_core::config_watcher::force_reload` is sync).
        //    Handler consumes the new `CommandContext::config_path:
        //    Option<PathBuf>` field populated unconditionally at
        //    context.rs:166 with `Some(slash_ctx.config_path.clone())`
        //    — AGS-815 session_id precedent. All 3 TuiEvent branches
        //    (no-change TextDelta / with-change TextDelta / Err Error)
        //    preserved byte-identical. Registry wiring:
        //    registry.rs:209 import, :770 field, :1311 breadcrumb
        //    replacing declare_handler! stub, :1458 insert_primary
        //    with ReloadHandler::new(). See
        //    .gates/TASK-AGS-POST-6-BODIES-B20-RELOAD/ for the full
        //    gate trail. ─────────────────────────────────────────
        // ── /logout ───────────────────────────────────────────
        // TASK-AGS-POST-6-BODIES-B23-LOGOUT: body migrated to
        // `crate::command::logout::LogoutHandler` (DIRECT pattern —
        // sync `impl CommandHandler` operating on stateless fs ops:
        // `dirs::home_dir()` + `.exists()` + `std::fs::remove_file`
        // all sync; no new CommandContext field needed, no
        // context.rs edits, no fixture churn). 3 TuiEvent branches
        // (remove Ok `Logged out. Credentials cleared.\nRestart…` /
        // remove Err `Failed to clear credentials: {e}` / !exists
        // `No stored credentials found. Using API key auth.`)
        // preserved byte-identical via `try_send`. Dispatcher
        // routes `/logout` through Registry at registry.rs:1571
        // (`insert_primary("logout", Arc::new(LogoutHandler::new()))`);
        // import at :314, breadcrumb at :1410. `dirs::home_dir()`
        // still used by /login has MOVED to login.rs (B22); no
        // remaining uses in slash.rs top-level.
        // See .gates/TASK-AGS-POST-6-BODIES-B23-LOGOUT/.
        // ── /help ──────────────────────────────────────────────
        // TASK-AGS-POST-6-BODIES-B06-HELP Gate 5: shipped `/help` match
        // arm DELETED. Routing now owned by the dispatcher + registry:
        //   - crate::command::help::HelpHandler (impl)
        //   - crate::command::registry::default_registry (registration)
        //   - Option 3 default arm at slash.rs ~953 (`_ => true`) prevents
        //     skill-registry fallback double-fire (HelpSkill exists in
        //     archon-core builtin.rs:270 but is short-circuited).
        // See .gates/TASK-AGS-POST-6-BODIES-B06-HELP/ for the gate trail.
        // ── /rename: body migrated to src/command/rename.rs
        //    (TASK-AGS-POST-6-BODIES-B17-RENAME, DIRECT pattern).
        //    Dispatcher PATH A at slash.rs:46 fires
        //    RenameHandler::execute via the registry BEFORE this arm;
        //    Gate 5 (this commit) deleted the shipped legacy /rename
        //    arm at this position. The new handler is a sync impl
        //    that reuses the AGS-815 CommandContext::session_id:
        //    Option<String> field (populated unconditionally by
        //    build_command_context in context.rs — no snapshot/
        //    effect-slot needed because both SessionStore::open and
        //    naming::set_session_name are sync). Registry wiring:
        //    registry.rs:214 import, :1248-1260 breadcrumb replacing
        //    the shipped declare_handler! stub, :1353 insert_primary
        //    with RenameHandler::new(). See
        //    .gates/TASK-AGS-POST-6-BODIES-B17-RENAME/ for the full
        //    gate trail. Option 3 default arm `_ => true,` below
        //    preserves the dispatcher's "command consumed" return
        //    for any stray legacy callsite. ──────────────────────
        // ── /resume: body migrated to src/command/resume.rs (AGS-810,
        //    DIRECT pattern). Dispatcher PATH A at slash.rs:46 fires
        //    ResumeHandler::execute via the registry BEFORE this arm;
        //    aliases /continue and /open-session route there too. Arm
        //    deleted per TUI-410 dead-code rule. ──────────────────
        // ── /mcp: body migrated to src/command/mcp.rs (AGS-811,
        //    SNAPSHOT-ONLY pattern). Dispatcher PATH A at slash.rs:46
        //    fires McpHandler::execute via the registry BEFORE this
        //    arm; build_command_context awaits
        //    McpServerManager::get_server_info + list_tools_for at the
        //    dispatch site and threads an owned McpSnapshot through
        //    CommandContext. Arm deleted per TUI-410 dead-code rule.
        //    ──────────────────────────────────────────────────────
        // ── /fork: body migrated to src/command/fork.rs (AGS-815,
        //    DIRECT pattern). Dispatcher PATH A at slash.rs:46 fires
        //    ForkHandler::execute via the registry BEFORE this arm;
        //    build_command_context populates `CommandContext::session_id`
        //    unconditionally (DIRECT-pattern contract — not gated on
        //    the primary name, unlike the SNAPSHOT-ONLY fields) so the
        //    sync handler body can call
        //    `archon_session::fork::fork_session` without needing a
        //    per-command match arm in the builder. Arm deleted per
        //    TUI-410 dead-code rule. Do NOT re-add — see TUI-410 lesson.
        //    ──────────────────────────────────────────────────────
        // ── /checkpoint body-migrated to `crate::command::checkpoint::
        //    CheckpointHandler` (TASK-AGS-POST-6-BODIES-B21-CHECKPOINT).
        //    Pattern: DIRECT (NOT EFFECT-SLOT async as the task tag
        //    originally suggested — recon of
        //    `crates/archon-session/src/checkpoint.rs` proved all three
        //    methods (open :83, list_modified :244, restore :198) are
        //    sync). Handler consumes existing
        //    `CommandContext::session_id: Option<String>` field
        //    (AGS-815, already populated unconditionally by
        //    build_command_context — no new field needed). All 8
        //    byte-identical TuiEvent branches preserved (list-empty /
        //    list non-empty / list Err / restore usage-error / restore
        //    Ok / restore Err / store-open Err / catch-all TextDelta).
        //    Registry wiring: registry.rs:259 import, :1379 breadcrumb
        //    replacing declare_handler! stub, :1497 insert_primary
        //    with CheckpointHandler::new(). See .gates/
        //    TASK-AGS-POST-6-BODIES-B21-CHECKPOINT/ for the full
        //    gate trail. ─────────────────────────────────────────
        // ── /add-dir ───────────────────────────────────────────
        // TASK-AGS-POST-6-BODIES-B10-ADDDIR: shipped arm DELETED.
        // Handler: crate::command::add_dir::AddDirHandler (EFFECT-SLOT
        // pattern mirroring B04-DIFF — sync validation + stash of
        // CommandEffect::AddExtraDir(PathBuf); apply_effect in
        // crate::command::context awaits the extra_dirs.lock().push
        // and emits the tracing::info! record).
        // Wired at crate::command::registry::default_registry
        // insert_primary "add-dir". Option 3 default arm at
        // slash.rs:~896 (`_ => true,`) returns true for every
        // dispatcher-routed command; session.rs:2491
        // `if handled { continue; }` prevents AddDirSkill double-fire.
        // ── /color ─────────────────────────────────────────────
        // TASK-AGS-POST-6-BODIES-B09-COLOR: shipped arm DELETED.
        // Handler: crate::command::color::ColorHandler (DIRECT pattern
        // mirroring AGS-819 /theme — sync `archon_tui::theme::parse_color`,
        // no snapshot, no effect slot, no new CommandContext field).
        // Wired at crate::command::registry::default_registry
        // insert_primary "color". Option 3 default arm at slash.rs:909
        // returns true for every dispatcher-routed command.
        // ── /theme: body migrated to src/command/theme.rs (AGS-819,
        //    DIRECT pattern, export.rs-style breadcrumb). Dispatcher
        //    PATH A at slash.rs:46 fires ThemeHandler::execute via the
        //    registry BEFORE this arm; `archon_tui::theme::theme_by_name`
        //    and `archon_tui::theme::available_themes` are sync helpers
        //    so the handler emits TuiEvent::SetTheme + TextDelta + Error
        //    directly via `ctx.tui_tx.try_send(..)` — no snapshot, no
        //    effect slot, no new CommandContext field. Arm deleted per
        //    TUI-410 dead-code rule. Do NOT re-add — see TUI-410 lesson.
        //    ──────────────────────────────────────────────────────
        // ── /recall: body migrated to src/command/recall.rs
        //    (TASK-AGS-POST-6-BODIES-B18-RECALL, DIRECT-sync-via-
        //    MemoryTrait pattern). Dispatcher PATH A at slash.rs:46
        //    fires RecallHandler::execute via the registry BEFORE
        //    this arm; Gate 5 (this commit) deleted the shipped
        //    legacy /recall arm at this position. The new handler
        //    is a sync impl that reuses the AGS-817 cross-cutting
        //    CommandContext::memory: Option<Arc<dyn MemoryTrait>>
        //    field (populated unconditionally by
        //    build_command_context in context.rs — no snapshot/
        //    effect-slot needed because MemoryTrait::recall_memories
        //    is sync on the object-safe trait). Byte-identity of all
        //    four event branches (empty-query Error with U+2014
        //    em-dash, no-match TextDelta, multi-match TextDelta with
        //    "{count} memories matching '{query}':" header +
        //    "  [{id_short}] {title}\n    {snippet}...\n\n" entries,
        //    Err TextDelta "Memory search failed: {e}") preserved
        //    verbatim. Registry wiring: registry.rs:232 import,
        //    :1323 breadcrumb replacing the shipped declare_handler!
        //    stub, :1393 insert_primary with RecallHandler::new().
        //    See .gates/TASK-AGS-POST-6-BODIES-B18-RECALL/ for the
        //    full gate trail. ─────────────────────────────────────
        // ── /rules body-migrated to `crate::command::rules::RulesHandler`
        //    (TASK-AGS-POST-6-BODIES-B19-RULES). Pattern:
        //    DIRECT-sync-via-MemoryTrait (identical to B18 /recall) —
        //    the shipped `RulesEngine::new(&dyn MemoryTrait)` constructor
        //    and its `get_rules_sorted` / `update_rule` / `remove_rule`
        //    methods are all sync, so `impl CommandHandler for
        //    RulesHandler` consumes `ctx.memory.as_ref()` directly
        //    inside the sync execute body — no SNAPSHOT/EFFECT-SLOT
        //    threading needed. Reuses the AGS-817 cross-cutting
        //    `CommandContext::memory: Option<Arc<dyn MemoryTrait>>`
        //    field already populated unconditionally by
        //    `build_command_context` at context.rs:69; no new builder
        //    match arm added. All 14 TuiEvent branches (list-empty /
        //    list-header / list-per-rule / list-Err / edit-short /
        //    edit-success / edit-Err / edit-no-match / edit-lookup-Err
        //    / remove-Ok (positional rule.text) / remove-Err /
        //    remove-no-match / remove-lookup-Err / catch-all) preserved
        //    byte-identical. Registry wiring: registry.rs:254 import,
        //    :1358 breadcrumb replacing the shipped declare_handler!
        //    stub, :1428 insert_primary with RulesHandler::new().
        //    See .gates/TASK-AGS-POST-6-BODIES-B19-RULES/ for the full
        //    gate trail. ─────────────────────────────────────────────
        // TASK-AGS-POST-6-BODIES-B04-DIFF (Option 3 — handler-owns-recognition):
        // Default arm returns `true` for any input that survived the
        // `!ctx.dispatcher.recognizes(input)` guard at the top of this match
        // block. `recognizes` returns true iff the input parses as a slash
        // command whose name is registered in the dispatcher (see
        // dispatcher.rs:143). Reaching this default means:
        //   (a) the command IS recognized (registered), AND
        //   (b) no legacy match arm above claimed it.
        // Which implies the command has already been fully handled by the
        // dispatcher's CommandHandler at line 44 (migrated DIRECT /
        // DIRECT-with-effect / snapshot pattern). Returning `true` prevents
        // session.rs:2491 from falling through to the skill-registry
        // fallback at session.rs:2503 and double-firing the command via a
        // colliding skill (e.g. DiffSkill at builtin.rs:82, FastSkill at
        // builtin.rs:34, ThinkingSkill / BugSkill in expanded.rs). Before
        // this change, every migrated command whose match arm had been
        // deleted (B01-FAST, B02-THINKING, B03-BUG, B04-DIFF — all four
        // had colliding skills) returned `false` and was double-executed
        // by the skill registry.
        //
        // Unrecognized inputs still short-circuit to `return false` at
        // line 62, preserving skill-fallback behaviour for commands that
        // ONLY exist as skills (no primary handler).
        _ => true,
    }
}

// ---------------------------------------------------------------------------
// /diff handler
// ---------------------------------------------------------------------------

pub(crate) async fn handle_diff_command(tui_tx: &tokio::sync::mpsc::Sender<TuiEvent>, working_dir: &PathBuf) {
    let result = tokio::process::Command::new("git")
        .arg("diff")
        .arg("--stat")
        .current_dir(working_dir)
        .output()
        .await;

    match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !output.status.success() {
                if stderr.contains("not a git repository") {
                    let _ = tui_tx
                        .send(TuiEvent::TextDelta("\nNot in a git repository.\n".into()))
                        .await;
                } else {
                    let _ = tui_tx
                        .send(TuiEvent::Error(format!("git diff failed: {stderr}")))
                        .await;
                }
                return;
            }
            if stdout.is_empty() {
                let _ = tui_tx
                    .send(TuiEvent::TextDelta("\nNo uncommitted changes.\n".into()))
                    .await;
            } else {
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(format!("\n{stdout}")))
                    .await;
            }
        }
        Err(e) => {
            let _ = tui_tx
                .send(TuiEvent::Error(format!("Failed to run git: {e}")))
                .await;
        }
    }
}
