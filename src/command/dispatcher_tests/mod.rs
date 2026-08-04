use super::*;
use crate::command::registry::{CommandHandler, RegistryBuilder, default_registry};
use archon_tui::app::TuiEvent;
use std::sync::Mutex;
use tokio::sync::mpsc;

/// Build a fresh `CommandContext` backed by a bounded channel the
/// test can drain via `try_recv`. Capacity of 8 matches the real
/// input pipeline order of magnitude while leaving headroom.
fn make_ctx() -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
    // TASK-AGS-POST-6-SHARED-FIXTURES-V2: migrated to CtxBuilder.
    // The builder uses capacity 16 (was 8); dispatcher tests emit
    // at most a handful of events, so observational behavior is
    // unchanged.
    crate::command::test_support::CtxBuilder::new().build()
}

/// A test-only handler that records every `execute` invocation so
/// the test can assert both that it was called and with which args.
struct RecordingHandler {
    calls: Arc<Mutex<Vec<Vec<String>>>>,
}

impl CommandHandler for RecordingHandler {
    fn execute(&self, _ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        self.calls.lock().unwrap().push(args.to_vec());
        Ok(())
    }
    fn description(&self) -> &str {
        "recording handler (test only)"
    }
}

// -----------------------------------------------------------------
// Recognized / unknown / non-slash paths
// -----------------------------------------------------------------

/// Test-local handler that mirrors the THIN-WRAPPER no-op contract:
/// `execute` returns `Ok(())` WITHOUT emitting any `TuiEvent`. Used
/// by `dispatch_recognized_command_returns_ok` below so the witness
/// test is INDEPENDENT of any specific production command stub —
/// TASK-AGS-POST-6-NO-STUB has removed the last `declare_handler!`
/// stubs from the registry, so every previous swap target is now a
/// real (or byte-identically-wrapped) handler with observable
/// behavior we must not cargo-cult into this generic witness. Shape
/// mirrors `RecordingHandler` above (test-local) and
/// `registry::tests::NoAliasHandler`.
struct SilentOkHandler;
impl CommandHandler for SilentOkHandler {
    fn execute(&self, _ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        Ok(())
    }
    fn description(&self) -> &str {
        "silent ok handler (test only)"
    }
}

struct FailingHandler;
impl CommandHandler for FailingHandler {
    fn execute(&self, _ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        anyhow::bail!("workflow command requires working directory context")
    }
    fn description(&self) -> &str {
        "failing handler (test only)"
    }
}

// -----------------------------------------------------------------
// Argument passing (parser composition)
//
// Registry has no public "insert" API and TASK-AGS-623 is
// out-of-scope for registry.rs changes, so these two tests
// exercise the exact composition `Dispatcher::dispatch` performs
// (parser::parse → handler.execute(raw_args)) against a fake handler
// directly, rather than round-tripping through a custom Registry.
// This still guarantees that the parser output is faithfully
// forwarded to handler.execute — which is the contract under test.
// -----------------------------------------------------------------

fn invoke_handler_via_parse(handler: &dyn CommandHandler, input: &str) -> anyhow::Result<()> {
    let parsed = crate::command::parser::parse(input).expect("parser must accept input");
    let (mut ctx, _rx) = make_ctx();
    handler.execute(&mut ctx, &parsed.raw_args)
}

// -----------------------------------------------------------------
// TASK-AGS-POST-6-DISPATCH-SMOKE: end-to-end dispatcher coverage.
//
// The body-migrate stream (B01..B24) finished with 40 primaries
// routed through `Dispatcher::dispatch`. The AGS-POST-6-FALLTHROUGH
// ticket then deleted the legacy 477-line slash.rs match, leaving
// the dispatcher as the single routing authority. What we were
// missing up to this point was a loop-the-registry smoke covering
// EVERY primary + EVERY alias in one pass. Unit-per-handler tests
// (one per command body file) each prove their own slice, but
// nothing in the suite pinned "iterate the whole catalog, confirm
// none of them hit the dispatch-layer 'Unknown command' branch".
//
// The four tests below close that gap:
//
//   * `dispatch_smoke_all_primaries_route_without_unknown_error`
//     — loops every registered primary name, dispatches `/{name}`
//       with a fresh channel per iteration, and asserts that the
//       emitted-event stream contains NO `TuiEvent::Error(msg)`
//       whose `msg` begins with `"Unknown command"`. Handler-level
//       Err is tolerated (most handlers need populated context
//       fields this fixture deliberately leaves at `None`); the
//       smoke is strictly a DISPATCH-LAYER miss detector.
//
//   * `dispatch_smoke_all_aliases_route_without_unknown_error`
//     — walks the (primary, alias) space using the same strategy
//       as `registry_integration_all_commands_wired` (registry.rs
//       :2597) — `registry.names()` + `handler.aliases()` — and
//       asserts the same "no dispatch-layer Unknown command" for
//       every alias. Closes the contract that the alias map is
//       exhaustively reachable via the dispatcher.
//
//   * `recognizes_smoke_all_primaries_return_true`
//     — cheap: for every primary `/{name}`, `recognizes` must be
//       true. Pairs with `recognizes_returns_true_for_registered_name`
//       (single-sample witness) and lifts it to FULL coverage.
//
//   * `registry_primary_count_matches_expected_count`
//     — defensive regression guard. If a future refactor silently
//       drops or doubles a primary, this fails IMMEDIATELY without
//       needing a full dispatch loop. Numeric witness pinned to
//       the registry-side `EXPECTED_COMMAND_COUNT` constant in
//       registry.rs's #[cfg(test)] block; the two constants must
//       move in lockstep when a primary is added or removed.
//
// Failure-report strategy mirrors `registry_integration_all_commands_wired`
// (registry.rs:2564) — collect-and-report, so a single run surfaces
// every broken command/alias simultaneously instead of panicking at
// the first failure.
// -----------------------------------------------------------------

