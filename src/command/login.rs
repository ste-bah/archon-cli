//! TASK-AGS-POST-6-BODIES-B22-LOGIN: /login slash-command handler
//! (DIRECT pattern, body-migrate).
//!
//! Real `CommandHandler` impl moved here from the `declare_handler!`
//! stub in `src/command/registry.rs:1295` and the legacy match arm at
//! `src/command/slash.rs:285-309`.
//!
//! # R1 — pattern = DIRECT (NOT EFFECT-SLOT as the B22 task tag suggests)
//!
//! Recon of slash.rs:285-309 proved DIRECT is the correct pattern. The
//! shipped `/login` body performs only sync filesystem+string-format
//! work — `dirs::home_dir()`, `.join(...)`, `cred_path.exists()`,
//! `ctx.auth_label` read, and `push_str` building a combined message —
//! plus a single `tui_tx.send(TuiEvent::TextDelta(msg)).await`
//! emission. There is NO OAuth flow, NO credential read, NO async mutex
//! guard, and NO write-back to `SlashCommandContext` state.
//! Consequently:
//!
//! - NO `LoginSnapshot` type (nothing to pre-compute inside an async
//!   guard, unlike `/status` / `/cost` / `/mcp` SNAPSHOT variants).
//! - NO `CommandEffect` variant (handler never mutates shared state;
//!   it only emits a single `TuiEvent` — matches AGS-815 /fork and B20
//!   /reload DIRECT precedent).
//! - A NEW `CommandContext::auth_label: Option<String>` field is added
//!   and populated UNCONDITIONALLY by `build_command_context` (mirrors
//!   the AGS-815 `session_id` / AGS-817 `memory` / B20 `config_path`
//!   cross-cutting precedent — not the per-primary SNAPSHOT gating
//!   pattern). `String` clone is cheap; every handler observes this
//!   field for free without a per-command builder match arm.
//!
//! # R2 — sync CommandHandler::execute rationale
//!
//! `CommandHandler::execute` is sync per the AGS-622 trait contract.
//! The shipped `/login` match arm at slash.rs:285-309 was *async* only
//! because it lived inside the async dispatch loop and emitted via
//! `tui_tx.send(..).await`. The underlying work is 100% sync (no
//! `async fn`, no `.await` in its body — only filesystem existence
//! check and string formatting). In the new sync handler, we emit via
//! `ctx.tui_tx.try_send(..)` (best-effort — dropping a UI message
//! under channel backpressure is preferable to stalling the
//! dispatcher). Matches AGS-815 /fork + B17 /rename + B20 /reload
//! precedent exactly.
//!
//! # R3 — args ignored (shipped silent-ignore behaviour preserved)
//!
//! The shipped match arm took no args — it matched on the literal
//! `"/login"` string, so trailing tokens like `/login foo` never
//! reached this branch. Under the new registry dispatcher the parser
//! tokenizes `/login foo` into `name = "login"` + `args = ["foo"]` and
//! routes to `LoginHandler::execute`. To preserve the shipped
//! silent-ignore behaviour the handler simply ignores `args` — does
//! NOT emit an error for unexpected arguments. Byte-equivalent to
//! shipped for every possible input that used to reach the arm
//! (`/login` alone), and strictly wider / permissive for inputs that
//! didn't. Matches B20 /reload R3 exactly.
//!
//! # R4 — byte-identity of description / aliases / emitted events
//!
//! - `description()` returns `"Authenticate against the configured backend"` —
//!   byte-identical to the `declare_handler!` stub at registry.rs:1295.
//! - `aliases()` returns `&[]` — the shipped stub used the 2-arg
//!   `declare_handler!` form (no aliases slice) and spec lists none.
//! - Emitted events preserve the shipped slash.rs:285-309 format
//!   strings byte-for-byte. Single `TuiEvent::TextDelta(msg)` where
//!   `msg` is built by the same sequence of `push_str` calls across
//!   both branches (authenticated vs not-authenticated).
//!
//! # R5 — aliases = zero
//!
//! Shipped pre-B22: none (2-arg `declare_handler!` form at
//! registry.rs:1295). Spec lists none. No aliases added. Matches
//! /fork / /rename / /mcp / /context / /hooks / /reload precedent.
//!
//! # R6 — auth_label unconditional-populate (AGS-815-style)
//!
//! Unlike the SNAPSHOT-ONLY tickets (AGS-807/808/809/811/814) which
//! gate their populate step on the primary name, this ticket extends
//! `CommandContext` with `auth_label: Option<String>` populated
//! UNCONDITIONALLY in `build_command_context` — mirroring the AGS-815
//! `session_id`, AGS-817 `memory`, B01-FAST `fast_mode_shared`,
//! B02-THINKING `show_thinking`, B04-DIFF `working_dir`, B06-HELP
//! `skill_registry`, B13-GARDEN `garden_config`, and B20-RELOAD
//! `config_path` DIRECT cross-cutting precedent. `String` clone per
//! dispatch is cheap (one heap alloc); future DIRECT handlers that
//! need the auth label inherit this field for free without a
//! per-command builder match arm.
//!
//! The production builder always populates
//! `Some(slash_ctx.auth_label.clone())`. `None` is the sentinel
//! reserved for test fixtures that construct `CommandContext` directly
//! without standing up a full `SlashCommandContext`; in those tests
//! the handler observes `None` and returns an Err-with-message
//! describing the missing-auth_label condition rather than panicking.
//! Mirrors the AGS-815 `fork_handler_execute_without_session_id_returns_err`
//! and B20 `execute_without_config_path_returns_err` pattern.
//!
//! # R7 — Gates 1-4 double-fire note
//!
//! During the Gates 1-4 window, BOTH the new `LoginHandler` (PATH A,
//! via the dispatcher) AND the legacy `"/login" =>` match arm at
//! slash.rs:285-309 are live. Every `/login` invocation therefore
//! fires twice — once via the handler and once via the legacy arm.
//! This is the Stage-6 body-migrate protocol: Gate 5 deletes the
//! legacy match arm in a SEPARATE subsequent subagent run (NOT this
//! subagent's responsibility). Do NOT touch slash.rs in this ticket.
//!
//! # R8 — existing `handle_login` CLI entry point preserved
//!
//! The pre-existing `handle_login` async fn (extracted from src/main.rs
//! as part of TUI-325) is the CLI-subcommand body for `archon login`
//! (OAuth browser flow). It is called from `main.rs:151` and is
//! entirely unrelated to the slash-command handler. This module keeps
//! both symbols side-by-side; no rename / remove / duplicate-code
//! concern.

