use std::path::PathBuf;

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
