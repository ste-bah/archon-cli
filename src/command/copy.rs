//! TASK-AGS-POST-6-BODIES-B14-COPY: /copy slash-command handler
//! (body-migrate, SNAPSHOT pattern — READ-only).
//!
//! Reference: shipped inline match arm at `src/command/slash.rs:151-253`.
//! Source:   shipped `declare_handler!(CopyHandler, "Copy the last
//!           assistant message to the clipboard")` stub at
//!           `src/command/registry.rs:1014` (no aliases).
//!
//! # R1 PATTERN-CONFIRM (SNAPSHOT chosen)
//!
//! The shipped body at slash.rs:151-253 performs ONE async action
//! followed by multiple sync actions:
//!
//! 1. **Async READ** `ctx.last_assistant_response.lock().await` — the
//!    only `.await` in the arm. Captures the last assistant message
//!    string content.
//! 2. **Sync tool-detect + sync subprocess spawn + sync wait** — the
//!    three clipboard candidates (`xclip`, `clip.exe`, `pbcopy`) are
//!    probed via `std::process::Command::new("which")...` and the
//!    chosen tool is invoked via `std::process::Command::new(tool)...`
//!    + `.spawn()` + `.wait()`. No async anywhere in the sync tail.
//! 3. **Sync emission** of `TuiEvent::TextDelta` (success + chars) or
//!    `TuiEvent::Error` (no-tool) — shipped uses `.send(..).await` but
//!    `try_send` preserves identity for a single-receiver fast path
//!    (post-migration ordering verified via tests).
//!
//! Because the ONLY async step is the `last_assistant_response` lock,
//! this is a textbook SNAPSHOT migration (AGS-807 / AGS-808 / B08 /
//! B11 / B12 precedent). The builder awaits the lock, clones the
//! string, and threads it through `CommandContext::copy_snapshot`. The
//! sync handler consumes the snapshot and does the tool-detect +
//! spawn + emit work. NO `CommandEffect` variant — nothing writes
//! back to async-locked shared state.
//!
//! * **READ side → SNAPSHOT pattern**. A new
//!   `copy_snapshot: Option<CopySnapshot>` field on `CommandContext`
//!   is populated by `build_command_context` ONLY when the primary
//!   resolves to `/copy`. The builder awaits
//!   `slash_ctx.last_assistant_response.lock().await` and clones the
//!   string so the sync handler can read the content without locking.
//!
//! # R2 PRIMARY-ALREADY-REGISTERED
//!
//! `copy` is already a primary in the default registry via the
//! `declare_handler!(CopyHandler, "Copy the last assistant message
//! to the clipboard")` stub at registry.rs:1014 (no aliases). This
//! ticket is a body-migrate, NOT a gap-fix: primary count is
//! UNCHANGED. The stub is REMOVED in favour of the real type defined
//! in this file, imported at the top of registry.rs, and kept at the
//! existing `insert_primary("copy", Arc::new(CopyHandler::new()))`
//! site.
//!
//! # R3 ALIASES (zero — preserved from shipped)
//!
//! The shipped stub used the two-arg `declare_handler!` form (no
//! aliases slice). Zero aliases preserved. Pinned by test
//! `copy_handler_aliases_are_empty`.
//!
//! # R4 ARG SEMANTICS
//!
//! The shipped arm matched `/copy` literally — no args were consumed.
//! Post-migration, the handler's `args: &[String]` is IGNORED in
//! every branch. Any trailing tokens after `/copy` simply route here
//! and are silently discarded — byte-identical to shipped behaviour
//! (the `"/copy" =>` arm did not even parse a strip_prefix remainder).
//!
//! # R5 CLIPBOARD-RUNNER INDIRECTION (testability extension)
//!
//! The shipped arm inlines three back-to-back `std::process::Command`
//! invocations for tool detection and a per-tool `spawn + write_all +
//! wait` block. This works in production but is invisible to unit
//! tests — no hook point to force a specific tool, force a specific
//! spawn outcome, or capture the content handed to the subprocess.
//!
//! To enable the three terminal branches (Ok + NoToolFound +
//! SpawnFailed) to be tested under the sync interface WITHOUT
//! invoking real subprocesses, a small internal trait
//! [`ClipboardRunner`] factors the subprocess work. The production
//! impl [`SystemClipboardRunner`] preserves shipped behaviour
//! byte-for-byte (same candidate order, same fallback string). Tests
//! substitute a configurable mock runner.
//!
//! The trait is `pub(crate)` and lives in this file — nothing outside
//! the /copy migration touches it. This matches the B13 `TestMemory`
//! inline-double precedent (scope-local testability extension that
//! does NOT leak to `archon_test_support`).
//!
//! # R6 EMISSION ORDER (unchanged vs shipped)
//!
//! Shipped order per branch:
//!
//! ```ignore
//! // empty branch:
//! tui_tx.send(TuiEvent::TextDelta("\nNo assistant response to copy.\n")).await;
//!
//! // ok branch (after successful copy):
//! tui_tx.send(TuiEvent::TextDelta(format!("\nCopied {chars} characters to clipboard.\n"))).await;
//!
//! // no-tool / spawn-failed branch:
//! tui_tx.send(TuiEvent::Error("No clipboard tool found...")).await;
//! ```
//!
//! Post-migration uses `try_send` instead of `.send(..).await`. Every
//! production deployment has a drained mpsc receiver (TUI event loop)
//! so the fast path is identical; `try_send` never blocks and returns
//! `Err` only if the channel is full or closed, which is never true
//! at the /copy dispatch site. The dispatcher contract (handler must
//! return `anyhow::Result<()>` synchronously) forbids `.await`, so
//! `try_send` is the only option — matches every prior B-series
//! migration.
//!
//! # R7 TEMPORARY DOUBLE-FIRE NOTE (Gates 1-4 scope)
//!
//! For Gates 1-4 of this ticket the legacy match arm at
//! `src/command/slash.rs:151-253` is LEFT INTACT. Because
//! `dispatcher.dispatch` fires the handler BEFORE the recognized-
//! command short-circuit allows fall-through into the match, `/copy`
//! will fire CopyHandler AND the legacy arm on every input. Gate 5
//! (live-smoke + legacy-arm deletion) removes the double fire in
//! production. Mirrors every prior B-series migration.
//!
//! The key implication for Gates 1-4 testing: the handler SHOULD be
//! exercised only through the unit-test suite (`cargo test
//! command::copy`) and the dispatcher-integration tests added at
//! Gate 5 — invoking the legacy arm in a live-smoke before Gate 5
//! would produce two clipboard writes per `/copy` invocation. Gate 5
//! deletes the legacy arm first, then runs the dispatcher-integration
//! tests under the fully-migrated single-fire invariant.

