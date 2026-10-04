//! TASK-AGS-POST-6-BODIES-B23-LOGOUT: /logout slash-command handler
//! (DIRECT pattern, body-migrate).
//!
//! Real `CommandHandler` impl moved here from the `declare_handler!`
//! stub in `src/command/registry.rs:1393` and the legacy match arm at
//! `src/command/slash.rs:365-392`.
//!
//! # R1 — pattern = DIRECT (NOT EFFECT-SLOT as the B23 task tag suggests)
//!
//! Recon of slash.rs:365-392 proved DIRECT is the correct pattern. The
//! shipped `/logout` body performs only sync filesystem work —
//! `dirs::home_dir()`, `.join(...)`, `cred_path.exists()`, and
//! `std::fs::remove_file` — plus three `tui_tx.send(..).await`
//! emissions across its three branches. There is NO OAuth flow, NO
//! async I/O, NO async mutex guard, and NO write-back to
//! `SlashCommandContext` state. Consequently:
//!
//! - NO `LogoutSnapshot` type (nothing to pre-compute inside an async
//!   guard, unlike `/status` / `/cost` / `/mcp` SNAPSHOT variants).
//! - NO `CommandEffect` variant (handler never mutates shared state;
//!   it emits `TuiEvent` values only — matches AGS-815 /fork, B20
//!   /reload, and B22 /login DIRECT precedent).
//! - NO new `CommandContext` field (unlike B22 /login which added
//!   `auth_label`: /logout reads no cross-cutting state. The only
//!   runtime signal is the presence or absence of
//!   `~/.archon/.credentials.json` on disk, which is resolved inline
//!   via `dirs::home_dir()` — no builder involvement required).
//!
//! # R2 — sync CommandHandler::execute rationale
//!
//! `CommandHandler::execute` is sync per the AGS-622 trait contract.
//! The shipped `/logout` match arm at slash.rs:365-392 was *async*
//! only because it lived inside the async dispatch loop and emitted
//! via `tui_tx.send(..).await`. The underlying work is 100% sync (no
//! `async fn`, no `.await` in its body — only `dirs::home_dir()`,
//! `Path::exists()`, `std::fs::remove_file()` — all synchronous). In
//! the new sync handler we emit via `ctx.tui_tx.try_send(..)` (best-
//! effort — dropping a UI message under channel backpressure is
//! preferable to stalling the dispatcher). Matches B17 /rename + B20
//! /reload + B22 /login precedent exactly.
//!
//! # R3 — args ignored (shipped silent-ignore behaviour preserved)
//!
//! The shipped match arm took no args — it matched on the literal
//! `"/logout"` string, so trailing tokens like `/logout foo` never
//! reached this branch. Under the new registry dispatcher the parser
//! tokenizes `/logout foo` into `name = "logout"` + `args = ["foo"]`
//! and routes to `LogoutHandler::execute`. To preserve the shipped
//! silent-ignore behaviour the handler simply ignores `args` — does
//! NOT emit an error for unexpected arguments. Byte-equivalent to
//! shipped for every possible input that used to reach the arm
//! (`/logout` alone), and strictly wider / permissive for inputs that
//! didn't. Matches B20 /reload and B22 /login R3 exactly.
//!
//! # R4 — byte-identity of description / aliases / emitted events
//!
//! - `description()` returns `"Clear stored credentials"` — byte-
//!   identical to the `declare_handler!` stub at registry.rs:1393.
//! - `aliases()` returns `&[]` — the shipped stub used the 2-arg
//!   `declare_handler!` form (no aliases slice) and spec lists none.
//! - Emitted events preserve the shipped slash.rs:365-392 format
//!   strings byte-for-byte across all three branches:
//!     1. cred_path exists + remove Ok -> TextDelta("\nLogged out.
//!        Credentials cleared.\nRestart and run /login to
//!        re-authenticate.\n")
//!     2. cred_path exists + remove Err -> Error(format!(
//!        "Failed to clear credentials: {e}"))
//!     3. !cred_path.exists() -> TextDelta("\nNo stored credentials
//!        found. Using API key auth.\n")
//!
//! # R5 — aliases = zero
//!
//! Shipped pre-B23: none (2-arg `declare_handler!` form at
//! registry.rs:1393). Spec lists none. No aliases added. Matches
//! /fork / /rename / /mcp / /context / /hooks / /reload / /login
//! precedent.
//!
//! # R6 — no CommandContext field added
//!
//! Unlike B22 /login which added `auth_label: Option<String>` as a
//! cross-cutting DIRECT field, /logout reads no shared state. The
//! filesystem probe resolves against `dirs::home_dir()` inline — no
//! builder involvement, no new field, no fixture churn. The handler
//! is effectively stateless with respect to `CommandContext` beyond
//! the `tui_tx` channel every handler uses to emit events.
//!
//! # R7 — Gates 1-4 double-fire note
//!
//! During the Gates 1-4 window, BOTH the new `LogoutHandler` (PATH A,
//! via the dispatcher) AND the legacy `"/logout" =>` match arm at
//! slash.rs:365-392 are live. Every `/logout` invocation therefore
//! fires twice — once via the handler and once via the legacy arm.
//! This is the Stage-6 body-migrate protocol: Gate 5 deletes the
//! legacy match arm in a SEPARATE parent-context run (NOT this
//! subagent's responsibility). Do NOT touch slash.rs in this ticket.

