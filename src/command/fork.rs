//! TASK-AGS-815: /fork slash-command handler (Option C, DIRECT pattern,
//! FIRST Batch-3 body-migrate).
//!
//! Real `CommandHandler` impl moved here from the `declare_handler!`
//! stub in `src/command/registry.rs:524` and the legacy match arm at
//! `src/command/slash.rs:614-645`.
//!
//! # Why DIRECT (no snapshot, no effect slot)?
//!
//! The shipped `/fork` body reads `archon_session` synchronously —
//! every call into `archon_session::storage::SessionStore::open` and
//! `archon_session::fork::fork_session` is a plain sync function (no
//! `.await`). There are no `tokio::sync::Mutex` guards on the read
//! path and no writes back to `SlashCommandContext` state. Consequently:
//!
//! - NO `ForkSnapshot` type (nothing to pre-compute inside an async
//!   guard, unlike `/status` / `/model` / `/cost` / `/mcp` /
//!   `/context`).
//! - NO `CommandEffect` variant (handler never mutates shared state;
//!   it only emits `TuiEvent`s — matches AGS-810 /resume precedent).
//! - NO `build_command_context` match arm added for `/fork`. Unlike
//!   the SNAPSHOT-ONLY tickets (AGS-807/808/809/811/814) which gate
//!   their populate step on the primary name, AGS-815 extends
//!   `CommandContext` with a `session_id: Option<String>` field that
//!   is populated UNCONDITIONALLY in the builder's outer literal.
//!   The tradeoff: every dispatch pays for one `String::clone()` of
//!   `SlashCommandContext::session_id`. Session ids are UUIDs (36
//!   bytes), so the per-dispatch cost is negligible and it avoids
//!   proliferating per-command arms for a field future handlers
//!   (AGS-818 /export, AGS-819 /theme, future /checkpoint migrations)
//!   may also want to read.
//!
//! The sole side effect is `ctx.tui_tx.try_send(TuiEvent::…)` — sync
//! and legal inside `CommandHandler::execute`. Matches AGS-810
//! /resume DIRECT-pattern precedent.
//!
//! # Byte-for-byte output preservation
//!
//! Every emitted string is faithful to the deleted slash.rs:614-645
//! body:
//! - Fork success -> `TuiEvent::TextDelta(format!("\nConversation forked \
//!   as: {new_id}\nResume with: archon --resume {new_id}\nOriginal \
//!   session: {session_id}\n"))`
//! - fork_session error -> `TuiEvent::Error(format!("Fork failed: {e}"))`
//! - SessionStore::open error -> `TuiEvent::Error(format!("Session store \
//!   error: {e}"))`
//!
//! The one emission-primitive change is `tui_tx.send(..).await` (async)
//! -> `ctx.tui_tx.try_send(..)` (sync), matching
//! AGS-806/807/808/809/810/811/814 precedent. `/fork` output is
//! best-effort UI — dropping a message under channel backpressure is
//! preferable to stalling the dispatcher.
//!
//! # Aliases
//!
//! Shipped pre-AGS-815: none. Spec lists none. No aliases added —
//! matches `/mcp` / `/context` / `/hooks` precedent.
//!
//! # Args-path reconciliation
//!
//! Shipped body used `s.strip_prefix("/fork").unwrap_or("").trim()` on
//! the full input string, so a `/fork my name` input (two tokens) was
//! forwarded verbatim as the new session name. The registry parser
//! tokenizes on whitespace, so `args` is `["my", "name"]` — two
//! entries. To preserve the shipped single-string semantics while
//! going through the parser, the handler joins `args` with a single
//! space. Empty `args` -> empty string -> `None` passed to
//! `fork_session` (anonymous fork). Single-token args pass through
//! unchanged.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// Zero-sized handler registered as the primary `/fork` command.
///
/// No aliases. Shipped pre-AGS-815 stub carried none; spec lists
/// none. Matches `/mcp` / `/context` / `/hooks` precedent.
pub(crate) struct ForkHandler;