use archon_tui::app::TuiEvent;
use std::io::Write;
use std::sync::Arc;

use crate::command::registry::{CommandContext, CommandHandler};
use crate::slash_context::SlashCommandContext;

// ---------------------------------------------------------------------------
// Snapshot
// ---------------------------------------------------------------------------

/// Owned snapshot of the assistant-response string consumed by the
/// /copy handler.
///
/// Built at the dispatch site by `build_command_context` (where
/// `.await` is allowed) and threaded through [`CommandContext`] so
/// the sync handler can consume without holding the
/// `SlashCommandContext::last_assistant_response` mutex.
///
/// Carries a single owned `String` because the shipped arm at
/// slash.rs:151-253 consumes the content in THREE ways:
///
/// 1. As an emptiness check (`is_empty()`) to route to the
///    no-response-yet branch.
/// 2. As a source-of-bytes for the clipboard subprocess stdin
///    (`as_bytes()`).
/// 3. As a `len()` source for the success-message character count.
///
/// All three observations are byte-identical against a clone of the
/// locked string, so the snapshot clones once at build time and the
/// handler makes all three observations without re-locking.
#[derive(Debug, Clone)]
pub(crate) struct CopySnapshot {
    /// Clone of `SlashCommandContext::last_assistant_response`
    /// captured via an awaited `.lock().await`. String (not &str) so
    /// the snapshot is owned and the guard is dropped before the
    /// handler runs.
    pub(crate) last_response: String,
}

/// Build a [`CopySnapshot`] by awaiting the
/// `last_assistant_response` lock in the SAME position as the shipped
/// READ path at `src/command/slash.rs:154`.
///
/// Called from `build_command_context` ONLY when the primary command
/// resolves to `/copy`. All other commands leave
/// `copy_snapshot = None` to avoid unnecessary lock traffic on
/// `last_assistant_response`.
pub(crate) async fn build_copy_snapshot(slash_ctx: &SlashCommandContext) -> CopySnapshot {
    let guard = slash_ctx.last_assistant_response.lock().await;
    let last_response = guard.clone();
    drop(guard); // Guard dropped before return (explicit for clarity).
    CopySnapshot { last_response }
}

