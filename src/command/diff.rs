//! TASK-AGS-POST-6-BODIES-B04-DIFF: /diff slash-command handler
//! (Option C, DIRECT with-effect pattern body-migrate).
//!
//! Reference: docs/stage-7.5/tickets/TASK-AGS-POST-6-BODIES-B04-DIFF.md
//! Based on: src/command/slash.rs:923 (live `handle_diff_command` helper)
//! Source: src/command/registry.rs:673 (`declare_handler!(DiffHandler, ...)`
//!   no-op stub being replaced)
//! Derived from: src/command/model.rs (TASK-AGS-808 effect-slot precedent —
//!   sync handler stashes `CommandEffect`, dispatch-site `apply_effect`
//!   awaits the mutation).
//!
//! Gate 1 skeleton. Real `impl CommandHandler for DiffHandler` lands at
//! Gate 2, replacing the `declare_handler!(DiffHandler, ...)` no-op stub
//! at `src/command/registry.rs:673` and the legacy match arm at
//! `src/command/slash.rs:356-359`.
//!
//! # Why DIRECT with-effect (not trivial, not snapshot)
//!
//! The shipped `/diff` body at slash.rs:357 is:
//!
//! ```ignore
//! "/diff" => {
//!     handle_diff_command(tui_tx, &ctx.working_dir).await;
//!     true
//! }
//! ```
//!
//! where `handle_diff_command` (slash.rs:923) spawns `git diff --stat`
//! via `tokio::process::Command::new("git")` and emits up to five
//! different `TuiEvent` variants depending on exit status and stdout
//! content (not-a-repo TextDelta, git-error Error, spawn-error Error,
//! no-changes TextDelta, stdout TextDelta).
//!
//! - Shipped behavior REQUIRES `await` on a subprocess, which cannot
//!   run inside a sync `CommandHandler::execute`.
//! - Therefore the handler MUST stash a `CommandEffect` variant; the
//!   dispatch site in `slash.rs::handle_slash_command` consumes it via
//!   `.take()` and `apply_effect` awaits the subprocess call — the
//!   established TASK-AGS-808 effect-slot pattern (see `model.rs`).
//! - Unlike `/model` (which stashes a resolved String to write into a
//!   mutex), `/diff` stashes a `PathBuf` (the current working directory)
//!   to pass into the existing `handle_diff_command` helper unchanged.
//! - Because the handler needs the project root but `CommandContext`
//!   today has no `working_dir` field, B04 adds `working_dir:
//!   Option<PathBuf>` to `CommandContext` and populates it
//!   UNCONDITIONALLY in `build_command_context` from
//!   `SlashCommandContext::working_dir` (mirrors the AGS-815
//!   `session_id` / AGS-817 `memory` / B01 `fast_mode_shared` / B02
//!   `show_thinking` cross-cutting precedent).
//!
//! # Byte-for-byte output preservation
//!
//! The handler itself emits NOTHING directly — it only stashes the
//! effect. All user-visible output (5 emission branches) continues to
//! be produced by the existing `handle_diff_command` helper at
//! slash.rs:923, which apply_effect will call unchanged. Byte-identity
//! of the emitted TextDelta / Error strings is therefore preserved by
//! CALL-SITE REUSE, not by literal duplication in this module.
//!
//! `description()` string IS reproduced in this module (replacing the
//! `declare_handler!` macro arg) and MUST be byte-identical to the
//! shipped `"Show a diff of recent file modifications"` at
//! registry.rs:673 (MD5 `c452e46307d9621006ffff5ceeaadd02`).
//!
//! # Trailing-args policy (decision rationale)
//!
//! The shipped arm at slash.rs:356 matches exactly `"/diff"` and would
//! fall through for `"/diff foo"` to the default "unknown command"
//! handler. Post-migration, ALL `/diff*` inputs route to `DiffHandler`
//! via the registry. Chosen preservation strategy: **ignore trailing
//! args and always stash the RunGitDiffStat effect** — mirrors the
//! B03-BUG trailing-args promotion. Simpler code, better UX. The
//! Gate 2 test `diff_handler_ignores_trailing_args` pins this contract.
//!
//! # Missing `working_dir` handling
//!
//! Test fixtures that construct a `CommandContext` without a full
//! `SlashCommandContext` (via `make_*_ctx` helpers in
//! `test_support.rs`) may leave `working_dir: None`. When the handler
//! sees `None`, it emits a `TuiEvent::Error` with a missing-shared-
//! state message (mirrors the B01 `fast_mode_shared: None` handling)
//! and returns `Ok(())` without stashing an effect. Production code
//! paths always populate `Some(path)` in `build_command_context`, so
//! this branch is reachable only via tests.

