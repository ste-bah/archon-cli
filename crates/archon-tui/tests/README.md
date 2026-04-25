# archon-tui — integration and regression tests

This directory holds every integration test, snapshot test, and
regression guard for the `archon-tui` crate. Cargo auto-detects any
`*.rs` file in `tests/` as a separate test binary (no `[[test]]` entry
in `Cargo.toml` required), so adding a new guard is just dropping a
new file here and following the naming conventions below.

## Regression Guards

These tests exist specifically to stop known failure modes from
reappearing. Each one is keyed to a ticket in
`project-tasks/archon-fixes/tui_fixes/` and — unlike coverage tests —
they deliberately assert on source-code shape, not runtime behaviour,
so they also catch regressions that would never reach a runtime path
during `cargo test`.

| Ticket / File | Purpose |
|---------------|---------|
| TASK-TUI-900 — `preserve_no_await_on_send_gate.rs` | Bans `.await` on any `_tx` / `_sender` / `_channel` producer `.send(...)` callsite under `crates/archon-tui/src/` (ERR-TUI-001 bounded-channel deadlock guard). Scope: files matching `*event*`, `*channel*`, `*agent*`, `*input*`, `*subagent*`. Whitelist documented inline. |
| TASK-TUI-901 — `preserve_auto_background_d5.rs` | Reserved: asserts the auto-background-after-Nms spawn-path (REQ-TUI-DISP-D5) stays wired through `AgentDispatcher::spawn_turn`. Add when TASK-TUI-901 lands. |
| TASK-TUI-902 — `preserve_cozodb_d8.rs` | Reserved: asserts the CozoDB session-persistence D8 hook stays wired. Add when TASK-TUI-902 lands. |
| TASK-TUI-903 — `preserve_500_loc_cap.rs` | Reserved: asserts no file under `src/` exceeds the 500-LoC ceiling (REM-2 split budget). Add when TASK-TUI-903 lands. |
| TASK-TUI-904 — `preserve_error_kinds.rs` | Reserved: asserts the `ErrorKind` enum variants required by the TUI error taxonomy still exist and are reachable. Add when TASK-TUI-904 lands. |
| TASK-TUI-905 — `preserve_stage9_integration_audit.rs` | Reserved: end-to-end integration check that all Stage 9 guards run and pass together. Add when TASK-TUI-905 lands. |

When you add a new guard:

1. Name the file `preserve_<topic>.rs` so `cargo test preserve_` filters
   the whole PRESERVE suite.
2. Add a row to the table above with the ticket id and a one-line
   purpose.
3. Reference the ticket at the top of the file so future maintainers
   can find the spec.
4. Make the failure message contain the ticket id and the
   word `VIOLATION` — CI log-scrapers key on both.

## Other tests

Everything else in this directory is either an integration smoke test,
a snapshot test, or a benchmark driver. The naming convention is:

- `*_tests.rs` — feature-level integration tests.
- `*_smoke.rs` — minimal end-to-end reachability checks.
- `*_e2e.rs` — full-flow tests driving the public API as a user would.
- `*_statistical.rs` / `*_p95.rs` — latency / histogram harnesses that
  read `HDRHistogram` percentiles.
- `snapshots/` — `insta`-managed golden outputs.
- `common/` — shared test helpers (see `common/mod.rs`).