/// Canonical primary-count invariant. Mirrors
/// `registry::tests::EXPECTED_COMMAND_COUNT` in registry.rs's
/// #[cfg(test)] block. That constant is not re-exported, so we pin
/// the same integer here. **If either constant moves, BOTH must be
/// updated in lockstep** — see TASK-#211 commit body for the
/// regression where #206/#215/#210 each bumped the registry-side
/// constant without updating this dispatcher mirror.
///
/// Sequence: 49 → 50 (#206) → 51 (#215) → 52 (#210) → 53 (#211)
/// → 54 (#212) → 55 (#213) → 56 (#214) → 57 (#216) → 58 (#217)
/// → 59 (#207) → 60 (#208) → 61 (#209) → 65 (Phase 5+6: completion +
/// behaviour primaries) → 76 (v0.1.38 Evidence Engine: kb, prov,
/// meaning, constellation primaries + gametheory inspection
/// subcommands and slash mirrors) → 78 (v0.1.40 Codex auth: /auth
/// + /chat primaries for the OpenAI-Codex provider surface) → 80
/// (v1.2.0 reasoning quality: /reasoning + /briefing) → 81
/// (v1.3.3 video evidence: /video) → 82 (PRD-008 cognitive loop: /cognitive)
/// → 83 (PRD-009 dynamic workflows: /workflow)
/// → 84 (v1.3.11 Trading Lab: /trading)
/// → 85 (FCDP in-session drafting: /draft)
/// → 87 (Phase 6 traceability: /requirements).
const EXPECTED_PRIMARY_COUNT: usize = 87;

/// Drain every currently-queued event from `rx` using `try_recv`
/// until the channel reports empty, returning the drained events
/// in FIFO order. The smoke tests below call this once per
/// dispatch so handler-emitted events do not leak into the next
/// iteration.
fn drain_events(
    rx: &mut archon_tui::event_channel::TuiEventReceiver,
) -> Vec<archon_tui::app::TuiEvent> {
    let mut out = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(ev) => out.push(ev),
            Err(mpsc::error::TryRecvError::Empty) => return out,
            Err(mpsc::error::TryRecvError::Disconnected) => return out,
        }
    }
}

/// Return the first `TuiEvent::Error(msg)` whose `msg` begins with
/// `"Unknown command"` — the exact prefix the dispatcher's
/// unknown-command branch emits via
/// `errors::format_unknown_command` (TASK-AGS-804). Returns `None`
/// when the event stream has no dispatch-layer miss, which is the
/// smoke-test pass condition.
fn first_unknown_command_error(events: &[archon_tui::app::TuiEvent]) -> Option<String> {
    events.iter().find_map(|ev| match ev {
        archon_tui::app::TuiEvent::Error(msg) if msg.starts_with("Unknown command") => {
            Some(msg.clone())
        }
        _ => None,
    })
}

/// Drive one `(primary_or_alias, input)` through the dispatcher
/// with a fresh channel, wrap the call in `catch_unwind` so
/// handler-internal panics (e.g. DenialsHandler's
/// `.expect("denial_snapshot populated")` on a stripped fixture —
/// denials.rs:151) do NOT abort the whole smoke sweep, and
/// return any drained `TuiEvent::Error("Unknown command…")` as a
/// failure candidate.
///
/// Why `catch_unwind` is sound here:
///   * Several handlers (`DenialsHandler`, `McpHandler`,
///     `CopyHandler`, …) explicitly `.expect()` on missing
///     context fields — the author's stated intent is "panic to
///     surface wiring bugs LOUDLY at test-time". Our dispatcher
///     test fixture deliberately leaves those fields at `None`,
///     so those handlers WILL panic under this smoke. That is
///     out-of-scope for a DISPATCH-LAYER smoke — we only care
///     whether `Dispatcher::dispatch` routed the input to a
///     handler at all (versus emitting the dispatch-layer
///     "Unknown command" error). `catch_unwind` lets us treat
///     "handler ran, then panicked" as SUCCESS for routing —
///     which is what we want.
///   * We pass `AssertUnwindSafe` because `CommandContext` holds
///     a `tokio::sync::mpsc::Sender` which is not
///     `UnwindSafe`. That is fine: the ctx is about to be
///     dropped, and any handler-level panic leaves it in a
///     well-defined state (same-or-fewer events in the channel).
///
/// Returns `Some(msg)` if the dispatcher emitted an
/// "Unknown command" error (the real failure mode we are
/// hunting), otherwise `None` — regardless of whether the
/// handler succeeded, returned Err, or panicked.
fn smoke_dispatch_detect_unknown_error(dispatcher: &Dispatcher, input: &str) -> Option<String> {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let (mut ctx, mut rx) = make_ctx();
    // NOTE: `catch_unwind` lets a handler panic without aborting
    // the smoke sweep, but the process-wide panic hook still
    // runs — so each trapped panic prints a stack header to
    // stderr. That is acceptable (the output is still a PASS
    // for cargo) and preferable to `set_hook`/`take_hook` here:
    // the panic hook is PROCESS-wide, and with `--test-threads=2`
    // swapping it under the primaries smoke would race the
    // aliases smoke (or any other concurrently-running test
    // whose panic output we would then lose). Keeping the
    // default hook means correctness trumps output quietness.
    let _result = catch_unwind(AssertUnwindSafe(|| {
        let _ = dispatcher.dispatch(&mut ctx, input);
    }));
    let events = drain_events(&mut rx);
    first_unknown_command_error(&events)
}

mod cases_a;
mod cases_b;