impl CommandHandler for ForkHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        args: &[String],
    ) -> anyhow::Result<()> {
        // 1. Require session_id. `build_command_context` populates this
        //    unconditionally from `SlashCommandContext::session_id` so
        //    at the real dispatch site this branch never fires. Test
        //    fixtures that construct `CommandContext` directly with
        //    `session_id: None` will hit this branch and observe an
        //    Err — mirroring the `status_handler_execute_without_
        //    snapshot_returns_err` pattern established in AGS-807.
        let Some(session_id) = ctx.session_id.as_deref() else {
            return Err(anyhow::anyhow!(
                "/fork dispatched without session_id — \
                 CommandContext::session_id was None (build_command_context \
                 always populates it; this is a test-fixture or wiring bug)"
            ));
        };

        // 2. Reconcile shipped single-string arg semantics vs. parser
        //    tokenization. See module rustdoc ArgsPath section.
        //    Joining with ' ' preserves the shipped behaviour where
        //    `/fork my name` forks as "my name". Single-token args
        //    pass through unchanged. Empty args -> empty string.
        let joined = args.join(" ");
        let name_arg = joined.trim();
        let fork_name = if name_arg.is_empty() {
            None
        } else {
            Some(name_arg)
        };

        // 3. Open the session store. Every downstream branch depends
        //    on a valid `SessionStore`; a failure here surfaces as a
        //    user-facing `TuiEvent::Error`, matching the shipped Err
        //    arm at slash.rs:638-641.
        let db_path = archon_session::storage::default_db_path();
        match archon_session::storage::SessionStore::open(&db_path) {
            Ok(store) => {
                match archon_session::fork::fork_session(
                    &store,
                    session_id,
                    fork_name,
                ) {
                    Ok(new_id) => {
                        let msg = format!(
                            "\nConversation forked as: {new_id}\n\
                             Resume with: archon --resume {new_id}\n\
                             Original session: {session_id}\n"
                        );
                        ctx.emit(TuiEvent::TextDelta(msg));
                    }
                    Err(e) => {
                        ctx.emit(TuiEvent::Error(
                            format!("Fork failed: {e}"),
                        ));
                    }
                }
            }
            Err(e) => {
                ctx.emit(TuiEvent::Error(format!(
                    "Session store error: {e}"
                )));
            }
        }
        Ok(())
    }

    fn description(&self) -> &'static str {
        "Fork the current session into a new branch"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-815: tests for /fork slash-command body-migrate
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use archon_tui::app::TuiEvent;
    use tokio::sync::mpsc;

    /// Build a `CommandContext` with a freshly-created channel and the
    /// supplied `session_id`. Mirrors the `make_ctx` fixtures in
    /// task.rs / cost.rs / model.rs / status.rs / resume.rs.
    ///
    /// Every optional field other than `session_id` stays `None`:
    /// `/fork` is a DIRECT-pattern handler and does not consume any
    /// of the typed snapshots.
    fn make_ctx(
        session_id: Option<String>,
    ) -> (CommandContext, mpsc::Receiver<TuiEvent>) {
        // TASK-AGS-POST-6-SHARED-FIXTURES-V2: migrated to CtxBuilder.
        crate::command::test_support::CtxBuilder::new()
            .with_session_id_opt(session_id)
            .build()
    }

    #[test]
    fn fork_handler_description_matches() {
        let h = ForkHandler;
        let desc = h.description().to_lowercase();
        assert!(
            desc.contains("fork") || desc.contains("session"),
            "ForkHandler description should reference 'fork' or \
             'session', got: {}",
            h.description()
        );
    }

    #[test]
    fn fork_handler_aliases_are_empty() {
        let h = ForkHandler;
        assert!(
            h.aliases().is_empty(),
            "ForkHandler has no aliases per AGS-815 spec (shipped stub \
             had none, spec lists none), got: {:?}",
            h.aliases()
        );
    }

    /// When `CommandContext::session_id` is `None`, execute() must
    /// return Err with a message describing the missing field. The
    /// real builder populates the field unconditionally; this branch
    /// guards against test-fixture or wiring regressions.
    #[test]
    fn fork_handler_execute_without_session_id_returns_err() {
        let (mut ctx, _rx) = make_ctx(None);
        let h = ForkHandler;
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_err(),
            "ForkHandler::execute must return Err when session_id is \
             None (builder contract violation), got: {res:?}"
        );
        let msg = format!("{:#}", res.unwrap_err());
        assert!(
            msg.contains("session_id"),
            "Err message must mention 'session_id' so the operator can \
             trace the wiring bug, got: {msg}"
        );
    }

    /// Executing with a valid session_id must return Ok(()) regardless
    /// of whether the default DB exists on disk. Branches:
    ///   - DB missing: `SessionStore::open` returns Err -> emit
    ///     `TuiEvent::Error("Session store error: ...")`, still Ok(()).
    ///   - DB present + source id absent: `fork_session` returns Err
    ///     (no source session) -> emit `TuiEvent::Error("Fork failed:
    ///     ...")`, still Ok(()).
    ///   - DB present + source id matches: `fork_session` succeeds ->
    ///     emit `TuiEvent::TextDelta("Conversation forked as: ...")`,
    ///     Ok(()).
    /// We assert Ok(()) invariant only, because test environments have
    /// varying DB state and we do not want to couple this test to the
    /// operator's `~/.archon/sessions.db`.
    #[test]
    fn fork_handler_execute_with_session_id_returns_ok_regardless_of_db_state() {
        let (mut ctx, _rx) =
            make_ctx(Some("fake-session-id-for-ags-815-test".to_string()));
        let h = ForkHandler;
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_ok(),
            "ForkHandler::execute(valid session_id) must return Ok(()) \
             regardless of session DB state (error branches emit \
             TuiEvent::Error and still return Ok), got: {res:?}"
        );
    }

    /// Verify that at least one event is emitted when executing with a
    /// session id. Most likely branch in test environments: the
    /// default DB opens but the fake id does not resolve -> Error
    /// variant. If the default DB does not open at all -> Error
    /// variant. If by coincidence the default DB has the fake id ->
    /// TextDelta. All three variants are valid.
    #[test]
    fn fork_handler_execute_with_session_id_emits_text_delta_or_error() {
        let (mut ctx, mut rx) =
            make_ctx(Some("fake-session-id-for-ags-815-test".to_string()));
        let h = ForkHandler;
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_ok(),
            "ForkHandler::execute must return Ok(()), got: {res:?}"
        );
        let mut emitted = false;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                TuiEvent::TextDelta(_) | TuiEvent::Error(_) => {
                    emitted = true;
                }
                other => panic!(
                    "ForkHandler emitted unexpected event variant: \
                     {other:?}"
                ),
            }
        }
        assert!(
            emitted,
            "ForkHandler must emit at least one TextDelta or Error \
             event for a named-arg miss path"
        );
    }

    /// Multi-token args are joined with a single space to preserve the
    /// shipped `s.strip_prefix(\"/fork\").trim()` semantics where
    /// `/fork my name` forks as the literal name `"my name"`. We cannot
    /// observe the fork_session call directly, but we CAN confirm the
    /// handler returns Ok(()) for multi-token input (no panic on
    /// `args.join(" ")` with zero, one, or many args).
    #[test]
    fn fork_handler_execute_accepts_multi_token_args_without_panicking() {
        let (mut ctx, _rx) = make_ctx(Some("fake-session-id".to_string()));
        let h = ForkHandler;
        let res = h.execute(
            &mut ctx,
            &["my".to_string(), "forked".to_string(), "name".to_string()],
        );
        assert!(
            res.is_ok(),
            "ForkHandler must accept multi-token args and join them \
             with spaces per shipped behaviour, got: {res:?}"
        );
    }
}