#[cfg(test)]
mod tests {
    // Gate 1 skeleton — 5 `#[ignore]` + `todo!()` stubs. Real
    // assertions land at Gate 2 once `DiffHandler` exists, the
    // `CommandContext::working_dir` field is added, the `CommandEffect
    // ::RunGitDiffStat(PathBuf)` variant is added, and the
    // `make_diff_ctx` / peer-fixture updates are in place in
    // `test_support.rs`.
    //
    // N=5 tests — covers the effect-stash matrix for a DIRECT
    // with-effect handler plus the byte-identity description pin:
    //   1. working_dir=Some(path), args=[]        → effect stashed, no events
    //   2. working_dir=None,       args=[]        → Error event, no effect stashed
    //   3. working_dir=Some(path), args=["foo"]   → same effect as case 1
    //   4. DiffHandler.description() byte-identical to shipped literal
    //   5. working_dir=Some(path), args=[]        → zero direct TuiEvent emissions
    //
    // Path A-variant inline tests (colocated with the handler impl,
    // matching the B01-FAST, B02-THINKING, and B03-BUG precedent).

    #[test]
    #[ignore = "Gate 2: working_dir=Some(PathBuf::from(\"/tmp\")), args=[] \
                must stash CommandEffect::RunGitDiffStat(PathBuf::from(\"/tmp\")) \
                into ctx.pending_effect; drain_tui_events -> zero events"]
    fn diff_handler_stashes_effect_when_working_dir_present() {
        todo!(
            "Gate 2: DiffHandler.execute(&mut ctx, &[]) with \
             ctx.working_dir=Some(path) -> Ok(()); assert \
             ctx.pending_effect == Some(CommandEffect::RunGitDiffStat(path)) \
             and drain_tui_events(&mut rx) returns an empty Vec"
        )
    }

    #[test]
    #[ignore = "Gate 2: working_dir=None, args=[] must emit exactly one \
                TuiEvent::Error describing the missing-shared-state \
                condition (no effect stashed, pending_effect stays None)"]
    fn diff_handler_errors_when_working_dir_none() {
        todo!(
            "Gate 2: DiffHandler.execute(&mut ctx, &[]) with \
             ctx.working_dir=None -> Ok(()); assert \
             ctx.pending_effect.is_none() and drain_tui_events returns \
             exactly one TuiEvent::Error whose payload describes the \
             missing working_dir condition (mirrors B01-FAST's \
             fast_mode_shared=None handling pattern)"
        )
    }

    #[test]
    #[ignore = "Gate 2: args=[\"foo\"] must stash the SAME \
                CommandEffect::RunGitDiffStat(path) as args=[] case \
                (trailing args ignored, always stash effect — preserves \
                B03-BUG-style trailing-args promotion policy)"]
    fn diff_handler_ignores_trailing_args() {
        todo!(
            "Gate 2: DiffHandler.execute(&mut ctx, &[String::from(\"foo\")]) \
             with ctx.working_dir=Some(path) -> Ok(()); assert \
             ctx.pending_effect == Some(CommandEffect::RunGitDiffStat(path)) \
             (same PathBuf as args=[] case; trailing arg has no effect)"
        )
    }

    #[test]
    #[ignore = "Gate 2: DiffHandler.description() must return the \
                byte-identical shipped string \"Show a diff of recent \
                file modifications\" (MD5 c452e46307d9621006ffff5ceeaadd02); \
                replaces the declare_handler! macro arg at registry.rs:673"]
    fn diff_handler_description_byte_identical_to_shipped() {
        todo!(
            "Gate 2: DiffHandler.description() -> \
             \"Show a diff of recent file modifications\" \
             (byte-identical, MD5-verified at Sherlock Gate 3)"
        )
    }

    #[test]
    #[ignore = "Gate 2: with working_dir=Some(path), args=[], the handler \
                must NOT emit any TuiEvent directly — all user-visible \
                output is deferred through apply_effect -> \
                handle_diff_command. Zero events from execute()."]
    fn diff_handler_execute_emits_no_events_directly() {
        todo!(
            "Gate 2: DiffHandler.execute(&mut ctx, &[]) with \
             ctx.working_dir=Some(path) -> Ok(()); drain_tui_events \
             returns an empty Vec (NO TextDelta, NO Error, NO \
             ThinkingToggle — execute path is effect-only, user output \
             comes from apply_effect's subsequent call into \
             handle_diff_command)"
        )
    }
}