use crate::Result;
use crate::command::registry::{CommandContext, CommandHandler};
use archon_tui::app::TuiEvent;

// ---------------------------------------------------------------------------
// Pre-existing CLI `archon login` entry point (TUI-325). Untouched by B22.
// ---------------------------------------------------------------------------

pub async fn handle_login(_config: &archon_core::config::ArchonConfig) -> Result<()> {
    let http_client = reqwest::Client::new();
    let cred_path = archon_llm::tokens::credentials_path();

    eprintln!("Starting OAuth login...");
    match archon_llm::oauth::login(&cred_path, &http_client).await {
        Ok(_) => {
            eprintln!("Login successful! Credentials saved.");
            Ok(())
        }
        Err(e) => {
            eprintln!("Login failed: {e}");
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-BODIES-B22-LOGIN: slash-command handler.
// ---------------------------------------------------------------------------

/// Zero-sized handler registered as the primary `/login` command.
///
/// No aliases. Shipped pre-B22 stub carried none (2-arg
/// `declare_handler!` form at registry.rs:1295); spec lists none.
/// Matches /fork / /rename / /mcp / /context / /hooks / /reload
/// precedent.
pub(crate) struct LoginHandler;

impl LoginHandler {
    /// Unit-struct constructor. Matches peer body-migrated handlers
    /// (`DoctorHandler::new`, `UsageHandler::new`, `RenameHandler::new`,
    /// `RecallHandler::new`, `RulesHandler::new`, `ReloadHandler::new`)
    /// even though the unit struct is constructible without it — the
    /// explicit constructor keeps the call site in registry.rs
    /// copy-editable across peers.
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Default for LoginHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHandler for LoginHandler {
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        // R6: require auth_label. `build_command_context` populates
        // this unconditionally from `SlashCommandContext::auth_label`
        // per the AGS-815 session_id / AGS-817 memory / B20 config_path
        // cross-cutting precedent, so at the real dispatch site this
        // branch never fires. Test fixtures that construct
        // `CommandContext` directly with `auth_label: None` will hit
        // this branch and observe an Err — mirroring the AGS-815
        // `fork_handler_execute_without_session_id_returns_err` and
        // B20 `execute_without_config_path_returns_err` pattern.
        let auth_label = ctx.auth_label.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "LoginHandler invoked without ctx.auth_label populated — \
                 build_command_context bug"
            )
        })?;

        // R4: byte-for-byte preservation of slash.rs:285-309 `cred_path`
        // construction.
        let cred_path = dirs::home_dir()
            .unwrap_or_default()
            .join(".archon")
            .join(".credentials.json");

        // R4: byte-for-byte preservation of the slash.rs:291-306 message
        // build. Single `msg` buffer, two branches (authenticated /
        // not-authenticated), single TextDelta emission.
        let mut msg = String::from("\nAuthentication status:\n");
        msg.push_str(&format!("  Method: {}\n", auth_label));
        if cred_path.exists() {
            msg.push_str(&format!("  Credentials: {}\n", cred_path.display()));
            msg.push_str("  Status: authenticated\n\n");
            msg.push_str("  To re-authenticate, run in another terminal:\n");
            msg.push_str("    archon login\n");
        } else {
            msg.push_str("  Credentials: not found\n");
            msg.push_str("  Status: using API key or not authenticated\n\n");
            msg.push_str("  To authenticate with OAuth:\n");
            msg.push_str("    1. Exit this session (Ctrl+D)\n");
            msg.push_str("    2. Run: archon login\n");
            msg.push_str("    3. Follow the browser flow\n");
            msg.push_str("    4. Restart archon\n");
        }

        // R2: sync emission via `try_send`. The shipped arm used
        // `tui_tx.send(..).await` which is forbidden in sync trait
        // methods; best-effort `try_send` matches B17 /rename + B20
        // /reload precedent (dropping a UI message under channel
        // backpressure is preferable to stalling the dispatcher).
        ctx.emit(TuiEvent::TextDelta(msg));
        Ok(())
    }

    fn description(&self) -> &str {
        // R4: byte-identical to declare_handler! stub at
        // registry.rs:1295.
        "Authenticate against the configured backend"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // R5: zero aliases. Shipped stub used the 2-arg
        // declare_handler! form (no aliases slice); spec lists none.
        &[]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-BODIES-B22-LOGIN: tests for /login slash-command body-migrate
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "login_tests.rs"]
mod tests;