use crate::command::registry::{CommandContext, CommandHandler};
use archon_tui::app::TuiEvent;

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-BODIES-B23-LOGOUT: slash-command handler.
// ---------------------------------------------------------------------------

/// Zero-sized handler registered as the primary `/logout` command.
///
/// No aliases. Shipped pre-B23 stub carried none (2-arg
/// `declare_handler!` form at registry.rs:1393); spec lists none.
/// Matches /fork / /rename / /mcp / /context / /hooks / /reload /
/// /login precedent.
pub(crate) struct LogoutHandler;

impl LogoutHandler {
    /// Unit-struct constructor. Matches peer body-migrated handlers
    /// (`DoctorHandler::new`, `UsageHandler::new`, `RenameHandler::new`,
    /// `RecallHandler::new`, `RulesHandler::new`, `ReloadHandler::new`,
    /// `LoginHandler::new`) even though the unit struct is
    /// constructible without it — the explicit constructor keeps the
    /// call site in registry.rs copy-editable across peers.
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Default for LogoutHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHandler for LogoutHandler {
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        // R4: byte-for-byte preservation of slash.rs:365-370 `cred_path`
        // construction.
        let cred_path = dirs::home_dir()
            .unwrap_or_default()
            .join(".archon")
            .join(".credentials.json");

        // R4: byte-for-byte preservation of the slash.rs:371-390 three-
        // branch emission shape. Sync `Path::exists()` gate; sync
        // `std::fs::remove_file()` on the authenticated branch; single
        // fallback TextDelta on the unauthenticated branch.
        if cred_path.exists() {
            match std::fs::remove_file(&cred_path) {
                Ok(()) => {
                    // R2: sync emission via `try_send`. The shipped arm
                    // used `tui_tx.send(..).await` which is forbidden in
                    // sync trait methods; best-effort `try_send` matches
                    // B17 /rename + B20 /reload + B22 /login precedent.
                    ctx.emit(TuiEvent::TextDelta(
                        "\nLogged out. Credentials cleared.\n\
                         Restart and run /login to re-authenticate.\n"
                            .into(),
                    ));
                }
                Err(e) => {
                    ctx.emit(TuiEvent::Error(format!("Failed to clear credentials: {e}")));
                }
            }
        } else {
            ctx.emit(TuiEvent::TextDelta(
                "\nNo stored credentials found. Using API key auth.\n".into(),
            ));
        }
        Ok(())
    }

    fn description(&self) -> &str {
        // R4: byte-identical to declare_handler! stub at
        // registry.rs:1393.
        "Clear stored credentials"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // R5: zero aliases. Shipped stub used the 2-arg
        // declare_handler! form (no aliases slice); spec lists none.
        &[]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-BODIES-B23-LOGOUT: tests for /logout slash-command body-migrate
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "logout_tests.rs"]
mod tests;