// ---------------------------------------------------------------------------
// Clipboard runner (R5 testability extension)
// ---------------------------------------------------------------------------

/// Factored clipboard-subprocess interface so the three terminal
/// branches of `CopyHandler::execute` (Ok, NoToolFound, SpawnFailed)
/// can be exercised under the sync interface without invoking real
/// subprocesses.
///
/// Production impl: [`SystemClipboardRunner`] — byte-identical to the
/// shipped inline subprocess logic at slash.rs:163-237.
///
/// Test impl: `MockClipboardRunner` (inline in `mod tests`) — drives
/// the three terminal outcomes from pre-configured slots.
pub(crate) trait ClipboardRunner: Send + Sync {
    /// Probe the environment for a usable clipboard tool, returning
    /// one of `"xclip"`, `"clip.exe"`, `"pbcopy"`, or `"none"` in
    /// the SAME priority order as the shipped arm at
    /// slash.rs:163-186.
    fn detect_tool(&self) -> &'static str;

    /// Spawn the chosen tool, pipe `content` to its stdin, and wait
    /// for the child to exit. Returns `true` on successful spawn +
    /// write + wait (regardless of exit code, matching shipped
    /// behaviour — shipped only checks `spawn.is_ok()` then calls
    /// `wait()` unconditionally), `false` on any spawn failure.
    ///
    /// `tool` MUST be one of the tokens returned by
    /// [`detect_tool`]; callers are forbidden from passing
    /// `"none"` (the handler branches on `"none"` BEFORE calling
    /// `copy_to_clipboard`).
    fn copy_to_clipboard(&self, tool: &str, content: &str) -> bool;
}

/// Production [`ClipboardRunner`] — preserves the shipped subprocess
/// logic at slash.rs:163-237 byte-for-byte.
///
/// Candidate order (matches shipped):
///   1. `xclip`   (Linux)
///   2. `clip.exe` (WSL → Windows clipboard)
///   3. `pbcopy`  (macOS)
///
/// Detection uses `std::process::Command::new("which")` + the
/// candidate name (matches shipped verbatim).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SystemClipboardRunner;

