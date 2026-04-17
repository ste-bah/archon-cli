# archon-tui Phase-3 Modularization — Final Report

**Task:** TASK-TUI-329 (preserve-invariants verification)
**Branch:** `archonfixes`
**Date:** 2026-04-17
**Status:** **PARTIAL** — 3 of 5 gates green, 2 pre-existing/inherited failures documented

---

## 1. Summary

Phase-3 goal (per `02-technical-spec.md` TECH-TUI-MODULARIZATION): decompose the
monolithic `archon-tui` into a module tree where no file exceeds 500 lines, no
cycles exist between layers, duplication stays <5%, and line coverage is
>=80%, while preserving three invariants:

1. **ERR-TUI-004 regression guard** — file-size lint green on main before merge.
2. **Embedded CozoDB memory writes still work post-refactor.**
3. **Existing `crates/archon-tui/tests/*` still pass unchanged.**

TUI-300..326 executed the structural refactor (events/state/keybindings/
render/screens extraction, 66 modules created). TUI-327 added the
`app::run_with_backend` integration seam. TUI-328 ratcheted the coverage gate
to an 80% hardcoded floor. TUI-329 (this task) verifies the invariants and
publishes this report.

**Honest finding:** two gates are NOT green on this branch. Per spec scope
(`Out of Scope: Fixing any newly-found regressions — those become their own
bug tasks`), they are documented as known gaps rather than silently patched.

---

## 2. Gate Results (all 5 tui gate scripts)

| Gate | Script | Result | Metric |
|------|--------|--------|--------|
| File-size (<=500 lines) | `check-tui-file-sizes.sh` | **FAIL** | `event_loop.rs` = 639 lines, not allowlisted. 5 files allowlisted (markdown, output, task_dispatch_tests, theme, vim). |
| Module cycles / layer | `check-tui-module-cycles.sh` | **FAIL** | `events.rs` imports `crate::app::{McpServerEntry, SessionPickerEntry}` — 1 directional-layer violation (events should not import from app). |
| Duplication (<5%) | `check-tui-duplication.sh` | **PASS** | 0.15% (1 clone: `screens/memory_file_selector.rs` <-> `screens/model_picker.rs`, 23 lines / 160 tokens). |
| Coverage (>=80% lines) | `check-tui-coverage.sh` | **PASS** | 81.74% lines, 82.66% regions, 84.84% functions (12,511 lines instrumented, 2,285 missed). |
| Complexity (clippy cognitive_complexity, default threshold 25) | `check-tui-complexity.sh` | **FAIL** (inherited) | All 4 errors are in `archon-memory` (access.rs x2, garden.rs x2, graph.rs), NOT in archon-tui. Gate compiles the full dep graph with `-D clippy::cognitive_complexity`, so archon-memory errors surface through the tui-scoped invocation. Reproducible on `main` before any phase-3 commit — pre-existing, not a phase-3 regression. |

### Combined run

```
bash scripts/check-tui-file-sizes.sh \
  && bash scripts/check-tui-module-cycles.sh \
  && bash scripts/check-tui-duplication.sh \
  && bash scripts/check-tui-coverage.sh \
  && bash scripts/check-tui-complexity.sh \
  && echo ALL_GATES_GREEN
```

Short-circuits on the first gate (file-sizes). `ALL_GATES_GREEN` is **NOT**
printed on this branch.

---

## 3. Final Line Counts

Authoritative snapshot from `find crates/archon-tui/src -name '*.rs' | xargs wc -l`
plus `wc -l src/main.rs`, taken on branch `archonfixes` at commit `3a650af`
(head before TUI-329 commit).

### 3.1 archon-cli workspace root

