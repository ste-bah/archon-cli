//! TASK-AGS-POST-6-BODIES-B03-BUG: /bug slash-command handler
//! (Option C, DIRECT pattern body-migrate — trivial variant).
//!
//! Real `CommandHandler` impl moved here from the `declare_handler!`
//! stub at `src/command/registry.rs:658` and the legacy match arm at
//! `src/command/slash.rs:349-356`. The body is the simplest possible
//! slash-command migration in the codebase: zero args, zero state,
//! single constant-string TextDelta emission.
//!
//! # Why DIRECT (trivial variant)
//!
//! The shipped `/bug` body at slash.rs:349-356 is the simplest possible
//! slash-command body in the codebase:
//!
//! ```ignore
//! "/bug" => {
//!     let _ = tui_tx
//!         .send(TuiEvent::TextDelta(
//!             "\nReport bugs at https://github.com/anthropics/archon/issues\n".into(),
//!         ))
//!         .await;
//!     true
//! }
//! ```
//!
//! - NO args (exact-match `"/bug"` arm — no subcommand parse)
//! - NO shared state (no `Arc<AtomicBool>`, no `Arc<Mutex<T>>`)
//! - NO `CommandContext` field added (no new cross-cutting plumbing)
//! - NO snapshot (no read-side state to pre-compute)
//! - NO `CommandEffect` variant (no write-side state to defer)
//! - NO preceding `ThinkingToggle`-style event — SINGLE TextDelta
//! - NO aliases — shipped pre-B03-BUG stub carries none; spec lists none
//!
//! Simpler than B01-FAST (which had an atomic toggle) and B02-THINKING
//! (which had subcommand parsing + two-event emission). The only
//! structural change from shipped is the emission primitive:
//! `tui_tx.send(..).await` → `ctx.tui_tx.try_send(..)` per B01/B02
//! precedent. The emitted TextDelta string is byte-identical to the
//! shipped literal at slash.rs:351-353 — Sherlock Gate 3 will MD5-verify.
//!
//! # Byte-for-byte output preservation
//!
//! Migrated string (handler body reproduces this exactly):
//! ```ignore
//! "\nReport bugs at https://github.com/anthropics/archon/issues\n"
//! ```
//!
//! The leading and trailing `\n` wrap MUST be preserved.
//!
//! # Trailing-args policy (decision rationale)
//!
//! The shipped arm at slash.rs:349 matches exactly `"/bug"` and would
//! fall through for `"/bug foo"` to the default "unknown command"
//! handler. Post-migration, ALL `/bug*` inputs route to `BugHandler`
//! via the registry. The chosen preservation strategy is: **ignore
//! trailing args and always emit the URL**. Simpler code, better UX
//! (the user gets help even if they add arguments), and the legacy
//! fall-through was a dispatch quirk, not a documented contract. This
//! is a mild semantic promotion from the shipped behavior, documented
//! in the ticket at TASK-AGS-POST-6-BODIES-B03-BUG.md lines 130-140.
//!
//! The Gate 2 test `bug_handler_ignores_trailing_args` pins this
//! contract — passing `args=["foo"]` must produce the same single
//! TextDelta as `args=[]`.
//!
//! # try_send vs send().await
//!
//! Emission primitive: `ctx.tui_tx.try_send(..)` (sync) instead of the
//! legacy `tui_tx.send(..).await` (async). Matches every peer migrated
//! handler (AGS-806..819) and B01-FAST / B02-THINKING. `/bug` output is
//! best-effort informational UI — dropping a message under 16-cap
//! channel backpressure is preferable to stalling the dispatcher.
//!
//! # Aliases
//!
//! Shipped pre-B03-BUG: none. Spec lists none. No aliases added.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// Zero-sized handler registered as the primary `/bug` command.
///
/// No aliases. Shipped pre-B03-BUG stub carried none; spec lists none.
/// Trailing args are intentionally ignored (see module rustdoc
/// "Trailing-args policy").
pub(crate) struct BugHandler;

impl CommandHandler for BugHandler {
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        // Single TextDelta emission. Byte-for-byte preserved from
        // slash.rs:351-353 — leading and trailing `\n` wrap PRESERVED.
        // Sherlock Gate 3 will MD5 this literal against the shipped
        // string to prove byte equivalence.
        //
        // Trailing args intentionally ignored (see module rustdoc):
        // `_args` underscore-prefixed to signal the unused binding.
        ctx.emit(TuiEvent::TextDelta(
            "\nReport bugs at https://github.com/anthropics/archon/issues\n".to_string(),
        ));
        Ok(())
    }

    fn description(&self) -> &str {
        // Byte-identical to the shipped registry.rs:658 stub
        // description — preserves shipped-wins drift-reconcile.
        "Report a bug with current session context"
    }
}

#[cfg(test)]
mod tests {
    // Gate 2 real tests. Replace the Gate 1 `#[ignore]` + `todo!()`
    // skeleton with real assertions against the landed BugHandler impl.
    // Uses the `make_bug_ctx` helper added to `test_support.rs` in this
    // gate. Test names preserved from Gate 1 skeleton for traceability.

    use super::*;
    use crate::command::registry::CommandHandler;
    use crate::command::test_support::*;
    use archon_tui::app::TuiEvent;

    #[test]
    fn bug_handler_execute_emits_bug_url_textdelta() {
        let (mut ctx, mut rx) = make_bug_ctx();
        BugHandler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "expected exactly one event; got: {:?}",
            events
        );
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                assert!(
                    s.contains("https://github.com/anthropics/archon/issues"),
                    "expected bug URL in TextDelta; got: {:?}",
                    s
                );
                assert!(
                    s.starts_with('\n') && s.ends_with('\n'),
                    "expected leading+trailing \\n wrap; got: {:?}",
                    s
                );
            }
            other => panic!("expected TuiEvent::TextDelta, got: {:?}", other),
        }
    }

    #[test]
    fn bug_handler_ignores_trailing_args() {
        let (mut ctx_a, mut rx_a) = make_bug_ctx();
        let (mut ctx_b, mut rx_b) = make_bug_ctx();
        BugHandler.execute(&mut ctx_a, &[]).unwrap();
        BugHandler
            .execute(&mut ctx_b, &[String::from("foo")])
            .unwrap();
        let events_a = drain_tui_events(&mut rx_a);
        let events_b = drain_tui_events(&mut rx_b);
        // Both must emit exactly one TextDelta with byte-identical
        // payload — preserves legacy "no state change, no output
        // divergence for trailing args" semantics (mild semantic
        // promotion from shipped exact-match-only arm; see module
        // rustdoc "Trailing-args policy").
        assert_eq!(
            events_a.len(),
            1,
            "args=[] must emit exactly one event; got: {:?}",
            events_a
        );
        assert_eq!(
            events_b.len(),
            1,
            "args=[\"foo\"] must emit exactly one event; got: {:?}",
            events_b
        );
        match (&events_a[0], &events_b[0]) {
            (TuiEvent::TextDelta(a), TuiEvent::TextDelta(b)) => {
                assert_eq!(
                    a, b,
                    "trailing args must not change TextDelta payload — \
                     byte-identical to args=[] case"
                );
            }
            _ => panic!(
                "expected both events to be TextDelta; got: a={:?}, b={:?}",
                events_a[0], events_b[0]
            ),
        }
    }
}