impl ClipboardRunner for SystemClipboardRunner {
    fn detect_tool(&self) -> &'static str {
        if std::process::Command::new("which")
            .arg("xclip")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            "xclip"
        } else if std::process::Command::new("which")
            .arg("clip.exe")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            "clip.exe"
        } else if std::process::Command::new("which")
            .arg("pbcopy")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            "pbcopy"
        } else {
            "none"
        }
    }

    fn copy_to_clipboard(&self, tool: &str, content: &str) -> bool {
        match tool {
            "xclip" => {
                let mut child = std::process::Command::new("xclip")
                    .arg("-selection")
                    .arg("clipboard")
                    .stdin(std::process::Stdio::piped())
                    .spawn();
                if let Ok(ref mut c) = child {
                    if let Some(ref mut stdin) = c.stdin {
                        let _ = stdin.write_all(content.as_bytes());
                    }
                    let _ = c.wait();
                    true
                } else {
                    false
                }
            }
            "clip.exe" => {
                let mut child = std::process::Command::new("clip.exe")
                    .stdin(std::process::Stdio::piped())
                    .spawn();
                if let Ok(ref mut c) = child {
                    if let Some(ref mut stdin) = c.stdin {
                        let _ = stdin.write_all(content.as_bytes());
                    }
                    let _ = c.wait();
                    true
                } else {
                    false
                }
            }
            "pbcopy" => {
                let mut child = std::process::Command::new("pbcopy")
                    .stdin(std::process::Stdio::piped())
                    .spawn();
                if let Ok(ref mut c) = child {
                    if let Some(ref mut stdin) = c.stdin {
                        let _ = stdin.write_all(content.as_bytes());
                    }
                    let _ = c.wait();
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// Real `/copy` handler — consumes a pre-built [`CopySnapshot`] from
/// [`CommandContext`] and delegates clipboard subprocess work to a
/// [`ClipboardRunner`].
///
/// # Branch matrix
///
/// * `copy_snapshot == None` → `anyhow::Err` (wiring regression).
/// * `last_response.is_empty()` → TextDelta
///   `"\nNo assistant response to copy.\n"`, no clipboard call.
/// * `last_response` non-empty + `detect_tool() == "none"` → Error
///   `"No clipboard tool found. Install xclip (Linux), or use
///   clip.exe (WSL) / pbcopy (macOS)."`, no emission of success
///   TextDelta.
/// * `last_response` non-empty + `detect_tool()` returns a real tool
///   + `copy_to_clipboard` returns `false` (spawn failure) → same
///   "No clipboard tool found..." Error (matches shipped semantics:
///   shipped emits the no-tool Error on ANY `copied == false`
///   outcome, regardless of whether the tool string was `"none"`).
/// * `last_response` non-empty + `detect_tool()` returns a real tool
///   + `copy_to_clipboard` returns `true` → TextDelta
///   `"\nCopied {chars} characters to clipboard.\n"`.
pub(crate) struct CopyHandler {
    /// Clipboard subprocess runner. Production builds use
    /// [`SystemClipboardRunner`]; tests substitute a configurable
    /// mock. Arc so the handler remains `Clone`/`Send + Sync`
    /// without requiring the trait to be `Clone`.
    runner: Arc<dyn ClipboardRunner>,
}

impl CopyHandler {
    /// Default production constructor — wires a
    /// [`SystemClipboardRunner`] that preserves shipped subprocess
    /// behaviour byte-for-byte.
    pub(crate) fn new() -> Self {
        Self {
            runner: Arc::new(SystemClipboardRunner),
        }
    }

    /// Test-only constructor — substitute an arbitrary
    /// [`ClipboardRunner`] impl.
    #[cfg(test)]
    pub(crate) fn with_runner(runner: Arc<dyn ClipboardRunner>) -> Self {
        Self { runner }
    }
}

impl Default for CopyHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHandler for CopyHandler {
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        // R4: args are IGNORED. The shipped arm matched `/copy`
        // literally with no strip_prefix — trailing tokens were
        // silently discarded. Preserved here via the `_args`
        // parameter rename.

        // Consume the pre-built snapshot populated by
        // `build_command_context` when the primary resolved to
        // `/copy`. A `None` here indicates a wiring regression
        // (builder bypassed or alias map drifted); surface it as a
        // loud `Err` rather than a user-facing message (mirrors
        // B12 PermissionsHandler defensive stance).
        let snap = ctx.copy_snapshot.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "CopyHandler invoked without copy_snapshot populated — \
                 build_command_context wiring regression"
            )
        })?;

        if snap.last_response.is_empty() {
            // Empty-response branch — byte-identical TextDelta from
            // shipped slash.rs:156-160.
            ctx.emit(TuiEvent::TextDelta(
                "\nNo assistant response to copy.\n".to_string(),
            ));
            return Ok(());
        }

        // Non-empty branch — probe the clipboard tool and attempt
        // the copy.
        let tool = self.runner.detect_tool();
        // The shipped arm routed `tool == "none"` to `copied = false`
        // via the catch-all match arm at slash.rs:236 (`_ => false`).
        // Here we skip the subprocess call when `tool == "none"` to
        // avoid invoking `copy_to_clipboard("none", ...)`; per the
        // trait contract callers are forbidden from passing "none".
        let copied = if tool == "none" {
            false
        } else {
            self.runner.copy_to_clipboard(tool, &snap.last_response)
        };

        if copied {
            let chars = snap.last_response.len();
            ctx.emit(TuiEvent::TextDelta(format!(
                "\nCopied {chars} characters to clipboard.\n"
            )));
        } else {
            // Byte-identical to shipped slash.rs:247-249.
            ctx.emit(TuiEvent::Error(
                "No clipboard tool found. Install xclip (Linux), or use clip.exe (WSL) / pbcopy (macOS)."
                    .to_string(),
            ));
        }
        Ok(())
    }

    fn description(&self) -> &str {
        // Byte-for-byte preservation of the shipped declare_handler!
        // stub at registry.rs:1014 (shipped-wins drift-reconcile).
        "Copy the last assistant message to the clipboard"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // R3: zero aliases shipped → zero aliases preserved. Pinned
        // by test `copy_handler_aliases_are_empty`.
        &[]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "copy_tests.rs"]
mod tests;