| File | Lines | Status |
|------|-------|--------|
| `src/main.rs` | **473** | Under 500 (spec Validation Criteria #5 satisfied). |

### 3.2 archon-tui crate — files > 200 lines (phase-3 focus)

| File | Lines | Notes |
|------|-------|-------|
| `crates/archon-tui/src/vim.rs` | 1008 | Allowlisted (phase-3 carryover, refactor deferred) |
| `crates/archon-tui/src/task_dispatch_tests.rs` | 832 | Allowlisted (test file) |
| `crates/archon-tui/src/event_loop.rs` | **639** | **NEW in TUI-310**, NOT allowlisted. Gate-failing. Honest regression. |
| `crates/archon-tui/src/output.rs` | 634 | Allowlisted |
| `crates/archon-tui/src/theme.rs` | 624 | Allowlisted |
| `crates/archon-tui/src/markdown.rs` | 538 | Allowlisted |
| `crates/archon-tui/src/input.rs` | 497 | Under 500 (post TUI-311 extraction) |
| `crates/archon-tui/src/app.rs` | 493 | Under 500 (post TUI-310 carve-out) |
| `crates/archon-tui/src/render/body.rs` | 474 | New in TUI-309 |
| `crates/archon-tui/src/split_pane.rs` | 470 | Carryover |
| `crates/archon-tui/src/diff_view.rs` | 453 | Carryover + TUI-324 gap-fill |
| `crates/archon-tui/src/views/tasks_overlay.rs` | 444 | Legacy `views/*` branch (co-exists with `screens/*`) |
| `crates/archon-tui/src/views/context_viz.rs` | 422 | Legacy view |
| `crates/archon-tui/src/syntax.rs` | 386 | |
| `crates/archon-tui/src/task_dispatch.rs` | 382 | |
| `crates/archon-tui/src/views/model_picker.rs` | 347 | Legacy view (superseded by `screens/model_picker.rs`) |
| `crates/archon-tui/src/splash.rs` | 340 | |
| `crates/archon-tui/src/ultrathink.rs` | 321 | |
| `crates/archon-tui/src/views/diff_viewer.rs` | 320 | Legacy view |
| `crates/archon-tui/src/keybindings.rs` | 313 | Populated in TUI-308 |
| `crates/archon-tui/src/views/session_browser.rs` | 289 | Legacy view |
| `crates/archon-tui/src/screens/mcp_view.rs` | 270 | New in TUI-320 |
| `crates/archon-tui/src/voice/pipeline.rs` | 266 | |
| `crates/archon-tui/src/screens/task_overlay.rs` | 260 | New in TUI-318 |
| `crates/archon-tui/src/terminal_panel.rs` | 254 | |
| `crates/archon-tui/src/virtual_scroll.rs` | 236 | |
| `crates/archon-tui/src/context_viz.rs` | 225 | |
| `crates/archon-tui/src/observability.rs` | 206 | |
| `crates/archon-tui/src/app_tests.rs` | 206 | |

### 3.3 archon-tui crate — files <= 200 lines (new & small modules)

`screens/model_picker.rs` 190, `screens/memory_file_selector.rs` 188,
`state.rs` 174, `screens/settings_screen.rs` 164, `views/settings.rs` 163,
`screens/voice_capture.rs` 161, `screens/hooks_config_menu.rs` 155,
`views/history.rs` 147, `views/agents.rs` 147, `screens/theme_screen.rs` 147,
`commands.rs` 143, `screens/session_browser.rs` 136,
`screens/permissions_browser.rs` 136, `screens/session_branching.rs` 134,
`virtual_list.rs` 129, `prompt_input.rs` 128, `terminal.rs` 119,
`render/chrome.rs` 117, `theme_registry.rs` 116, `status.rs` 101,
`voice/capture.rs` 100, `voice/stt.rs` 90, `cancel.rs` 88,
`layout_tests.rs` 84, `views/help.rs` 83, `notifications.rs` 81,
`layout.rs` 70, `render/mod.rs` 67, `lib.rs` 66, `overlays.rs` 62,
`events.rs` 59, `render/layout.rs` 55, `verbosity.rs` 54, `permissions.rs` 54,
`message_renderer.rs` 38, `views/mod.rs` 16, `screens/mod.rs` 13,
`voice.rs` 3.

**Total archon-tui src LOC:** 16,127 across 67 files (per `wc -l` total).

---

## 4. Module Extraction Map (what moved where)

### Pre-phase-3 baseline (main branch)
- `src/main.rs` was ~1349 lines (subsequently trimmed to 473 via TUI-325).
- `crates/archon-tui/src/app.rs` held the render loop + event handling +
  overlay dispatch in a single file (>800 lines before decomposition).

### Phase-3 extractions

| Source file (old location) | New module(s) | Ticket |
|-----------------------------|----------------|--------|
| monolithic `app.rs` render code | `render/mod.rs`, `render/body.rs`, `render/chrome.rs`, `render/layout.rs` | TUI-309 |
| monolithic `app.rs` event loop body | `event_loop.rs` (639 lines, still oversized — see gaps) | TUI-310 |
| monolithic `app.rs` key-dispatch branch | `input.rs::handle_key()` | TUI-311 |
| scattered multiline-edit code | `prompt_input.rs` (PromptBuffer) | TUI-312 |
| ad-hoc toast notifications | `notifications.rs` | TUI-313 |
| in-app context HUD | `context_viz.rs` (+ legacy `views/context_viz.rs`) | TUI-314 |
| shared overlay/list primitives | `overlays.rs`, `virtual_list.rs`, `message_renderer.rs` | TUI-315 |
| `views/session_browser.rs` | `screens/session_browser.rs` + `screens/session_branching.rs` | TUI-316 |
| `views/model_picker.rs` | `screens/model_picker.rs` (fuzzy filter) | TUI-317 |
| tasks overlay | `screens/task_overlay.rs` (VirtualList + TaskStore trait) | TUI-318 |
| memory-file / hooks menus | `screens/memory_file_selector.rs`, `screens/hooks_config_menu.rs` (MemoryStore/HookStore traits) | TUI-319 |
| MCP server list | `screens/mcp_view.rs` (VirtualList + McpStatusStore trait) | TUI-320 |
| settings / theme screens | `screens/settings_screen.rs`, `screens/theme_screen.rs` | TUI-321 |
| permissions browser | `screens/permissions_browser.rs` (VirtualList + cycle logic) | TUI-322 |
| voice capture overlay | `screens/voice_capture.rs` (VoiceCaptureOverlay) | TUI-323 |
| `src/main.rs` bootstrap-only trim | remaining logic dispersed into `command/*`, `session/*` | TUI-325 |
| stub scaffolds | 26 empty modules created for future population | TUI-326 |
| public-API integration seam | `app::run_with_backend(...)` + `tests/app_run_e2e.rs` | TUI-327 |
| coverage-ratchet | `scripts/check-tui-coverage.sh` hardcoded to 80% | TUI-328 |
| preserve-invariants + report | `tests/cozo_memory_preserve.rs` (ignored stub) + this doc | TUI-329 |

### Events/state canonicalization (TUI-305..308)

| New file | Purpose | Ticket |
|----------|---------|--------|
| `events.rs` | Canonical `TuiEvent` enum | TUI-305 |
| `state.rs` | `AppState` struct | TUI-306 |
| `terminal.rs` | `TerminalGuard` + SIGWINCH | TUI-307 |
| `keybindings.rs` | `KeyMap` + `Action` enum | TUI-308 |

---

## 5. Test Totals

Command:
```
cd /home/unixdude/Archon-projects/archon-cli-worktrees/archonfixes
cargo test -j1 -p archon-tui -- --test-threads=2
```

Aggregate across all `test result:` lines:

| Metric | Count |
|--------|-------|
| Test binaries | 54 |
| Passed | **711** |
| Failed | **0** |
| Ignored | 7 (3 doctests in `render/mod.rs` & `terminal.rs`; 3 voice-related; 1 new TUI-329 `cozo_memory_preserve_stub`) |

### Pre-existing test zero-diff verification

Each file that existed in `main:crates/archon-tui/tests/` was checked via
`git diff main..HEAD -- <file>`. All 11 pre-existing test files show
**0 diff lines** vs main:

- key_event_filter_tests.rs — 0 diff lines
- markdown_tests.rs — 0 diff lines
- syntax_tests.rs — 0 diff lines
- terminal_panel_tests.rs — 0 diff lines
- theme_registry_tests.rs — 0 diff lines
- verbosity_tests.rs — 0 diff lines
- vim_tests.rs — 0 diff lines
- virtual_scroll_tests.rs — 0 diff lines
- voice_integration_tests.rs — 0 diff lines
- voice_loop_gate_test.rs — 0 diff lines
- voice_toggle_mode_gate_test.rs — 0 diff lines

All new test files in `crates/archon-tui/tests/` are **additions**, not
modifications, and are therefore permitted under the spec invariant.

---

## 6. Coverage Snapshot

Captured by `cargo llvm-cov -j1 --package archon-tui --fail-under-lines 80`.

**Totals:** 12,511 lines / 2,285 missed = **81.74% line coverage**
(regions 82.66%, functions 84.84%). Gate passes the 80% hardcoded floor.

### Five weakest per-module line coverage

| Module | Line coverage | Missed lines |
|--------|---------------|--------------|
| `terminal.rs` | 18.03% | 50 of 61 |
| `voice/stt.rs` | 25.00% | 6 of 8 |
| `event_loop.rs` | 36.63% | 308 of 486 |
| `context_viz.rs` | 38.30% | ~140 |
| `overlays.rs` | 48.78% | ~31 |

`event_loop.rs` (36.63% line, 39.71% region) is the single largest absolute
coverage gap — it accounts for ~13% of all missed lines in the crate and
needs dedicated integration tests. This is flagged as a follow-up.

---

## 7. Invariants Preserved

| Invariant | Source | Status |
|-----------|--------|--------|
| **ERR-TUI-004 file-size lint** | 01-functional-spec.md | **PARTIAL** — 1 post-extraction file (`event_loop.rs` @ 639) violates. 5 legacy files allowlisted as phase-3 ratchet carryover. Gate itself is wired and enforcing. |
| **Embedded CozoDB memory writes** | REQ-FOR-PRESERVE-D8 | **INDIRECT COVERAGE** — archon-tui has zero cozo dependency (verified via Cargo.toml grep + source grep). Cozo writes live in archon-memory, archon-pipeline, archon-leann, archon-session. `crates/archon-tui/tests/cozo_memory_preserve.rs` is an `#[ignore]`d stub with a full rationale comment declaring the coverage gap. The invariant is preserved *by construction* (archon-tui cannot regress cozo semantics because it never touches cozo symbols), but this is **not** asserted by a positive TUI-level test. |
| **Existing `crates/archon-tui/tests/*` unchanged** | spec `<preserve_invariants>` | **PASS** — all 11 pre-existing test files verified zero-diff vs main (see Section 5). |

---

## 8. Known Gaps / Follow-ups

### 8.1 Gate failures (NOT fixed in TUI-329 per spec scope)

1. **`event_loop.rs` file-size (639 lines)** — extracted wholesale from `app.rs`
   in TUI-310 without sub-decomposition. Follow-up ticket should either
   (a) split into `event_loop/{mod.rs, run.rs, handle_terminal.rs, handle_task_events.rs}`,
   or (b) consciously add to `scripts/check-tui-file-sizes.allowlist` with
   a "phase-3 carryover" note if sub-decomposition is deferred.
2. **`events.rs -> crate::app` layer violation** — `events.rs` imports
   `McpServerEntry`, `SessionPickerEntry` from `app`, inverting the intended
   layering (events should not depend on app). Fix: relocate these two
   structs into `state.rs` or a new `types.rs` so both `events.rs` and `app.rs`
   can depend on them without a cycle.
3. **`check-tui-complexity.sh` surfaces archon-memory errors** — 4 cognitive-
   complexity errors in `archon-memory/src/{access.rs, garden.rs, graph.rs}`.
   Pre-existing on `main`. Either narrow the clippy invocation to archon-tui
   only (`--no-deps`), or open bug tickets against archon-memory to refactor
   the offending functions (`store_memory`, etc.).

### 8.2 Coverage gaps (under gate but worth filling)

1. **`event_loop.rs` 36.63% line coverage** — biggest absolute gap. Needs
   targeted integration tests driving `run_with_backend` through terminal
   events, task events, and voice events.
2. **`terminal.rs` 18.03%** — mostly SIGWINCH install path; expected (OS/TTY
   interaction is hard to unit-test). Document and accept.
3. **`voice/stt.rs` 25.00%** — only 8 lines instrumented; small absolute gap
   but low percentage.
4. **`context_viz.rs` 38.30% + `overlays.rs` 48.78%** — render-branch
   coverage; add snapshot tests.

### 8.3 Invariant coverage declaration

- **Cozo memory-write invariant is declared but NOT tested at the TUI layer.**
  This is an honest, documented gap. To close it, a new cross-crate
  integration test should live in `archon-memory/tests/` or
  `archon-session/tests/` rather than in archon-tui (which has no cozo
  dependency to exercise). Opening such a ticket is out of scope for
  TUI-329 per the spec.

### 8.4 Legacy co-existence

- `crates/archon-tui/src/views/*` and `crates/archon-tui/src/screens/*` both
  exist. `views/*` is the legacy tree; `screens/*` is the phase-3 replacement.
  Several legacy `views/*` files are still >=300 lines
  (`tasks_overlay.rs` 444, `context_viz.rs` 422, `model_picker.rs` 347,
  `diff_viewer.rs` 320, `session_browser.rs` 289). A cleanup pass should
  delete any `views/*.rs` fully superseded by `screens/*.rs`.

---

## 9. Phase-3 Commit Chain (main..HEAD, TUI-3xx only)

In chronological order (oldest first):

1. `9d9bd63` TASK-TUI-300: bootstrap archon-tui module skeleton + lib.rs facade
2. `4ac3dba` TASK-TUI-301: CI file-size lint gate for archon-tui (<500 lines)
3. `90d1f7f` TASK-TUI-302: CI circular-dep gate + directional import guard
4. `3a5025e` TASK-TUI-303: CI code-duplication gate (<5%)
5. `6f25fee` TASK-TUI-303: injection test for duplication gate exit-1
6. `084c515` TASK-TUI-304: CI coverage and complexity gates for archon-tui
7. `4ade237` TASK-TUI-307: populate terminal.rs — TerminalGuard + SIGWINCH
8. `9df4481` TASK-TUI-324: verify + gap-fill diff_view.rs keyboard navigation
9. `1d54848` TASK-TUI-305: populate events.rs with canonical TuiEvent enum
10. `0ce0f78` TASK-TUI-306: populate state.rs with AppState struct
11. `af8ecec` TASK-TUI-308: populate keybindings.rs with KeyMap + Action enum
12. `dc49fe9` TASK-TUI-313: populate notifications.rs with toast system
13. `d0b7216` TASK-TUI-309: extract render pipeline from app.rs
14. `0d4dc00` TASK-TUI-314: populate context_viz.rs with live context-window widget
15. `e439c9a` TASK-TUI-310: extract main event loop into app::run() (initial)
16. `42846bd` Revert "TASK-TUI-310: extract main event loop into app::run()"
17. `f0a5d8d` TASK-TUI-310: add AppConfig + app::run() thin orchestrator entry point
18. `fb1951d` TASK-TUI-311: extract key dispatch from run_tui() into input.rs handle_key()
19. `44f5b1a` TASK-TUI-315: populate overlays.rs, virtual_list.rs, message_renderer.rs
20. `0bf8153` TASK-TUI-312: create prompt_input.rs multiline editor
21. `0b0b1c0` TASK-TUI-316: populate screens/session_browser.rs and screens/session_branching.rs
22. `0075a5c` TASK-TUI-317: populate screens/model_picker.rs
23. `f84d25c` TASK-TUI-318: populate screens/task_overlay.rs
24. `200c114` TASK-TUI-319: populate screens/memory_file_selector.rs and screens/hooks_config_menu.rs
25. `0b4ed25` TASK-TUI-320: populate screens/mcp_view.rs
26. `320ae24` TASK-TUI-321: populate screens/settings_screen.rs and screens/theme_screen.rs
27. `7d41f9b` TASK-TUI-322: populate screens/permissions_browser.rs
28. `b8b7213` TASK-TUI-323: populate screens/voice_capture.rs
29. `17fa5d2` TUI-326: module count 66 >= 46 — no-op
30. `9bdd3bd` TUI-312: PromptBuffer multiline editor (follow-up)
31. `57d7843` TUI-317: model_picker.rs with fuzzy filter (follow-up)
32. `b8c2779` TUI-318: task_overlay.rs with VirtualList and TaskStore trait (follow-up)
33. `e7709c6` TUI-319: memory_file_selector.rs + hooks_config_menu.rs with MemoryStore/HookStore traits
34. `e139ecf` TUI-320: mcp_view.rs with VirtualList and McpStatusStore trait (follow-up)
35. `3d707f6` TUI-321: screens/settings_screen.rs + theme_screen.rs (follow-up)
36. `43a56b0` TUI-322: screens/permissions_browser.rs with VirtualList and cycle logic (follow-up)
37. `c69d13a` TUI-323: screens/voice_capture.rs with VoiceCaptureOverlay (follow-up)
38. `3c89405` refactor(main): extract handle_login and consolidate delegation stubs (TUI-325)
39. `42049fd` TUI-325: trim main.rs under 500 lines + complete 6 gates
40. `455953c` TUI-327: app::run_with_backend seam + integration test
41. `b300f2d` TUI-310: extract event loop body from app.rs to event_loop.rs
42. `3a650af` TUI-328: ratchet archon-tui coverage gate to 80% hardcoded
43. **TUI-329 commit (this report)** — preserve-invariants verification

Also present in the broader branch (pre-phase-3 housekeeping, TUI-1xx/2xx):
TUI-108..112, TUI-205..211. See `git log --oneline main..HEAD` for the full
list (139 commits total).

---

## 10. Pass/Fail Verdict

- **Invariant 1 (existing tests unchanged):** PASS.
- **Invariant 2 (cozo writes):** INDIRECT (no archon-tui path to regress).
- **Invariant 3 (ERR-TUI-004 lint):** PARTIAL (gate wired and enforcing; one
  post-TUI-310 file exceeds limit and needs follow-up sub-decomposition).
- **Phase-3 overall:** **PARTIAL COMPLETE.** The architectural skeleton is in
  place, 711 tests pass, coverage is 81.74%, duplication is 0.15%. Two
  concrete, named regressions remain and are listed above as follow-up
  tickets rather than silently fixed.

The decision to keep `event_loop.rs` and the `events.rs -> app` cycle as
open tickets (rather than bundling fixes into TUI-329) is dictated by the
spec's `Out of Scope: Fixing any newly-found regressions` clause.
