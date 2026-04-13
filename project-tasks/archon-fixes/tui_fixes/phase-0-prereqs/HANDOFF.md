# Phase-0 Handoff to Phases 1..9 (TUI Refactor)

This document is the canonical Phase-0 closeout handoff produced by
TASK-TUI-011. It enumerates, for each downstream phase (1..9), the
exact Phase-0 artefacts that the phase is permitted to consume as
evidence when it runs its own dev-flow gates. Phase 0 built the
infrastructure baselines, mock harnesses, fixture corpora, and CI
guards (TASK-TUI-001 through TASK-TUI-010); this handoff gives every
downstream phase a single place to look up "which TASK-TUI-NNN do I
cite, and how do I inject its hash into my gate evidence?". The
companion machine-readable snapshot is
`baselines/phase0-capture-manifest.json`, which pins sha256 hashes of
`loc-baseline.json` and `bench-eventloop-baseline.json` so downstream
phases can detect silent drift.

## Phase-1 — Eventloop

- Consumes: TASK-TUI-007 (TestBackend ratatui harness), TASK-TUI-008
  (mock LLM provider), TASK-TUI-009 (criterion bench harness
  `eventloop_throughput`).
- Description: the eventloop decomposition phase uses the TestBackend
  harness to assert terminal output determinism, the mock LLM provider
  to drive eventloop transitions without real network I/O, and the
  criterion bench to guard the throughput regression budget.
- Evidence injection: include
  `$(sha256sum project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/baselines/bench-eventloop-baseline.json)`
  in the Phase-1 gate-5 live-smoke evidence, and reference the
  TestBackend harness path and mock_agent module in gate-3 review
  evidence.

## Phase-2 — Eventchannel

- Consumes: TASK-TUI-004 (banned-patterns CI gate), TASK-TUI-008
  (mock_agent LLM provider), TASK-TUI-009 (criterion bench
  `eventloop_throughput`).
- Description: the eventchannel phase splits the single mpsc event
  pipe into bounded channels; the banned-patterns gate enforces that
  no `unwrap()`/`expect()`/`.await`-in-render regressions leak in, the
  mock_agent drives channel consumers under test, and the eventloop
  bench ensures channel restructuring does not exceed the throughput
  budget.
- Evidence injection: include
  `$(sha256sum project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/baselines/bench-eventloop-baseline.json)`
  in gate-5 evidence, and cite the TASK-TUI-004 banned-patterns
  allowlist delta (must be empty) in gate-3 evidence.

## Phase-3 — Modularization

- Consumes: TASK-TUI-002 (loc baseline), TASK-TUI-003 (file-size CI
  gate), TASK-TUI-006 (insta snapshot corpus), TASK-TUI-007
  (TestBackend harness).
- Description: the modularization phase splits oversized Rust files
  below the 500-LoC budget. It consumes the loc baseline to know which
  files to split, the file-size CI gate to enforce non-regression, the
  insta snapshot corpus (TASK-TUI-006) so that render output stays
  byte-identical across refactors, and the TestBackend harness to
  drive those snapshots.
- Evidence injection: include
  `$(sha256sum project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/baselines/loc-baseline.json)`
  in every Phase-3 gate-2 evidence bundle, and attach the file-size
  gate exit=0 transcript to gate-5.

## Phase-4 — Subagent

- Consumes: TASK-TUI-008 (mock_agent LLM provider), TASK-TUI-010
  (fake_registry fixture corpus).
- Description: the subagent phase wires the multi-agent dispatch
  layer; it uses the mock_agent harness to simulate provider responses
  deterministically and the TASK-TUI-010 fake_registry fixture corpus
  to stand in for the real tool/plugin registry.
- Evidence injection: cite the mock_agent module path and the
  fake_registry fixture path in gate-3 review evidence; include the
  fixture corpus sha in gate-5 live-smoke evidence.

## Phase-5 — Providers

- Consumes: TASK-TUI-001 only (phase-0 dev-flow reference).
- Description: the providers phase introduces new LLM provider
  adapters; it requires no Phase-0 baseline other than the dev-flow
  reference that governs the 6-gate protocol.
- Evidence injection: reference the TASK-TUI-001 dev-flow-reference.md
  path in the Phase-5 plan section of each TASK.

## Phase-6 — Slash commands

- Consumes: TASK-TUI-001 only.
- Description: the slash-commands phase lands the command parser and
  dispatcher; it is pure new code and only depends on the Phase-0
  dev-flow reference.
- Evidence injection: reference TASK-TUI-001 dev-flow-reference in
  gate-1 evidence.

## Phase-7 — Session

- Consumes: TASK-TUI-001 only.
- Description: the session phase lands persistent session
  save/restore; it is additive and only cites the Phase-0 dev-flow
  reference.
- Evidence injection: reference TASK-TUI-001 dev-flow-reference in
  gate-1 evidence.

## Phase-8 — Observability

- Consumes: TASK-TUI-001 only (with forward reference to the
  phase0-capture-manifest for drift detection).
- Description: the observability phase wires tracing/metrics
  subscribers and is where the phase0-capture-manifest sha pins are
  wired into CI drift detection. Only TASK-TUI-001 is formally
  consumed; manifest hashes are a forward-looking contract.
- Evidence injection: reference TASK-TUI-001 dev-flow-reference, and
  include the phase0-capture-manifest.json sha256 in any CI drift
  alert wiring.

## Phase-9 — Preserve / cleanup

- Consumes: TASK-TUI-001 only.
- Description: the preserve/cleanup phase deletes dead code and
  finalises docs; it only cites the Phase-0 dev-flow reference.
- Evidence injection: reference TASK-TUI-001 dev-flow-reference in
  gate-1 evidence.

## Non-modification attestation

Phase 0 did not modify any production source file.

The following git-diff transcript was captured at the tail of the
`scripts/tui-phase0-capture.sh` run. It is intentionally empty,
proving that no file under `src/main.rs`, `crates/archon-tui/src/`, or
`crates/archon-tools/` was touched by any Phase-0 task
(TASK-TUI-001..011):

```
$ git diff --name-only src/main.rs crates/archon-tui/src/ crates/archon-tools/
(empty)
```

Downstream phases (1..9) MUST re-run this assertion at the start of
their own Phase-N gate-1 evidence collection to detect silent
Phase-0-into-production drift.
